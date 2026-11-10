// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use std::collections::HashMap;

pub(super) fn decode_request_body(method: Method, params: &Value) -> ValidationResult<RequestBody> {
    match method {
        Method::SessionControlsRead => Ok(RequestBody::SessionControlsRead(typed(params, true)?)),
        Method::SessionControlsApply => Ok(RequestBody::SessionControlsApply(typed(params, true)?)),
        Method::SessionQueueList => Ok(RequestBody::SessionQueueList(typed(params, true)?)),
        Method::SessionQueueUpdate => Ok(RequestBody::SessionQueueUpdate(typed(params, true)?)),
        Method::SessionQueueDelete => Ok(RequestBody::SessionQueueDelete(typed(params, true)?)),
        Method::SessionQueueReorder => Ok(RequestBody::SessionQueueReorder(typed(params, true)?)),
        Method::SessionQueueStart => Ok(RequestBody::SessionQueueStart(typed(params, true)?)),
        Method::ArtifactsList => Ok(RequestBody::ArtifactsList(typed(params, true)?)),
        Method::ArtifactsOpen => Ok(RequestBody::ArtifactsOpen(typed(params, true)?)),
        Method::ArtifactsRead => Ok(RequestBody::ArtifactsRead(typed(params, true)?)),
        Method::ArtifactsClose => Ok(RequestBody::ArtifactsClose(typed(params, true)?)),
        Method::TerminalCreate => Ok(RequestBody::TerminalCreate(typed(params, true)?)),
        Method::AttachmentsBegin => Ok(RequestBody::AttachmentsBegin(typed(params, true)?)),
        Method::AttachmentsChunk => Ok(RequestBody::AttachmentsChunk(typed(params, true)?)),
        Method::AttachmentsComplete => Ok(RequestBody::AttachmentsComplete(typed(params, true)?)),
        Method::AttachmentsDiscard => Ok(RequestBody::AttachmentsDiscard(typed(params, true)?)),
        Method::AttachmentsStatus => Ok(RequestBody::AttachmentsStatus(typed(params, true)?)),
        Method::PairConfirm => Ok(RequestBody::PairConfirm(typed(params, true)?)),
        Method::Hello => Ok(RequestBody::Hello(typed(params, true)?)),
        Method::HostSnapshot => Ok(RequestBody::HostSnapshot(typed(params, true)?)),
        Method::SessionOpen => Ok(RequestBody::SessionOpen(typed(params, true)?)),
        Method::SessionHistory => Ok(RequestBody::SessionHistory(typed(params, true)?)),
        Method::SessionUnsubscribe => Ok(RequestBody::SessionUnsubscribe(typed(params, true)?)),
        Method::CreationDirectories => Ok(RequestBody::CreationDirectories(typed(params, true)?)),
        Method::SessionCreate => Ok(RequestBody::SessionCreate(typed(params, true)?)),
        Method::AgentRename => Ok(RequestBody::AgentRename(typed(params, true)?)),
        Method::WorkspaceRename => Ok(RequestBody::WorkspaceRename(typed(params, true)?)),
        Method::PromptSubmit => Ok(RequestBody::PromptSubmit(typed(params, true)?)),
        Method::TurnCancel => Ok(RequestBody::TurnCancel(typed(params, true)?)),
        Method::RequestRespond => Ok(RequestBody::RequestRespond(typed(params, true)?)),
        Method::ActionStatus => Ok(RequestBody::ActionStatus(typed(params, true)?)),
        Method::ContentRead => Ok(RequestBody::ContentRead(typed(params, true)?)),
        Method::TerminalGridSubscribe => {
            Ok(RequestBody::TerminalGridSubscribe(typed(params, true)?))
        }
        Method::ShellGridSubscribe => Ok(RequestBody::ShellGridSubscribe(typed(params, true)?)),
        Method::ShellInput => Ok(RequestBody::ShellInput(typed(params, true)?)),
        Method::ShellResize => Ok(RequestBody::ShellResize(typed(params, true)?)),
        Method::ShellScroll => Ok(RequestBody::ShellScroll(typed(params, true)?)),
        Method::SkillsList => Ok(RequestBody::SkillsList(typed(params, true)?)),
        Method::SkillsPreviewOpen => Ok(RequestBody::SkillsPreviewOpen(typed(params, true)?)),
        Method::SkillsPreviewRead => Ok(RequestBody::SkillsPreviewRead(typed(params, true)?)),
        Method::SkillsPreviewClose => Ok(RequestBody::SkillsPreviewClose(typed(params, true)?)),

        Method::WorktreesList => Ok(RequestBody::WorktreesList(typed(params, true)?)),
        Method::ChangesRead => Ok(RequestBody::ChangesRead(typed(params, true)?)),
        Method::NotificationsSync => Ok(RequestBody::NotificationsSync(typed(params, true)?)),
        Method::NotificationsRead => Ok(RequestBody::NotificationsRead(typed(params, true)?)),
        Method::PushRegister => Ok(RequestBody::PushRegister(typed(params, true)?)),
        Method::PushProbeAck => Ok(RequestBody::PushProbeAck(typed(params, true)?)),
        Method::PushUnregister => Ok(RequestBody::PushUnregister(typed(params, true)?)),
        Method::DeviceUnpair => Ok(RequestBody::DeviceUnpair(typed(params, true)?)),
    }
}
pub(super) fn decode_result_body(method: Method, result: &Value) -> ValidationResult<ResultBody> {
    match method {
        Method::SessionControlsRead => Ok(ResultBody::SessionControlsRead(typed(result, false)?)),
        Method::SessionControlsApply => Ok(ResultBody::SessionControlsApply(typed(result, false)?)),
        Method::SessionQueueList => Ok(ResultBody::SessionQueueList(typed(result, false)?)),
        Method::SessionQueueUpdate => Ok(ResultBody::SessionQueueUpdate(typed(result, false)?)),
        Method::SessionQueueDelete => Ok(ResultBody::SessionQueueDelete(typed(result, false)?)),
        Method::SessionQueueReorder => Ok(ResultBody::SessionQueueReorder(typed(result, false)?)),
        Method::SessionQueueStart => Ok(ResultBody::SessionQueueStart(typed(result, false)?)),
        Method::ArtifactsList => Ok(ResultBody::ArtifactsList(typed(result, false)?)),
        Method::ArtifactsOpen => Ok(ResultBody::ArtifactsOpen(typed(result, false)?)),
        Method::ArtifactsRead => Ok(ResultBody::ArtifactsRead(typed(result, false)?)),
        Method::ArtifactsClose => Ok(ResultBody::ArtifactsClose(typed(result, false)?)),
        Method::TerminalCreate => Ok(ResultBody::TerminalCreate(typed(result, false)?)),
        Method::AttachmentsBegin => Ok(ResultBody::AttachmentsBegin(typed(result, false)?)),
        Method::AttachmentsChunk => Ok(ResultBody::AttachmentsChunk(typed(result, false)?)),
        Method::AttachmentsComplete => Ok(ResultBody::AttachmentsComplete(typed(result, false)?)),
        Method::AttachmentsDiscard => Ok(ResultBody::AttachmentsDiscard(typed(result, false)?)),
        Method::AttachmentsStatus => Ok(ResultBody::AttachmentsStatus(typed(result, false)?)),
        Method::PairConfirm => Ok(ResultBody::PairConfirm(typed(result, false)?)),
        Method::Hello => Ok(ResultBody::Hello(typed(result, false)?)),
        Method::HostSnapshot => Ok(ResultBody::HostSnapshot(typed(result, false)?)),
        Method::SessionOpen => Ok(ResultBody::SessionOpen(typed(result, false)?)),
        Method::SessionHistory => Ok(ResultBody::SessionHistory(typed(result, false)?)),
        Method::SessionUnsubscribe => Ok(ResultBody::SessionUnsubscribe(typed(result, false)?)),
        Method::CreationDirectories => Ok(ResultBody::CreationDirectories(typed(result, false)?)),
        Method::SessionCreate => Ok(ResultBody::SessionCreate(typed(result, false)?)),
        Method::AgentRename => Ok(ResultBody::AgentRename(typed(result, false)?)),
        Method::WorkspaceRename => Ok(ResultBody::WorkspaceRename(typed(result, false)?)),
        Method::PromptSubmit => Ok(ResultBody::PromptSubmit(typed(result, false)?)),
        Method::TurnCancel => Ok(ResultBody::TurnCancel(typed(result, false)?)),
        Method::RequestRespond => Ok(ResultBody::RequestRespond(typed(result, false)?)),
        Method::ActionStatus => {
            let o = result.as_object().ok_or_else(invalid)?;
            ensure(o.contains_key("receipt") != o.contains_key("state"))?;
            let status = if o.contains_key("receipt") {
                ActionStatusResult::Known(Box::new(typed(result, false)?))
            } else {
                ActionStatusResult::Unknown(typed(result, false)?)
            };
            Ok(ResultBody::ActionStatus(status))
        }
        Method::ContentRead => Ok(ResultBody::ContentRead(typed(result, false)?)),
        Method::TerminalGridSubscribe => {
            Ok(ResultBody::TerminalGridSubscribe(typed(result, false)?))
        }
        Method::ShellGridSubscribe => Ok(ResultBody::ShellGridSubscribe(typed(result, false)?)),
        Method::ShellInput => Ok(ResultBody::ShellInput(typed(result, false)?)),
        Method::ShellResize => Ok(ResultBody::ShellResize(typed(result, false)?)),
        Method::ShellScroll => Ok(ResultBody::ShellScroll(typed(result, false)?)),
        Method::SkillsList => Ok(ResultBody::SkillsList(typed(result, false)?)),
        Method::SkillsPreviewOpen => Ok(ResultBody::SkillsPreviewOpen(typed(result, false)?)),
        Method::SkillsPreviewRead => Ok(ResultBody::SkillsPreviewRead(typed(result, false)?)),
        Method::SkillsPreviewClose => Ok(ResultBody::SkillsPreviewClose(typed(result, false)?)),

        Method::WorktreesList => Ok(ResultBody::WorktreesList(typed(result, false)?)),
        Method::ChangesRead => {
            validate_changes_read_result(result)?;
            Ok(ResultBody::ChangesRead(typed(result, false)?))
        }
        Method::NotificationsSync => Ok(ResultBody::NotificationsSync(typed(result, false)?)),
        Method::NotificationsRead => Ok(ResultBody::NotificationsRead(typed(result, false)?)),
        Method::PushRegister => Ok(ResultBody::PushRegister(typed(result, false)?)),
        Method::PushProbeAck => Ok(ResultBody::PushProbeAck(typed(result, false)?)),
        Method::PushUnregister => Ok(ResultBody::PushUnregister(typed(result, false)?)),
        Method::DeviceUnpair => Ok(ResultBody::DeviceUnpair(typed(result, false)?)),
    }
}

/// Changes entries are a closed tagged union even on a response frame. Other response models
/// retain their forward-compatible decoding rules.
fn validate_changes_read_result(result: &Value) -> Validation {
    let outer = result.as_object().ok_or_else(invalid)?;
    let changes = outer
        .get("changes")
        .and_then(Value::as_object)
        .ok_or_else(invalid)?;
    let entries = changes
        .get("entries")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    for entry in entries {
        let entry = entry.as_object().ok_or_else(invalid)?;
        let kind = entry
            .get("kind")
            .and_then(Value::as_str)
            .ok_or_else(invalid)?;
        match kind {
            "tracked" => {
                exact_keys(
                    entry,
                    &[
                        "kind",
                        "path",
                        "index",
                        "worktree",
                        "stagedBinary",
                        "unstagedBinary",
                    ],
                    &[
                        "kind",
                        "path",
                        "originalPath",
                        "index",
                        "worktree",
                        "submodule",
                        "stagedBinary",
                        "unstagedBinary",
                    ],
                )?;
                let renamed = ["index", "worktree"].into_iter().any(|key| {
                    matches!(
                        entry.get(key).and_then(Value::as_str),
                        Some("renamed" | "copied")
                    )
                });
                ensure(renamed == entry.contains_key("originalPath"))?;
            }
            "untracked" => exact_keys(entry, &["kind", "path"], &["kind", "path"])?,
            _ => return Err(invalid()),
        }
    }
    Ok(())
}

fn exact_keys(
    object: &serde_json::Map<String, Value>,
    required: &[&str],
    allowed: &[&str],
) -> Validation {
    ensure(
        required.iter().all(|key| object.contains_key(*key))
            && object.keys().all(|key| allowed.contains(&key.as_str())),
    )
}

fn unique<T: Eq + std::hash::Hash>(ids: impl Iterator<Item = T>) -> Validation {
    let mut seen = HashSet::new();
    for id in ids {
        ensure(seen.insert(id))?;
    }
    Ok(())
}
fn count_options(counts: impl Iterator<Item = usize>) -> Validation {
    let mut total = 0;
    for count in counts {
        ensure(count <= 256)?;
        total += count;
    }
    ensure(total <= 1024)
}
pub(super) fn validate_content_ref(x: &ContentRef) -> Validation {
    ensure(x.byte_length.0 <= CONTENT_MAX)
}
pub(super) fn validate_pending_request(x: &PendingRequest) -> Validation {
    let kind = match &x.presentation {
        Presentation::OmpSelect(_) => PresentationKind::OmpSelect,
        Presentation::OmpEditor(_) => PresentationKind::OmpEditor,
        Presentation::CodexApproval(_) => PresentationKind::CodexApproval,
        Presentation::CodexQuestions(_) => PresentationKind::CodexQuestions,
        Presentation::CodexForm(_) => PresentationKind::CodexForm,
        Presentation::CodexUrl(_) => PresentationKind::CodexUrl,
        Presentation::DesktopOnly(_) => PresentationKind::DesktopOnly,
    };
    ensure(
        kind == x.kind
            && (matches!(kind, PresentationKind::DesktopOnly)
                || (x.provider == Provider::Omp)
                    == matches!(
                        kind,
                        PresentationKind::OmpSelect | PresentationKind::OmpEditor
                    )),
    )
}
pub(super) fn validate_question(x: &Question) -> Validation {
    count_options(std::iter::once(x.options.len()))
}
pub(super) fn validate_presentation_omp_select(x: &PresentationOmpSelect) -> Validation {
    count_options(std::iter::once(x.options.len()))?;
    ensure(
        x.selected
            .as_ref()
            .is_none_or(|n| n.0 < x.options.len() as u64)
            && x.markable_count
                .as_ref()
                .is_none_or(|n| n.0 <= x.options.len() as u64)
            && x.checked.iter().all(|n| n.0 < x.options.len() as u64),
    )
}
pub(super) fn validate_presentation_codex_approval(x: &PresentationCodexApproval) -> Validation {
    ensure(!x.choices.is_empty())?;
    count_options(std::iter::once(x.choices.len()))?;
    unique(x.choices.iter().map(|o| o.choice_id.0.as_str()))
}
pub(super) fn validate_presentation_codex_questions(x: &PresentationCodexQuestions) -> Validation {
    ensure(x.questions.len() <= 64)?;
    unique(x.questions.iter().map(|q| q.id.0.as_str()))?;
    count_options(x.questions.iter().map(|q| q.options.len()))
}
pub(super) fn validate_presentation_codex_form(x: &PresentationCodexForm) -> Validation {
    ensure(x.fields.len() <= 64)?;
    unique(x.fields.iter().map(|f| f.id.0.as_str()))?;
    count_options(
        x.fields
            .iter()
            .map(|f| f.options.as_ref().map_or(0, Vec::len)),
    )
}
pub(super) fn validate_presentation_codex_url(x: &PresentationCodexUrl) -> Validation {
    ensure(valid_http_url(&x.url))
}
pub(super) fn validate_answer_omp_select(x: &AnswerOmpSelect) -> Validation {
    ensure((x.action == SubmitAction::Submit) == x.value.is_some())?;
    answer_size("omp_select", x)
}
pub(super) fn validate_answer_omp_editor(x: &AnswerOmpEditor) -> Validation {
    ensure((x.action == SubmitAction::Submit) == x.text.is_some())
}
pub(super) fn validate_answer_codex_questions(x: &AnswerCodexQuestions) -> Validation {
    ensure(x.answers.len() <= 64)?;
    unique(x.answers.iter().map(|a| a.id.0.as_str()))?;
    answer_size("codex_questions", x)
}
pub(super) fn validate_answer_codex_form(x: &AnswerCodexForm) -> Validation {
    ensure((x.action == FormAction::Accept) == x.values.is_some())?;
    if let Some(values) = &x.values {
        ensure(values.len() <= 64)?;
        unique(values.iter().map(|v| v.id.0.as_str()))?;
    }
    answer_size("codex_form", x)
}
pub(super) fn validate_session_history_params(x: &SessionHistoryParams) -> Validation {
    ensure(x.cursor.is_some() == x.view_id.is_some())?;
    ensure(x.limit.as_ref().is_none_or(|n| (1..=100).contains(&n.0)))
}
pub(super) fn validate_content_read_params(x: &ContentReadParams) -> Validation {
    ensure((1..=65_536).contains(&x.limit_bytes.0) && x.offset_bytes.0 <= CONTENT_MAX)
}
pub(super) fn validate_notifications_sync_params(x: &NotificationsSyncParams) -> Validation {
    ensure(x.limit.as_ref().is_none_or(|n| (1..=100).contains(&n.0)))
}
pub(super) fn validate_hello_result(x: &HelloResult) -> Validation {
    ensure(x.backends.len() <= 2)?;
    let mut seen = HashSet::new();
    ensure(x.backends.iter().all(|b| seen.insert(b.provider)))
}
pub(super) fn validate_native_hierarchy(x: &NativeHierarchy) -> Validation {
    ensure(x.workspaces.len() + x.tabs.len() + x.panes.len() <= 512)?;
    ensure(match x.freshness {
        NativeFreshness::Current => x.reason.is_none(),
        NativeFreshness::Stale => x.reason == Some(NativeUnavailableReason::SourceUnavailable),
        NativeFreshness::Unavailable => {
            x.reason.is_some() && x.workspaces.is_empty() && x.tabs.is_empty() && x.panes.is_empty()
        }
    })?;
    unique(x.workspaces.iter().map(|w| w.workspace_id.0.as_str()))?;
    unique(x.tabs.iter().map(|t| t.tab_id.0.as_str()))?;
    unique(x.panes.iter().map(|p| p.pane_id.0.as_str()))?;
    unique(x.panes.iter().map(|p| p.terminal_id.0.as_str()))?;
    let workspaces: HashSet<_> = x.workspaces.iter().map(|w| &w.workspace_id).collect();
    let tabs: HashMap<_, _> = x
        .tabs
        .iter()
        .map(|t| (&t.tab_id, &t.workspace_id))
        .collect();
    ensure(x.tabs.iter().all(|t| workspaces.contains(&t.workspace_id)))?;
    ensure(x.workspaces.iter().all(|w| {
        w.active_tab_id
            .as_ref()
            .is_none_or(|id| tabs.get(id) == Some(&&w.workspace_id))
    }))?;
    ensure(
        x.panes
            .iter()
            .all(|p| tabs.get(&p.tab_id) == Some(&&p.workspace_id)),
    )?;
    ensure(
        x.freshness == NativeFreshness::Current
            || x.panes.iter().all(|p| {
                p.target.is_none() && p.unbound_row_id.is_none() && p.shell_target.is_none()
            }),
    )
}
pub(super) fn validate_host_snapshot_result(x: &HostSnapshotResult) -> Validation {
    ensure(
        x.servers.len() <= 64
            && x.projects.len() <= 128
            && x.sessions.len() + x.unbound.len() + x.shells.len() <= 512
            && x.hierarchy.len() <= 64
            && x.hierarchy
                .iter()
                .map(|h| h.workspaces.len() + h.tabs.len() + h.panes.len())
                .sum::<usize>()
                <= 512,
    )?;
    unique(x.servers.iter().map(|s| s.server_id.0.as_str()))?;
    unique(x.sessions.iter().map(|s| s.session_id.0.as_str()))?;
    unique(x.shells.iter().map(|s| s.target.shell_id.0.as_str()))?;
    unique(x.unbound.iter().map(|r| r.row_id.0.as_str()))?;
    unique(x.hierarchy.iter().map(|h| h.server_id.0.as_str()))?;
    ensure(x.shells.iter().all(|shell| {
        x.servers
            .iter()
            .any(|server| server.server_id == shell.server_id)
            && (shell.connectivity != SessionConnectivity::Ready
                || x.hierarchy.iter().any(|graph| {
                    graph.server_id == shell.server_id
                        && graph.freshness == NativeFreshness::Current
                        && graph.panes.iter().any(|pane| {
                            pane.shell_target.as_ref() == Some(&shell.target)
                                && pane.workspace_id == shell.workspace_id
                                && pane.tab_id == shell.tab_id
                                && pane.pane_id == shell.pane_id
                                && pane.terminal_id == shell.terminal_id
                        })
                }))
    }))?;
    if x.hierarchy.is_empty() {
        return Ok(());
    }
    let servers: HashSet<_> = x.servers.iter().map(|s| &s.server_id).collect();
    let sessions: HashMap<_, _> = x.sessions.iter().map(|s| (&s.session_id, s)).collect();
    let shells: HashMap<_, _> = x.shells.iter().map(|s| (&s.target.shell_id, s)).collect();
    let mut shell_targets = HashSet::new();
    let unbound: HashMap<_, _> = x.unbound.iter().map(|r| (&r.row_id, r)).collect();
    let mut targets = HashSet::new();
    let mut rows = HashSet::new();
    for h in &x.hierarchy {
        ensure(servers.contains(&h.server_id))?;
        for p in &h.panes {
            let provider = p.agent.as_ref().and_then(|a| match a.0.as_str() {
                "omp" => Some(Provider::Omp),
                "codex" => Some(Provider::Codex),
                _ => None,
            });
            if let Some(t) = &p.target {
                let s = sessions.get(&t.session_id).ok_or_else(invalid)?;
                ensure(
                    targets.insert(&t.session_id)
                        && s.generation == t.generation
                        && s.server_id == h.server_id
                        && s.workspace_id == p.workspace_id
                        && s.terminal_id == p.terminal_id
                        && s.pane_id == p.pane_id
                        && provider == Some(s.provider)
                        && matches!(
                            s.connectivity,
                            SessionConnectivity::Ready | SessionConnectivity::Synchronizing
                        ),
                )?;
            }
            if let Some(t) = &p.shell_target {
                let shell = shells.get(&t.shell_id).ok_or_else(invalid)?;
                ensure(
                    shell_targets.insert(&t.shell_id)
                        && shell.target == *t
                        && shell.server_id == h.server_id
                        && shell.workspace_id == p.workspace_id
                        && shell.tab_id == p.tab_id
                        && shell.pane_id == p.pane_id
                        && shell.terminal_id == p.terminal_id
                        && shell.connectivity == SessionConnectivity::Ready
                        && p.agent.is_none(),
                )?;
            }
            if let Some(id) = &p.unbound_row_id {
                let r = unbound.get(id).ok_or_else(invalid)?;
                ensure(
                    rows.insert(id)
                        && r.server_id == h.server_id
                        && r.workspace_id == p.workspace_id
                        && r.terminal_id == p.terminal_id
                        && r.pane_id == p.pane_id
                        && provider == Some(r.provider),
                )?;
            }
        }
    }
    Ok(())
}
pub(super) fn validate_session_open_result(x: &SessionOpenResult) -> Validation {
    ensure(x.pending_requests.len() <= 64 && x.history.len() <= 100)
}
pub(super) fn validate_session_history_result(x: &SessionHistoryResult) -> Validation {
    ensure(x.records.len() <= 100)
}
pub(super) fn validate_notifications_sync_result(x: &NotificationsSyncResult) -> Validation {
    ensure(
        x.events.len() <= 100
            && x.read.len() <= 512
            && x.retained_from.0 <= x.latest_sequence.0.saturating_add(1),
    )?;
    ensure(
        x.events.windows(2).all(|w| w[0].sequence < w[1].sequence)
            && x.events.iter().all(|e| e.sequence.0 <= x.latest_sequence.0),
    )?;
    unique(x.read.iter().map(|r| r.session_id.0.as_str()))?;
    if let Some(n) = &x.next_sequence {
        ensure(x.events.last().is_some_and(|e| e.sequence == *n))?;
    }
    Ok(())
}
pub(super) fn validate_history_upsert_event(x: &HistoryUpsertEvent) -> Validation {
    ensure(!x.records.is_empty() && x.records.len() <= 100)
}
pub(super) fn validate_receipt(x: &Receipt) -> Validation {
    ensure(x.created_at <= x.updated_at)?;
    ensure(
        matches!(
            x.method,
            ActionMethod::ShellCreate
                | ActionMethod::ShellInput
                | ActionMethod::ShellResize
                | ActionMethod::ShellScroll
        ) == x.shell_target.is_some(),
    )?;
    if matches!(
        x.method,
        ActionMethod::ShellCreate
            | ActionMethod::ShellInput
            | ActionMethod::ShellResize
            | ActionMethod::ShellScroll
    ) {
        ensure(x.target.is_none() && x.creation.is_none())?;
        if x.state == ReceiptState::Succeeded {
            ensure(match &x.result {
                Some(ActionResult::ShellCreation(result))
                    if x.method == ActionMethod::ShellCreate =>
                {
                    result.resources.complete()
                }
                Some(ActionResult::ShellInput(result)) if x.method == ActionMethod::ShellInput => {
                    result.progress.accepted()
                }
                Some(ActionResult::ShellResize(_)) if x.method == ActionMethod::ShellResize => true,
                Some(ActionResult::ShellScroll(_)) if x.method == ActionMethod::ShellScroll => true,
                _ => false,
            })?;
        }
    }
    if matches!(
        x.method,
        ActionMethod::AttachmentsBegin
            | ActionMethod::AttachmentsChunk
            | ActionMethod::AttachmentsComplete
            | ActionMethod::AttachmentsDiscard
    ) {
        ensure(x.target.is_some() && x.creation.is_none())?;
        if let Some(ActionResult::Attachment(result)) = &x.result {
            ensure(x.target.as_ref() == Some(&result.target))?;
        }
        if x.state == ReceiptState::Succeeded {
            ensure(matches!(&x.result, Some(ActionResult::Attachment(_))))?;
        }
    }
    // Old provider-target terminal.create receipts remain readable; new configured creation has no provider target.
    if x.method == ActionMethod::TerminalCreate {
        ensure(x.shell_target.is_none() && x.creation.is_none())?;
        if x.state == ReceiptState::Succeeded {
            ensure(
                matches!(&x.result,Some(ActionResult::ShellCreation(result)) if result.resources.complete()),
            )?;
        }
    }
    if x.method == ActionMethod::WorktreesRemove {
        ensure(x.target.is_some() && x.creation.is_none())?;
        if let Some(ActionResult::WorktreeRemoval(result)) = &x.result {
            ensure(Some(&result.target) == x.target.as_ref())?;
        }
        if x.state == ReceiptState::Succeeded {
            ensure(
                matches!(&x.result,Some(ActionResult::WorktreeRemoval(result)) if result.progress.complete()),
            )?;
        }
    }
    if x.method == ActionMethod::AgentRename {
        ensure(x.target.is_some() && x.creation.is_none())?;
        if let Some(ActionResult::AgentRenamed(result)) = &x.result {
            ensure(Some(&result.target) == x.target.as_ref())?;
        }
        if x.state == ReceiptState::Succeeded {
            ensure(
                matches!(&x.result, Some(ActionResult::AgentRenamed(result)) if Some(&result.target) == x.target.as_ref()),
            )?;
        }
    }
    if x.method == ActionMethod::SessionControlsApply {
        ensure(x.target.is_some() && x.creation.is_none() && x.result.is_none())?;
    }
    if matches!(
        x.method,
        ActionMethod::SessionQueueUpdate
            | ActionMethod::SessionQueueDelete
            | ActionMethod::SessionQueueReorder
            | ActionMethod::SessionQueueStart
    ) {
        ensure(x.target.is_some() && x.creation.is_none())?;
        if x.state == ReceiptState::Succeeded {
            ensure(matches!(&x.result, Some(ActionResult::Queue(_))))?;
        }
    }
    if let Some(result) = &x.result {
        ensure(matches!(
            (x.method, result),
            (ActionMethod::TerminalCreate, ActionResult::ShellCreation(_))
                | (ActionMethod::SessionCreate, ActionResult::Created(_))
                | (ActionMethod::ShellCreate, ActionResult::ShellCreation(_))
                | (ActionMethod::ShellInput, ActionResult::ShellInput(_))
                | (ActionMethod::ShellInput, ActionResult::ShellDispatch(_))
                | (ActionMethod::ShellResize, ActionResult::ShellResize(_))
                | (ActionMethod::ShellScroll, ActionResult::ShellScroll(_))
                | (
                    ActionMethod::WorktreesRemove,
                    ActionResult::WorktreeRemoval(_)
                )
                | (ActionMethod::AttachmentsBegin, ActionResult::Attachment(_))
                | (ActionMethod::AttachmentsChunk, ActionResult::Attachment(_))
                | (
                    ActionMethod::AttachmentsComplete,
                    ActionResult::Attachment(_)
                )
                | (
                    ActionMethod::AttachmentsDiscard,
                    ActionResult::Attachment(_)
                )
                | (ActionMethod::AgentRename, ActionResult::AgentRenamed(_))
                | (
                    ActionMethod::WorkspaceRename,
                    ActionResult::WorkspaceRenamed(_)
                )
                | (ActionMethod::PromptSubmit, ActionResult::Prompt(_))
                | (ActionMethod::PromptSubmit, ActionResult::Queue(_))
                | (ActionMethod::SessionQueueUpdate, ActionResult::Queue(_))
                | (ActionMethod::SessionQueueDelete, ActionResult::Queue(_))
                | (ActionMethod::SessionQueueReorder, ActionResult::Queue(_))
                | (ActionMethod::SessionQueueStart, ActionResult::Queue(_))
                | (ActionMethod::TurnCancel, ActionResult::Cancel(_))
                | (ActionMethod::RequestRespond, ActionResult::Response(_))
        ))?;
        if matches!(result, ActionResult::ShellScroll(_)) {
            ensure(x.state == ReceiptState::Succeeded)?;
        }
        if let ActionResult::ShellDispatch(result) = result {
            ensure(x.state != ReceiptState::Succeeded)?;
            if x.state == ReceiptState::Dispatched {
                ensure(result.progress.complete())?;
            }
        }
        if let ActionResult::Queue(result) = result {
            ensure(x.target.is_some() && x.shell_target.is_none() && x.creation.is_none())?;
            match x.method {
                ActionMethod::PromptSubmit | ActionMethod::SessionQueueUpdate => {
                    ensure(
                        result.entry_id.is_some()
                            && result.deleted.is_none()
                            && result.native_turn_id.is_none(),
                    )?;
                }
                ActionMethod::SessionQueueDelete => {
                    ensure(result.entry_id.is_some() && result.native_turn_id.is_none())?;
                    if x.state == ReceiptState::Succeeded {
                        ensure(result.deleted.is_some())?;
                    }
                }
                ActionMethod::SessionQueueStart => {
                    ensure(result.entry_id.is_some() && result.deleted.is_none())?;
                    if x.state == ReceiptState::Succeeded {
                        ensure(result.native_turn_id.is_some())?;
                    }
                }
                ActionMethod::SessionQueueReorder => ensure(
                    result.entry_id.is_none()
                        && result.deleted.is_none()
                        && result.native_turn_id.is_none(),
                )?,
                _ => return Err(invalid()),
            }
        }
    }
    if x.method == ActionMethod::SessionCreate && x.state == ReceiptState::Succeeded {
        ensure(
            matches!(&x.result, Some(ActionResult::Created(_)))
                && x.creation.as_ref().is_some_and(|c| {
                    c.stage == CreationStage::Ready && c.stage_state == StageState::Confirmed
                }),
        )?;
    }
    Ok(())
}
pub(super) fn validate_form_field(f: &FormField) -> Validation {
    let string = f.r#type == FieldType::String;
    let numeric = matches!(f.r#type, FieldType::Number | FieldType::Integer);
    let multi = f.r#type == FieldType::Multi;
    let choice = multi || f.r#type == FieldType::Single;
    ensure(
        (string || (f.format.is_none() && f.min_length.is_none() && f.max_length.is_none()))
            && (numeric || (f.minimum.is_none() && f.maximum.is_none()))
            && (multi || (f.min_items.is_none() && f.max_items.is_none()))
            && (choice == f.options.is_some()),
    )?;
    if let (Some(a), Some(b)) = (&f.min_length, &f.max_length) {
        ensure(a <= b)?;
    }
    if let (Some(a), Some(b)) = (&f.minimum, &f.maximum) {
        ensure(a <= b)?;
    }
    if let (Some(a), Some(b)) = (&f.min_items, &f.max_items) {
        ensure(a <= b)?;
    }
    if let Some(options) = &f.options {
        count_options(std::iter::once(options.len()))?;
    }
    if let Some(default) = &f.default {
        validate_field_value(f, default)?;
    }
    Ok(())
}
pub fn validate_field_value(f: &FormField, value: &FieldValue) -> Validation {
    match (f.r#type, value) {
        (FieldType::String, FieldValue::String(s)) => {
            let n = s.chars().count() as u64;
            ensure(
                f.min_length.as_ref().is_none_or(|x| n >= x.0)
                    && f.max_length.as_ref().is_none_or(|x| n <= x.0),
            )?;
            if let Some(format) = f.format {
                ensure(valid_format(format, s))?;
            }
        }
        (FieldType::Number | FieldType::Integer, FieldValue::String(s)) => {
            let n = Decimal::new(s)?;
            ensure(
                n.0 == *s
                    && (f.r#type != FieldType::Integer || n.is_integer())
                    && f.minimum.as_ref().is_none_or(|x| &n >= x)
                    && f.maximum.as_ref().is_none_or(|x| &n <= x),
            )?;
        }
        (FieldType::Boolean, FieldValue::Boolean(_)) => {}
        (FieldType::Single, FieldValue::String(s)) => ensure(
            f.options
                .as_ref()
                .is_some_and(|xs| xs.iter().any(|x| x.value == *s)),
        )?,
        (FieldType::Multi, FieldValue::Multi(xs)) => {
            ensure(
                f.min_items.as_ref().is_none_or(|x| xs.len() as u64 >= x.0)
                    && f.max_items.as_ref().is_none_or(|x| xs.len() as u64 <= x.0)
                    && f.options.as_ref().is_some_and(|options| {
                        xs.iter().all(|s| options.iter().any(|o| o.value == *s))
                    }),
            )?;
        }
        _ => return Err(invalid()),
    }
    Ok(())
}
/// Offered membership is checked against the exact generation-bound presentation.
pub fn validate_answer(presentation: &Presentation, answer: &Answer) -> Validation {
    presentation.validate()?;
    answer.validate()?;
    match (presentation, answer) {
        (Presentation::OmpSelect(p), Answer::OmpSelect(a)) => {
            if let Some(value) = &a.value {
                ensure(p.options.iter().any(|o| o.value == *value))?;
            }
        }
        (Presentation::OmpEditor(_), Answer::OmpEditor(_)) => {}
        (Presentation::CodexApproval(p), Answer::CodexApproval(a)) => {
            ensure(p.choices.iter().any(|c| c.choice_id == a.choice_id))?
        }
        (Presentation::CodexQuestions(p), Answer::CodexQuestions(a)) => {
            ensure(p.questions.len() == a.answers.len())?;
            for q in &p.questions {
                let answer = a
                    .answers
                    .iter()
                    .find(|a| a.id == q.id)
                    .ok_or_else(invalid)?;
                ensure(
                    q.options.is_empty()
                        || q.is_other
                        || answer
                            .values
                            .iter()
                            .all(|s| q.options.iter().any(|o| o.label.0 == *s)),
                )?;
            }
        }
        (Presentation::CodexForm(p), Answer::CodexForm(a)) => {
            if let Some(values) = &a.values {
                for value in values {
                    let field = p
                        .fields
                        .iter()
                        .find(|f| f.id == value.id)
                        .ok_or_else(invalid)?;
                    validate_field_value(field, &value.value)?;
                }
                ensure(
                    p.fields
                        .iter()
                        .all(|f| !f.required || values.iter().any(|v| v.id == f.id)),
                )?;
            }
        }
        (Presentation::CodexUrl(_), Answer::CodexUrl(_)) => {}
        _ => return Err(invalid()),
    }
    Ok(())
}

fn ipv4(s: &str, allow_leading_zeroes: bool) -> bool {
    let mut count = 0;
    for x in s.split('.') {
        if x.is_empty()
            || x.len() > 3
            || !x.bytes().all(|b| b.is_ascii_digit())
            || !x.parse::<u16>().is_ok_and(|n| n <= 255)
            || (!allow_leading_zeroes && x.len() > 1 && x.starts_with('0'))
        {
            return false;
        }
        count += 1;
    }
    count == 4
}
fn ipv6(s: &str, allow_ipv4_leading_zeroes: bool) -> bool {
    let mut parts = s.split("::");
    let first = parts.next().unwrap_or("");
    let second = parts.next();
    if parts.next().is_some() {
        return false;
    }
    let mut groups = 0;
    for (side, part) in [first, second.unwrap_or("")].iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        let mut xs = part.split(':').peekable();
        while let Some(x) = xs.next() {
            if x.contains('.') {
                if !(side == 1 || second.is_none())
                    || xs.peek().is_some()
                    || !ipv4(x, allow_ipv4_leading_zeroes)
                {
                    return false;
                }
                groups += 2;
            } else {
                if x.is_empty() || x.len() > 4 || !x.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return false;
                }
                groups += 1;
            }
        }
    }
    if second.is_some() {
        groups < 8
    } else {
        groups == 8
    }
}
fn ldh(s: &str) -> bool {
    !s.is_empty()
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && s.as_bytes().last().is_some_and(u8::is_ascii_alphanumeric)
}
fn valid_email(s: &str) -> bool {
    if !s.is_ascii() {
        return false;
    }
    let (local, domain) = if s.starts_with('"') {
        let b = s.as_bytes();
        let mut i = 1;
        while i < b.len() {
            if b[i] == b'"' {
                break;
            }
            if b[i] == b'\\' {
                i += 1;
                if i == b.len() || !(32..=126).contains(&b[i]) {
                    return false;
                }
            } else if !(b[i] == 32
                || b[i] == 33
                || (35..=91).contains(&b[i])
                || (93..=126).contains(&b[i]))
            {
                return false;
            }
            i += 1;
        }
        if i == b.len() || b.get(i + 1) != Some(&b'@') {
            return false;
        }
        (&s[..i + 1], &s[i + 2..])
    } else {
        let Some((local, domain)) = s.split_once('@') else {
            return false;
        };
        (local, domain)
    };
    if local.is_empty() || domain.is_empty() {
        return false;
    }
    if !local.starts_with('"')
        && !local.split('.').all(|a| {
            !a.is_empty()
                && a.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-/=?^_`{|}~".contains(&b))
        })
    {
        return false;
    }
    if domain.starts_with('[') && domain.ends_with(']') {
        let inner = &domain[1..domain.len() - 1];
        if ipv4(inner, true) {
            return true;
        }
        if inner
            .get(..5)
            .is_some_and(|s| s.eq_ignore_ascii_case("IPv6:"))
        {
            return ipv6(&inner[5..], true);
        }
        if let Some((tag, content)) = inner.split_once(':') {
            return ldh(tag)
                && !content.is_empty()
                && content
                    .bytes()
                    .all(|b| (33..=90).contains(&b) || (94..=126).contains(&b));
        }
        return false;
    }
    domain
        .split('.')
        .all(|label| ldh(label) && label.as_bytes()[0].is_ascii_alphanumeric())
}
fn uri_chars(s: &str, extra: &[u8]) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            if i + 2 >= b.len() || !b[i + 1].is_ascii_hexdigit() || !b[i + 2].is_ascii_hexdigit() {
                return false;
            }
            i += 3;
        } else {
            if !(b[i].is_ascii_alphanumeric()
                || b"-._~!$&'()*+,;=".contains(&b[i])
                || extra.contains(&b[i]))
            {
                return false;
            }
            i += 1;
        }
    }
    true
}
fn valid_authority(authority: &str) -> bool {
    let mut host = authority;
    if let Some((user, rest)) = authority.rsplit_once('@') {
        if !uri_chars(user, b":") {
            return false;
        }
        host = rest;
    }
    if host.starts_with('[') {
        let Some(end) = host.find(']') else {
            return false;
        };
        let literal = &host[1..end];
        let suffix = &host[end + 1..];
        if !suffix.is_empty()
            && !(suffix.starts_with(':') && suffix[1..].bytes().all(|b| b.is_ascii_digit()))
        {
            return false;
        }
        if ipv6(literal, false) {
            return true;
        }
        if literal.starts_with(['v', 'V'])
            && let Some((version, address)) = literal[1..].split_once('.')
        {
            return !version.is_empty()
                && version.bytes().all(|b| b.is_ascii_hexdigit())
                && !address.is_empty()
                && !address.contains('%')
                && uri_chars(address, b":");
        }
        false
    } else {
        if let Some((h, port)) = host.rsplit_once(':') {
            if !port.bytes().all(|b| b.is_ascii_digit()) {
                return false;
            }
            host = h;
        }
        uri_chars(host, b"")
    }
}
pub fn valid_uri(s: &str) -> bool {
    if !s.is_ascii() {
        return false;
    }
    let Some((scheme, rest)) = s.split_once(':') else {
        return false;
    };
    if scheme.is_empty()
        || !scheme.as_bytes()[0].is_ascii_alphabetic()
        || !scheme
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"+-.".contains(&b))
    {
        return false;
    }
    let (rest, fragment) = rest
        .split_once('#')
        .map_or((rest, None), |(r, f)| (r, Some(f)));
    if fragment.is_some_and(|f| !uri_chars(f, b":@/?")) {
        return false;
    }
    let (path, query) = rest
        .split_once('?')
        .map_or((rest, None), |(p, q)| (p, Some(q)));
    if query.is_some_and(|q| !uri_chars(q, b":@/?")) {
        return false;
    }
    if let Some(authority_path) = path.strip_prefix("//") {
        let (authority, path) = authority_path.find('/').map_or((authority_path, ""), |i| {
            (&authority_path[..i], &authority_path[i..])
        });
        valid_authority(authority) && uri_chars(path, b":@/")
    } else {
        !path.starts_with("//") && uri_chars(path, b":@/")
    }
}
fn days(year: i32, month: i32) -> i32 {
    match month {
        2 => {
            if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => 0,
    }
}
fn date(s: &str) -> Option<(i32, i32, i32)> {
    if s.len() != 10
        || s.as_bytes()[4] != b'-'
        || s.as_bytes()[7] != b'-'
        || !s
            .bytes()
            .enumerate()
            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit())
    {
        return None;
    }
    let y = s[..4].parse().ok()?;
    let m = s[5..7].parse().ok()?;
    let d = s[8..].parse().ok()?;
    if (1..=days(y, m)).contains(&d) {
        Some((y, m, d))
    } else {
        None
    }
}
fn valid_date_time(s: &str) -> bool {
    if !s.is_ascii() || s.len() < 20 {
        return false;
    }
    let Some((mut y, mut m, mut d)) = date(&s[..10]) else {
        return false;
    };
    let b = s.as_bytes();
    if !matches!(b[10], b'T' | b't')
        || b[13] != b':'
        || b[16] != b':'
        || ![11, 12, 14, 15, 17, 18]
            .iter()
            .all(|i| b[*i].is_ascii_digit())
    {
        return false;
    }
    let h = s[11..13].parse::<i32>().unwrap();
    let minute = s[14..16].parse::<i32>().unwrap();
    let second = s[17..19].parse::<i32>().unwrap();
    if h > 23 || minute > 59 || second > 60 {
        return false;
    }
    let mut at = 19;
    if b[at] == b'.' {
        at += 1;
        let start = at;
        while at < b.len() && b[at].is_ascii_digit() {
            at += 1;
        }
        if at == start {
            return false;
        }
    }
    let offset = if b.get(at).is_some_and(|b| matches!(b, b'Z' | b'z')) && at + 1 == b.len() {
        0
    } else {
        if at + 6 != b.len()
            || !matches!(b[at], b'+' | b'-')
            || b[at + 3] != b':'
            || ![at + 1, at + 2, at + 4, at + 5]
                .iter()
                .all(|i| b[*i].is_ascii_digit())
        {
            return false;
        }
        let oh = s[at + 1..at + 3].parse::<i32>().unwrap();
        let om = s[at + 4..].parse::<i32>().unwrap();
        if oh > 23 || om > 59 {
            return false;
        }
        (oh * 60 + om) * if b[at] == b'+' { 1 } else { -1 }
    };
    if second != 60 {
        return true;
    }
    let utc = h * 60 + minute - offset;
    let day = utc.div_euclid(1440);
    if day < 0 {
        d -= 1;
        if d == 0 {
            m -= 1;
            if m == 0 {
                m = 12;
                y -= 1;
            }
            d = days(y, m);
        }
    } else if day > 0 {
        d += 1;
        if d > days(y, m) {
            d = 1;
            m += 1;
            if m == 13 {
                m = 1;
                y += 1;
            }
        }
    }
    utc.rem_euclid(1440) == 1439 && d == days(y, m)
}
pub fn valid_format(format: FieldFormat, s: &str) -> bool {
    match format {
        FieldFormat::Email => valid_email(s),
        FieldFormat::Uri => valid_uri(s),
        FieldFormat::Date => s.is_ascii() && date(s).is_some(),
        FieldFormat::DateTime => valid_date_time(s),
    }
}

fn valid_http_url(s: &str) -> bool {
    if !valid_uri(s) {
        return false;
    }
    let Some((scheme, rest)) = s.split_once(':') else {
        return false;
    };
    if !(scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")) {
        return false;
    }
    let Some(rest) = rest.strip_prefix("//") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority
        .rsplit('@')
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("");
    !host.is_empty()
}

fn answer_size<T: Serialize>(kind: &str, answer: &T) -> Validation {
    let Value::Object(mut object) = serde_json::to_value(answer).map_err(|_| invalid())? else {
        return Err(invalid());
    };
    object.insert("kind".into(), Value::String(kind.into()));
    ensure(canonical_json_size(&Value::Object(object))? <= ANSWER_MAX)
}

pub fn validate_turn_cancel(provider: Provider, params: &TurnCancelParams) -> Validation {
    params.validate()?;
    ensure((provider == Provider::Codex) == params.native_turn_id.is_some())
}
pub fn validate_content_read(params: &ContentReadParams, byte_length: Uint) -> Validation {
    params.validate()?;
    byte_length.validate()?;
    ensure(byte_length.0 <= CONTENT_MAX && params.offset_bytes <= byte_length)
}
