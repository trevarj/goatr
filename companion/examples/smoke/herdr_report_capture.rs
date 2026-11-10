//! Private receiver-side OMP report capture for one owned diagnostic run.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    os::{
        fd::AsRawFd,
        unix::{
            fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
            net::{UnixListener, UnixStream},
            process::CommandExt,
        },
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

const GATE: &str = "owned-omp-creation-capture";
const MAX_LINES: u8 = 8;
// Preserve five receiver slots: two ingress families, their selected dispositions,
// and one integrity result.
const MAX_COMPANION_LINES: u8 = 3;
const MAX_CONNECTIONS: usize = 2;
const MAX_FRAME: usize = 128;
const PRODUCER_SOCKET: &str = "omp-producer-report.sock";
const PRODUCER_FALLBACK: &str = "omp-producer-fallback.jsonl";
const PRODUCER_HOOK: &str = "goatr-omp-producer-state.ts";
const STOCK_HOOK: &str = "/home/trev/.omp/agent/extensions/herdr-omp-agent-state.ts";
const STOCK_HOOK_SHA256: &str = "e3c650752bb9c2eb8bc1eecc9ce845c27434f6b7331af161353dd06ca6de48b7";
const PRODUCER_MAX_CONNECTIONS: usize = 1;
const PRODUCER_MAX_FRAMES: usize = 8;
const PRODUCER_MAX_TOTAL_BYTES: usize = 1024;
const PRODUCER_FALLBACK_MAX_RECORDS: usize = 4;
const PRODUCER_FALLBACK_MAX_BYTES: usize = 1024;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ProducerSettingsProof {
    v: u8,
    extensions: Vec<String>,
    #[serde(rename = "disabledExtensions")]
    disabled_extensions: Vec<String>,
    #[serde(rename = "extensionsSha256")]
    extensions_sha256: String,
    #[serde(rename = "disabledExtensionsSha256")]
    disabled_extensions_sha256: String,
    #[serde(rename = "stockHookExplicitlyConfigured")]
    stock_hook_explicitly_configured: bool,
    #[serde(rename = "stockHookDisabled")]
    stock_hook_disabled: bool,
    #[serde(rename = "stockHookBroadlyDisabled")]
    stock_hook_broadly_disabled: bool,
}

#[derive(Clone, Copy)]
struct ProducerFallback {
    integrity: &'static str,
    module_entered: bool,
    factory_entered: bool,
    socket: &'static str,
}

impl Default for ProducerFallback {
    fn default() -> Self {
        Self {
            integrity: "missing",
            module_entered: false,
            factory_entered: false,
            socket: "none",
        }
    }
}

struct ProducerFallbackFile {
    path: PathBuf,
    root: PathBuf,
    token: String,
}

struct ProducerState {
    active: bool,
    saw_ingress: bool,
    channel_loss: bool,
    overflow: bool,
    startup: &'static str,
    reference: &'static str,
    delivery: &'static str,
    fallback: ProducerFallback,
    process: Option<goatr::herdr::ProcessEvidence>,
}

impl Default for ProducerState {
    fn default() -> Self {
        Self {
            active: false,
            saw_ingress: false,
            channel_loss: false,
            overflow: false,
            startup: "none",
            reference: "none",
            delivery: "none",
            fallback: ProducerFallback::default(),
            process: None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BudgetMode {
    Creation,
    Offline,
    Backend,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BackendPriority {
    Launch,
    Readiness,
}

#[derive(Clone)]
pub(super) struct Budget(Arc<Mutex<BudgetState>>);

struct BudgetState {
    emitted: u8,
    companion_emitted: u8,
    mode: BudgetMode,
    reserve_offline_roles: bool,
    backend_launch_emitted: bool,
    backend_readiness_emitted: bool,
    coarse_shell_settled: bool,
    backend_seen: HashSet<(String, String)>,
    shell_probe_delivered: bool,
    server_emitted: bool,
    phone_emitted: bool,
    child_emitted: bool,
    creation_terminal_emitted: bool,
    creation_nonterminal_lines: u8,
    state_ingress: bool,
    session_ingress: bool,
    state_disposition: bool,
    session_disposition: bool,
    integrity: Option<&'static str>,
    producer_active: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EventKind {
    IngressState,
    IngressSession,
    DispositionState,
    DispositionSession,
    Overflow,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
enum SnapshotStage {
    #[default]
    None,
    Parsed,
    Enqueued,
    Dispatched,
}

impl SnapshotStage {
    const fn closed(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Parsed => "parsed",
            Self::Enqueued => "enqueued",
            Self::Dispatched => "dispatched",
        }
    }
}

#[derive(Default)]
struct SnapshotState {
    stage: SnapshotStage,
    write: Option<SnapshotWrite>,
    startup_failure: Option<SnapshotFailure>,
    startup_succeeded: bool,
    out_of_order: bool,
}

impl SnapshotState {
    fn observe(&mut self, event: SnapshotEvent) {
        match event {
            SnapshotEvent::Stage(stage) => {
                if self.write.is_some() || stage < self.stage {
                    self.out_of_order = true;
                }
                self.stage = self.stage.max(stage);
            }
            SnapshotEvent::Write(write) => {
                if self.write.is_some() {
                    self.out_of_order = true;
                } else {
                    self.write = Some(write);
                }
            }
        }
    }

    fn startup_failure(&mut self, failure: SnapshotFailure) {
        if !self.startup_succeeded && self.startup_failure.is_none() {
            self.startup_failure = Some(failure);
        }
    }

    fn startup_success(&mut self) {
        self.startup_succeeded = true;
        self.startup_failure = None;
    }
}

#[derive(Clone, Copy)]
enum SnapshotEvent {
    Stage(SnapshotStage),
    Write(SnapshotWrite),
}

#[derive(Clone, Copy)]
struct SnapshotWrite {
    guard: &'static str,
    io: &'static str,
}

#[derive(Clone, Copy)]
struct SnapshotFailure {
    phase: &'static str,
    guard: &'static str,
    io: &'static str,
}

// Keep this in lockstep with live.rs: a preflight record is accepted only when it names an
// authored existing failure boundary, never a generic backend diagnostic.
fn launch_preflight_guard(value: &Value) -> bool {
    matches!(
        (
            value.get("phase").and_then(Value::as_str),
            value.get("guard").and_then(Value::as_str),
        ),
        (
            Some("launch_preflight"),
            Some(
                "project_lookup"
                    | "project_path"
                    | "creation_overlay"
                    | "creation_overlay_path"
                    | "diagnostic_extension_args"
                    | "source_lookup"
                    | "topology_source"
                    | "topology_source_changed"
                    | "topology_project"
                    | "topology_focus"
                    | "measurement_absent"
                    | "wrapper_recheck_failure"
                    | "wrapper_recheck_mismatch"
                    | "pane_absent"
                    | "pane_mismatch"
                    | "resource_mismatch"
                    | "resource_pane_absent"
                    | "resource_terminal_absent"
                    | "process_connect"
                    | "process_evidence"
                    | "process_info"
                    | "shell_reporter"
                    | "shell_absent"
                    | "shell_not_idle"
                    | "shell_process"
                    | "shell_executable"
                    | "shell_reporter_recheck"
                    | "original_root"
                    | "root_recheck_source"
                    | "root_recheck_pane"
                    | "root_recheck_workspace"
                    | "root_recheck_terminal"
                    | "root_recheck_focus"
                    | "root_recheck_pane_focused"
                    | "root_recheck_agent"
                    | "root_recheck_agent_session"
                    | "root_recheck_agent_status"
                    | "root_recheck_agents"
                    | "original_shell_process"
                    | "original_shell_mismatch"
                    | "original_executable_recheck"
                    | "original_executable_mismatch"
                    | "launch_connect"
                    | "launch_evidence"
                    | "launch_socket_fence"
            )
        )
    )
}

// This table is intentionally limited to the existing backend reporter's closed native
// launch and readiness families. Generic output stays silent until both are retained.
fn backend_priority(value: &Value) -> Option<BackendPriority> {
    let phase = value.get("phase")?.as_str()?;
    let guard = value.get("guard")?.as_str()?;
    match (phase, guard) {
        (
            "herdr_launch",
            "herdr_response_timeout"
            | "herdr_response_io"
            | "ack_matched"
            | "ack_mismatch"
            | "rejected_known"
            | "rejected_other"
            | "source_changed"
            | "unavailable",
        ) => Some(BackendPriority::Launch),
        ("launch_preflight", _) if launch_preflight_guard(value) => Some(BackendPriority::Launch),
        (
            "omp_ready",
            "topology_not_ready"
            | "topology_failure"
            | "native_report_absent"
            | "native_report_wrong"
            | "no_connection",
        ) => Some(BackendPriority::Readiness),
        _ => None,
    }
}

// Accept only the closed settlement shape already filtered by the companion stderr parser.
fn coarse_shell_settlement(value: &Value) -> bool {
    if value.get("phase").and_then(Value::as_str) != Some("coarse_shell")
        || value.get("frame").and_then(Value::as_str) != Some("none")
    {
        return false;
    }
    let code = value.get("code").and_then(Value::as_str);
    let io = value.get("io").and_then(Value::as_str);
    match value.get("guard").and_then(Value::as_str) {
        Some("installed") => code == Some("none") && io == Some("none"),
        Some(
            "consent_absent" | "peer_identity" | "shell_absent" | "no_candidate" | "record_absent",
        ) => code == Some("NotReady") && io == Some("none"),
        Some("source_lookup") => {
            matches!(code, Some("NotFound" | "NotReady" | "ResourceLimit")) && io == Some("none")
        }
        Some(
            "reporter_process" | "connect" | "process_info" | "reported_process"
            | "shell_executable" | "source_recheck",
        ) => {
            matches!(code, Some("NotReady" | "ResourceLimit"))
                && matches!(
                    io,
                    Some(
                        "not_found"
                            | "timed_out"
                            | "connection_refused"
                            | "unexpected_eof"
                            | "connection_reset"
                            | "connection_aborted"
                            | "broken_pipe"
                            | "not_connected"
                            | "permission_denied"
                            | "invalid_data"
                            | "other"
                    )
                )
        }
        _ => false,
    }
}

fn backend_budget_admits(state: &BudgetState, priority: Option<BackendPriority>) -> bool {
    if state.backend_launch_emitted && state.backend_readiness_emitted {
        return state.emitted < MAX_LINES;
    }
    match priority {
        Some(BackendPriority::Launch) => !state.backend_launch_emitted && state.emitted < MAX_LINES,
        Some(BackendPriority::Readiness) => {
            !state.backend_readiness_emitted && state.emitted < MAX_LINES
        }
        None => {
            state.backend_launch_emitted
                && state.backend_readiness_emitted
                && state.emitted < MAX_LINES
        }
    }
}

fn backend_seen_key(value: &Value) -> Option<(String, String)> {
    Some((
        value.get("phase")?.as_str()?.to_owned(),
        value.get("guard")?.as_str()?.to_owned(),
    ))
}

fn transport_progress(value: &Value) -> bool {
    matches!(
        (
            value.get("phase").and_then(Value::as_str),
            value.get("guard").and_then(Value::as_str)
        ),
        (
            Some("transport"),
            Some("dispatch_returned" | "response_queued" | "output_written_after_queue")
        )
    )
}

fn creation_transient_native_report(value: &Value) -> bool {
    matches!(
        (
            value.get("phase").and_then(Value::as_str),
            value.get("guard").and_then(Value::as_str)
        ),
        (Some("omp_ready"), Some("native_report_absent"))
            | (Some("follower"), Some("follower_native_report_absent"))
    )
}

fn creation_terminal_readiness(value: &Value) -> bool {
    if launch_preflight_guard(value) {
        return true;
    }
    matches!(
        (
            value.get("phase").and_then(Value::as_str),
            value.get("guard").and_then(Value::as_str)
        ),
        (
            Some("omp_ready"),
            Some(
                "report_absent_after_interactive_ready"
                    | "report_absent_without_interactive_ready"
                    | "original_agent_absent"
            )
        ) | (Some("shell_probe"), Some(_))
    )
}

/// Only the existing Backend result-deadline line may carry these finite observations.
pub(super) fn valid_probe_deadline(value: &Value) -> bool {
    let detailed = ["deadlineSite", "observation", "entryBudget"]
        .iter()
        .any(|key| value.get(key).is_some());
    let deadline = value.get("phase").and_then(Value::as_str) == Some("shell_probe")
        && value.get("guard").and_then(Value::as_str) == Some("result_deadline");
    if !deadline {
        return !detailed;
    }
    let Some(object) = value.as_object() else {
        return false;
    };
    if object.keys().any(|key| {
        !matches!(
            key.as_str(),
            "gate"
                | "phase"
                | "guard"
                | "code"
                | "io"
                | "frame"
                | "ack"
                | "deadlineSite"
                | "observation"
                | "entryBudget"
        )
    }) {
        return false;
    }
    if !detailed {
        return true;
    }
    if value.get("code").and_then(Value::as_str) != Some("Timeout")
        || value.get("io").and_then(Value::as_str) != Some("none")
        || value.get("frame").and_then(Value::as_str) != Some("none")
        || value.get("ack").is_some_and(|ack| !ack.is_null())
    {
        return false;
    }
    let site = value.get("deadlineSite").and_then(Value::as_str);
    let observation = value.get("observation").and_then(Value::as_str);
    let budget = value.get("entryBudget").and_then(Value::as_str);
    if !matches!(
        site,
        Some("loop_check" | "after_authorization" | "complete_before_parse" | "parsed_success")
    ) || !matches!(
        observation,
        Some("not_read" | "empty_only" | "partial_seen" | "complete_seen")
    ) || !matches!(budget, Some("expired" | "le_1s" | "le_5s" | "gt_5s"))
    {
        return false;
    }
    if matches!(site, Some("complete_before_parse" | "parsed_success"))
        && observation != Some("complete_seen")
    {
        return false;
    }
    budget != Some("expired") || (site == Some("loop_check") && observation == Some("not_read"))
}

impl Budget {
    pub(super) fn new(reserve_offline_roles: bool) -> Self {
        let mode = if reserve_offline_roles {
            BudgetMode::Offline
        } else {
            BudgetMode::Creation
        };
        Self::with_mode(mode, reserve_offline_roles)
    }

    /// Backend-only capture has no upstream receiver. Reserve both native first-guard
    /// families so generic server diagnostics cannot hide launch or readiness evidence.
    pub(super) fn new_backend() -> Self {
        Self::with_mode(BudgetMode::Backend, false)
    }

    fn with_mode(mode: BudgetMode, reserve_offline_roles: bool) -> Self {
        Self(Arc::new(Mutex::new(BudgetState {
            emitted: 0,
            companion_emitted: 0,
            mode,
            reserve_offline_roles,
            backend_launch_emitted: false,
            backend_readiness_emitted: false,
            coarse_shell_settled: false,
            backend_seen: HashSet::new(),
            shell_probe_delivered: false,
            server_emitted: false,
            phone_emitted: false,
            child_emitted: false,
            creation_terminal_emitted: false,
            creation_nonterminal_lines: 0,
            state_ingress: false,
            session_ingress: false,
            state_disposition: false,
            session_disposition: false,
            integrity: None,
            producer_active: false,
        })))
    }

    fn companion(&self, role: &'static str, mut value: Value) {
        let Ok(mut state) = self.0.lock() else {
            return;
        };
        let detailed = value.get("deadlineSite").is_some();
        let deadline = value.get("phase").and_then(Value::as_str) == Some("shell_probe")
            && value.get("guard").and_then(Value::as_str) == Some("result_deadline");
        if !valid_probe_deadline(&value)
            || (detailed && state.mode != BudgetMode::Backend)
            || (deadline && state.mode == BudgetMode::Backend && !detailed)
        {
            return;
        }
        if matches!(state.mode, BudgetMode::Creation | BudgetMode::Backend)
            && role == "server"
            && transport_progress(&value)
        {
            return;
        }
        if value.get("phase").and_then(Value::as_str) == Some("coarse_shell") {
            // Only Backend admits coarse startup capture. Creation keeps its original
            // three companion slots and all five reserved receiver slots unchanged.
            if state.mode != BudgetMode::Backend {
                return;
            }
            let reserved = u8::from(!state.backend_launch_emitted)
                + u8::from(!state.backend_readiness_emitted);
            if role != "server"
                || !coarse_shell_settlement(&value)
                || state.coarse_shell_settled
                || state.emitted.saturating_add(reserved) >= MAX_LINES
            {
                return;
            }
            state.coarse_shell_settled = true;
            state.emitted += 1;
            state.companion_emitted += 1;
            let _ = writeln!(std::io::stdout().lock(), "{value}");
            return;
        }
        let creation_terminal = state.mode == BudgetMode::Creation
            && role == "server"
            && creation_terminal_readiness(&value);
        if state.mode == BudgetMode::Creation
            && role == "server"
            && (creation_transient_native_report(&value)
                || (creation_terminal && state.creation_terminal_emitted)
                || (!creation_terminal && state.creation_nonterminal_lines >= 2))
        {
            return;
        }
        if state.mode == BudgetMode::Backend {
            if role != "server" {
                return;
            }
            let priority = backend_priority(&value);
            let Some(key) = backend_seen_key(&value) else {
                return;
            };
            let shell_probe_failure = value.get("phase").and_then(Value::as_str)
                == Some("shell_probe")
                && value
                    .get("code")
                    .and_then(Value::as_str)
                    .is_some_and(|code| code != "none");
            if state.emitted >= MAX_LINES
                || (!shell_probe_failure && !backend_budget_admits(&state, priority))
                || state.backend_seen.contains(&key)
            {
                return;
            }
            match priority {
                Some(BackendPriority::Launch) => state.backend_launch_emitted = true,
                Some(BackendPriority::Readiness) => state.backend_readiness_emitted = true,
                None => {}
            }
            state.backend_seen.insert(key);
            state.emitted += 1;
            state.companion_emitted += 1;
            let delivered = writeln!(std::io::stdout().lock(), "{value}").is_ok();
            state.shell_probe_delivered |= shell_probe_failure && delivered;
            return;
        }
        if !state.reserve_offline_roles {
            if role != "server"
                || state.emitted >= MAX_LINES
                || state.companion_emitted >= MAX_COMPANION_LINES
            {
                return;
            }
            if creation_terminal {
                state.creation_terminal_emitted = true;
            } else if state.mode == BudgetMode::Creation {
                state.creation_nonterminal_lines += 1;
            }
            state.emitted += 1;
            state.companion_emitted += 1;
            let _ = writeln!(std::io::stdout().lock(), "{value}");
            return;
        }
        let already_emitted = match role {
            "server" => state.server_emitted,
            "phone" => state.phone_emitted,
            "companion" => state.child_emitted,
            _ => return,
        };
        if already_emitted
            || state.emitted >= MAX_LINES
            || state.companion_emitted >= MAX_COMPANION_LINES
        {
            return;
        }
        let Some(object) = value.as_object_mut() else {
            return;
        };
        object.insert("role".into(), Value::String(role.into()));
        match role {
            "server" => state.server_emitted = true,
            "phone" => state.phone_emitted = true,
            "companion" => state.child_emitted = true,
            _ => unreachable!(),
        }
        state.emitted += 1;
        state.companion_emitted += 1;
        let _ = writeln!(std::io::stdout().lock(), "{value}");
    }

    /// The first selected server-side cause reserves one companion slot.
    pub(super) fn omp(&self, value: Value) {
        self.companion("server", value);
    }
    /// Acknowledges delivery through this budget, not a producer exit or native success.
    pub(super) fn shell_probe_captured(&self) -> bool {
        self.0.lock().is_ok_and(|state| state.shell_probe_delivered)
    }

    #[cfg(test)]
    pub(super) fn creation_counts(&self) -> (u8, u8, bool) {
        let state = self.0.lock().expect("capture budget mutex");
        (
            state.companion_emitted,
            state.creation_nonterminal_lines,
            state.creation_terminal_emitted,
        )
    }

    /// A first phone frame-read failure retains only the connection-close variant.
    pub(super) fn phone(&self, value: Value) {
        self.companion("phone", value);
    }

    /// Drop observes the owned companion child status before its existing cleanup signal.
    pub(super) fn child(&self, value: Value) {
        self.companion("companion", value);
    }

    fn receiver(&self, guard: &str) {
        let Some(kind) = closed_event(guard) else {
            return;
        };
        if kind == EventKind::Overflow {
            self.incomplete("capture_overflow");
            return;
        }
        let Ok(mut state) = self.0.lock() else {
            return;
        };
        match kind {
            EventKind::IngressState if state.state_ingress => return,
            EventKind::IngressSession if state.session_ingress => return,
            EventKind::DispositionState if state.state_disposition => return,
            EventKind::DispositionSession if state.session_disposition => return,
            _ => {}
        }
        // Three companion lines plus these four deduplicated families leave
        // the final line available for integrity, regardless of arrival order.
        if state.emitted >= MAX_LINES {
            return;
        }
        match kind {
            EventKind::IngressState => state.state_ingress = true,
            EventKind::IngressSession => state.session_ingress = true,
            EventKind::DispositionState => state.state_disposition = true,
            EventKind::DispositionSession => state.session_disposition = true,
            EventKind::Overflow => unreachable!(),
        }
        state.emitted += 1;
        emit_closed(guard);
    }

    fn snapshot_summary(&self, snapshot: &SnapshotState, integrity: &'static str) {
        let Some(mut value) = snapshot_summary_value(snapshot, integrity) else {
            return;
        };
        let Ok(mut state) = self.0.lock() else {
            return;
        };
        if state.producer_active
            || state.emitted >= MAX_LINES
            || state.companion_emitted >= MAX_COMPANION_LINES
        {
            return;
        }
        if state.reserve_offline_roles {
            if state.server_emitted {
                return;
            }
            let Some(object) = value.as_object_mut() else {
                return;
            };
            object.insert("role".into(), Value::String("server".into()));
            state.server_emitted = true;
        }
        state.emitted += 1;
        state.companion_emitted += 1;
        let _ = writeln!(std::io::stdout().lock(), "{value}");
    }

    fn incomplete(&self, guard: &'static str) {
        let Ok(mut state) = self.0.lock() else {
            return;
        };
        if state.integrity.is_some() || state.emitted >= MAX_LINES {
            return;
        }
        state.integrity = Some(guard);
        if !state.producer_active {
            state.emitted += 1;
            emit_closed(guard);
        }
    }

    fn activate_producer(&self) {
        if let Ok(mut state) = self.0.lock() {
            state.producer_active = true;
        }
    }

    fn finalized(&self, state: &CaptureState) {
        let integrity = if let Ok(budget) = self.0.lock() {
            budget.integrity.unwrap_or({
                if !state.saw_state_ingress && !state.saw_session_ingress {
                    "capture_no_ingress"
                } else if (state.saw_state_ingress && !state.saw_state_disposition)
                    || (state.saw_session_ingress && !state.saw_session_disposition)
                {
                    "capture_channel_loss"
                } else if !state.saw_session_disposition {
                    "capture_no_session"
                } else {
                    "capture_complete"
                }
            })
        } else if (state.saw_state_ingress && !state.saw_state_disposition)
            || (state.saw_session_ingress && !state.saw_session_disposition)
        {
            "capture_channel_loss"
        } else if !state.saw_session_disposition {
            "capture_no_session"
        } else {
            "capture_complete"
        };
        self.snapshot_summary(&state.snapshot, integrity);
        self.final_integrity(integrity, &state.producer);
    }

    fn final_integrity(&self, integrity: &'static str, producer: &ProducerState) {
        let Ok(mut state) = self.0.lock() else {
            return;
        };
        if !state.producer_active {
            if state.integrity.is_none() && state.emitted < MAX_LINES {
                state.integrity = Some(integrity);
                state.emitted += 1;
                emit_closed(integrity);
            }
            return;
        }
        if state.emitted >= MAX_LINES {
            return;
        }
        let producer_integrity = if producer.overflow {
            "overflow"
        } else if producer.channel_loss {
            "channel_loss"
        } else if producer.saw_ingress {
            "complete"
        } else {
            "no_ingress"
        };
        state.integrity = Some(integrity);
        state.emitted += 1;
        let value = json!({
            "gate": GATE,
            "role": "herdr_receiver",
            "phase": "herdr_report",
            "guard": integrity,
            "code": "none",
            "io": "none",
            "ack": null,
            "producer_startup": producer.startup,
            "producer_reference": producer.reference,
            "producer_delivery": producer.delivery,
            "producer_integrity": producer_integrity,
            "producer_fallback_integrity": producer.fallback.integrity,
            "producer_fallback_module_entered": producer.fallback.module_entered,
            "producer_fallback_factory_entered": producer.fallback.factory_entered,
            "producer_fallback_socket": producer.fallback.socket,
        });
        let _ = writeln!(std::io::stdout().lock(), "{value}");
    }
}

fn snapshot_summary_value(snapshot: &SnapshotState, integrity: &'static str) -> Option<Value> {
    let failure = snapshot.startup_failure?;
    let write = snapshot.write.unwrap_or(SnapshotWrite {
        guard: "none",
        io: "none",
    });
    Some(json!({
        "gate": GATE,
        "capture": "session_snapshot",
        "phase": failure.phase,
        "guard": failure.guard,
        "io": failure.io,
        "stage": snapshot.stage.closed(),
        "write": write.guard,
        "write_io": write.io,
        "integrity": if snapshot.out_of_order { "out_of_order" } else { integrity },
        "code": "none",
        "ack": null,
    }))
}

fn emit_closed(guard: &str) {
    let value = json!({
        "gate": GATE,
        "role": "herdr_receiver",
        "phase": "herdr_report",
        "guard": guard,
        "code": "none",
        "io": "none",
        "ack": null,
    });
    let _ = writeln!(std::io::stdout().lock(), "{value}");
}

#[derive(Clone, Copy)]
struct PinnedDesktop {
    pid: u32,
    start_ticks: u64,
}

struct CaptureState {
    pinned: Option<PinnedDesktop>,
    saw_state_ingress: bool,
    saw_session_ingress: bool,
    saw_state_disposition: bool,
    saw_session_disposition: bool,
    snapshot: SnapshotState,
    producer: ProducerState,
}

pub(super) struct Capture {
    socket: PathBuf,
    token: String,
    budget: Budget,
    stop: Arc<AtomicBool>,
    state: Arc<Mutex<CaptureState>>,
    task: Option<thread::JoinHandle<()>>,
    producer_socket: Option<PathBuf>,
    producer_fallback: Option<ProducerFallbackFile>,
    producer_task: Option<thread::JoinHandle<()>>,
    producer_env: Option<(String, String)>,
}

impl Capture {
    pub(super) fn bind(
        root: &Path,
        owner_uid: u32,
        token: String,
        reserve_offline_roles: bool,
    ) -> io::Result<Self> {
        if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid private capture token",
            ));
        }
        let socket = root.join("herdr-report.sock");
        let listener = UnixListener::bind(&socket)?;
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        let budget = Budget::new(reserve_offline_roles);
        let stop = Arc::new(AtomicBool::new(false));
        let state = Arc::new(Mutex::new(CaptureState {
            pinned: None,
            saw_state_ingress: false,
            saw_session_ingress: false,
            saw_state_disposition: false,
            saw_session_disposition: false,
            snapshot: SnapshotState::default(),
            producer: ProducerState::default(),
        }));
        let task_budget = budget.clone();
        let task_stop = stop.clone();
        let task_state = state.clone();
        let task_token = token.clone();
        let task = thread::Builder::new()
            .name("goatr-herdr-report".into())
            .spawn(move || {
                let mut connections = 0usize;
                while !task_stop.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            connections += 1;
                            if connections > MAX_CONNECTIONS
                                || !accepted_stream(
                                    stream,
                                    owner_uid,
                                    &task_token,
                                    &task_budget,
                                    &task_stop,
                                    &task_state,
                                )
                            {
                                task_budget.incomplete("capture_channel_loss");
                            }
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => {
                            task_budget.incomplete("capture_channel_loss");
                            return;
                        }
                    }
                }
            })?;
        Ok(Self {
            socket,
            token,
            budget,
            stop,
            state,
            task: Some(task),
            producer_socket: None,
            producer_fallback: None,
            producer_task: None,
            producer_env: None,
        })
    }

    pub(super) fn configure(&self, command: &mut std::process::Command) {
        command
            .env("HERDR_GOATR_OMP_REPORT_CAPTURE", "1")
            .env("HERDR_GOATR_OMP_REPORT_SOCKET", &self.socket)
            .env("HERDR_GOATR_OMP_REPORT_TOKEN", &self.token);
    }

    /// Pins the real spawned desktop identity before receiver data is admitted.
    pub(super) fn pin(&self, process: &goatr::herdr::ProcessEvidence) {
        if let Ok(mut state) = self.state.lock() {
            state.pinned = Some(PinnedDesktop {
                pid: process.pid,
                start_ticks: process.start_ticks,
            });
        }
    }

    pub(super) fn budget(&self) -> Budget {
        self.budget.clone()
    }

    /// Keeps the first closed helper-side failure for the final bounded summary.
    pub(super) fn startup_failure(
        &self,
        phase: &'static str,
        guard: &'static str,
        io: &'static str,
    ) {
        if let Ok(mut state) = self.state.lock() {
            state
                .snapshot
                .startup_failure(SnapshotFailure { phase, guard, io });
        }
    }

    /// A valid helper snapshot settles the startup observation, not merely a server write.
    pub(super) fn startup_success(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.snapshot.startup_success();
        }
    }

    /// Prepares the one private replacement only after the caller selected the
    /// owned creation capture and published its relay record. The immutable
    /// settings proof is produced outside this helper by the approved read-only
    /// metadata reader; this method never invokes OMP or reads provider state.
    pub(super) fn prepare_omp_producer(
        &mut self,
        config: &goatr::config::Config,
        port: u16,
    ) -> io::Result<bool> {
        let requested = std::env::var_os("GOATR_OWNED_OMP_PRODUCER_CAPTURE");
        let reader = std::env::var_os("GOATR_OWNED_OMP_PRODUCER_METADATA_READER");
        let reader_sha = std::env::var("GOATR_OWNED_OMP_PRODUCER_METADATA_READER_SHA256").ok();
        let bun = std::env::var_os("GOATR_OWNED_OMP_PRODUCER_BUN");
        let bun_sha = std::env::var("GOATR_OWNED_OMP_PRODUCER_BUN_SHA256").ok();
        if requested.is_none()
            && reader.is_none()
            && reader_sha.is_none()
            && bun.is_none()
            && bun_sha.is_none()
        {
            return Ok(false);
        }
        if requested.as_deref() != Some(std::ffi::OsStr::new("1"))
            || std::env::var_os("GOATR_OWNED_OMP_CREATION_CAPTURE").as_deref()
                != Some(std::ffi::OsStr::new("1"))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid producer capture opt-in",
            ));
        }
        let (Some(reader), Some(reader_sha), Some(bun), Some(bun_sha)) =
            (reader, reader_sha, bun, bun_sha)
        else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "incomplete producer metadata reader",
            ));
        };
        let base_overlay = goatr::backends::omp::overlay(config, port)?;
        let hook = config.runtime_dir.join(PRODUCER_HOOK);
        let producer_socket = config.runtime_dir.join(PRODUCER_SOCKET);
        let producer_socket_agent = config.runtime_dir_agent.join(PRODUCER_SOCKET);
        let producer_fallback = config.runtime_dir.join(PRODUCER_FALLBACK);
        let producer_fallback_agent = config.runtime_dir_agent.join(PRODUCER_FALLBACK);
        let token = goatr::pairing::hex(&goatr::pairing::random32()?);
        write_private_new(
            &hook,
            &instrumented_hook(&producer_socket_agent, &producer_fallback_agent, &token)?,
        )?;
        let settings = run_metadata_reader(
            &PathBuf::from(bun),
            &bun_sha,
            &PathBuf::from(reader),
            &reader_sha,
            &hook,
            &base_overlay,
            &config.projects[0].path,
        )?;
        let hook_sha256 = goatr::pairing::hex(&goatr::protocol::sha256(&read_private_exact(
            &hook,
            &config.runtime_dir,
        )?));
        let overlay_sha256 = goatr::backends::omp::producer_overlay(
            config,
            port,
            &settings.disabled_extensions,
            &hook_sha256,
        )?;
        let listener = UnixListener::bind(&producer_socket)?;
        fs::set_permissions(&producer_socket, fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        self.budget.activate_producer();
        if let Ok(mut state) = self.state.lock() {
            state.producer.active = true;
        }
        let state = self.state.clone();
        let budget = self.budget.clone();
        let stop = self.stop.clone();
        let listener_token = token.clone();
        let task = thread::Builder::new()
            .name("goatr-omp-producer-report".into())
            .spawn(move || {
                producer_listener(
                    listener,
                    goatr::pairing::uid(),
                    listener_token,
                    state,
                    budget,
                    stop,
                )
            })?;
        self.producer_socket = Some(producer_socket);
        self.producer_fallback = Some(ProducerFallbackFile {
            path: producer_fallback,
            root: config.runtime_dir.clone(),
            token: token.clone(),
        });
        self.producer_task = Some(task);
        self.producer_env = Some((overlay_sha256, hook_sha256));
        Ok(true)
    }

    pub(super) fn configure_companion(&self, command: &mut std::process::Command) {
        if let Some((overlay, hook)) = &self.producer_env {
            command
                .env("GOATR_OWNED_OMP_PRODUCER_CAPTURE", "1")
                .env("GOATR_OWNED_OMP_PRODUCER_OVERLAY_SHA256", overlay)
                .env("GOATR_OWNED_OMP_PRODUCER_HOOK_SHA256", hook);
        }
    }

    fn finalize(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(task) = self.producer_task.take() {
            let _ = task.join();
        }
        if let Some(task) = self.task.take() {
            let _ = task.join();
        }
        if let Some(fallback) = self.producer_fallback.take() {
            let observed = read_producer_fallback(&fallback);
            if let Ok(mut state) = self.state.lock() {
                state.producer.fallback = observed;
            }
        }
        if let Ok(state) = self.state.lock() {
            self.budget.finalized(&state);
        } else {
            self.budget.incomplete("capture_channel_loss");
        }
        let _ = fs::remove_file(&self.socket);
        if let Some(socket) = self.producer_socket.take() {
            let _ = fs::remove_file(socket);
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.finalize();
    }
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn sha256_hex(bytes: &[u8]) -> String {
    goatr::pairing::hex(&goatr::protocol::sha256(bytes))
}

fn proof_array_hash(values: &[String]) -> io::Result<String> {
    serde_json::to_vec(values)
        .map(|value| sha256_hex(&value))
        .map_err(io::Error::other)
}

fn parse_settings_proof(bytes: &[u8]) -> io::Result<ProducerSettingsProof> {
    let proof: ProducerSettingsProof = serde_json::from_slice(bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid producer proof"))?;
    if proof.v != 1
        || !valid_sha256(&proof.extensions_sha256)
        || !valid_sha256(&proof.disabled_extensions_sha256)
        || proof.extensions.len() > 128
        || proof.disabled_extensions.len() > 128
        || proof.stock_hook_explicitly_configured
        || proof.stock_hook_disabled
        || proof.stock_hook_broadly_disabled
        || proof
            .extensions
            .iter()
            .chain(&proof.disabled_extensions)
            .any(|value| {
                value.is_empty() || value.len() > 1024 || value.chars().any(char::is_control)
            })
        || proof_array_hash(&proof.extensions)? != proof.extensions_sha256
        || proof_array_hash(&proof.disabled_extensions)? != proof.disabled_extensions_sha256
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "producer proof rejected",
        ));
    }
    Ok(proof)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MetadataEnvelope {
    proof: ProducerSettingsProof,
    audit: MetadataAudit,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct MetadataAudit {
    #[serde(rename = "sourceHashes")]
    source_hashes: Vec<(String, String)>,
    #[serde(rename = "mainYamlSha256")]
    main_yaml_sha256: String,
    #[serde(rename = "stockSha256")]
    stock_sha256: String,
    #[serde(rename = "replacementSha256")]
    replacement_sha256: String,
    #[serde(rename = "environmentSha256")]
    environment_sha256: String,
    #[serde(rename = "beforeCount")]
    before_count: u64,
    #[serde(rename = "afterCount")]
    after_count: u64,
    #[serde(rename = "beforeSha256")]
    before_sha256: String,
    #[serde(rename = "afterSha256")]
    after_sha256: String,
    #[serde(rename = "unrelatedSha256")]
    unrelated_sha256: String,
    #[serde(rename = "pointInTimeOnly")]
    point_in_time_only: bool,
    #[serde(rename = "noAgentSession")]
    no_agent_session: bool,
}

fn verified_executable(path: &Path, expected_sha256: &str) -> io::Result<()> {
    if !valid_sha256(expected_sha256) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid metadata executable hash",
        ));
    }
    goatr::config::absolute(path)?;
    if fs::canonicalize(path)? != path {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "metadata executable path changed",
        ));
    }
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file()
        || metadata.uid() != goatr::pairing::uid()
        || metadata.mode() & 0o022 != 0
        || metadata.mode() & 0o111 == 0
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unsafe metadata executable",
        ));
    }
    let mut bytes = Vec::new();
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?
        .take(128 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 128 * 1024 * 1024 || sha256_hex(&bytes) != expected_sha256 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "metadata executable changed",
        ));
    }
    Ok(())
}

fn verified_private_reader(path: &Path, expected_sha256: &str) -> io::Result<()> {
    if !valid_sha256(expected_sha256) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid metadata reader hash",
        ));
    }
    goatr::config::absolute(path)?;
    if fs::canonicalize(path)? != path {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "metadata reader path changed",
        ));
    }
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file()
        || metadata.uid() != goatr::pairing::uid()
        || metadata.mode() & 0o7777 != 0o600
        || metadata.nlink() != 1
        || metadata.len() > 64 * 1024
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unsafe metadata reader",
        ));
    }
    let mut bytes = Vec::new();
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?
        .take(64 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 64 * 1024 || sha256_hex(&bytes) != expected_sha256 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "metadata reader changed",
        ));
    }
    Ok(())
}

fn closed_audit(audit: &MetadataAudit) -> bool {
    const SOURCES: [&str; 6] = [
        "src/config/settings.ts",
        "src/extensibility/settings.ts",
        "src/extensibility/extensions/loader.ts",
        "src/discovery/index.ts",
        "src/discovery/omp-extension-roots.ts",
        "node_modules/@oh-my-pi/pi-utils/src/dirs.ts",
    ];
    audit.source_hashes.len() == SOURCES.len()
        && audit
            .source_hashes
            .iter()
            .map(|(path, hash)| (path.as_str(), hash.as_str()))
            .eq(SOURCES
                .into_iter()
                .zip(audit.source_hashes.iter().map(|(_, hash)| hash.as_str())))
        && audit
            .source_hashes
            .iter()
            .all(|(_, hash)| valid_sha256(hash))
        && [
            &audit.main_yaml_sha256,
            &audit.stock_sha256,
            &audit.replacement_sha256,
            &audit.environment_sha256,
            &audit.before_sha256,
            &audit.after_sha256,
            &audit.unrelated_sha256,
        ]
        .into_iter()
        .all(|value| valid_sha256(value))
        && audit.before_count <= 256
        && audit.after_count <= 256
        && audit.point_in_time_only
        && audit.no_agent_session
}

fn run_metadata_reader(
    bun: &Path,
    bun_sha256: &str,
    reader: &Path,
    reader_sha256: &str,
    hook: &Path,
    base_overlay: &Path,
    cwd: &Path,
) -> io::Result<ProducerSettingsProof> {
    verified_executable(bun, bun_sha256)?;
    verified_private_reader(reader, reader_sha256)?;
    let mut command = Command::new(bun);
    command
        .arg(reader)
        .arg(hook)
        .arg(base_overlay)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // SAFETY: the child runs only the pinned metadata reader and drops every
    // inherited capability set before exec, before it can open configuration.
    unsafe {
        command.pre_exec(goatr::codex_relay::drop_inherited_capabilities);
    }
    let mut child = command.spawn()?;
    let mut bytes = Vec::new();
    child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("metadata stdout unavailable"))?
        .take(64 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if !child.wait()?.success() || bytes.len() > 64 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "metadata reader rejected",
        ));
    }
    let envelope: MetadataEnvelope = serde_json::from_slice(&bytes).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid metadata reader output",
        )
    })?;
    if !closed_audit(&envelope.audit) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "metadata audit rejected",
        ));
    }
    let audit_path = hook
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "metadata audit root missing"))?
        .join("omp-producer-metadata-audit.json");
    write_private_new(
        &audit_path,
        &serde_json::to_vec(&envelope.audit).map_err(io::Error::other)?,
    )?;
    parse_settings_proof(&serde_json::to_vec(&envelope.proof).map_err(io::Error::other)?)
}

fn read_private_exact(path: &Path, root: &Path) -> io::Result<Vec<u8>> {
    goatr::config::absolute(path)?;
    let canonical = fs::canonicalize(path)?;
    if canonical != path || !canonical.starts_with(root) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "private hook path changed",
        ));
    }
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file()
        || metadata.uid() != goatr::pairing::uid()
        || metadata.mode() & 0o7777 != 0o600
        || metadata.nlink() != 1
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unsafe private hook",
        ));
    }
    let mut bytes = Vec::new();
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?
        .take(64 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 64 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "private hook exceeded bound",
        ));
    }
    Ok(bytes)
}

fn write_private_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if bytes.len() > 64 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "private hook exceeded bound",
        ));
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct PrivateIdentity {
    dev: u64,
    ino: u64,
    uid: u32,
    mode: u32,
    nlink: u64,
    len: u64,
    mtime: i64,
    mtime_nsec: i64,
    ctime: i64,
    ctime_nsec: i64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct PrivateDirectoryIdentity {
    dev: u64,
    ino: u64,
    uid: u32,
    mode: u32,
}

fn private_identity(metadata: &fs::Metadata) -> PrivateIdentity {
    PrivateIdentity {
        dev: metadata.dev(),
        ino: metadata.ino(),
        uid: metadata.uid(),
        mode: metadata.mode() & 0o7777,
        nlink: metadata.nlink(),
        len: metadata.len(),
        mtime: metadata.mtime(),
        mtime_nsec: metadata.mtime_nsec(),
        ctime: metadata.ctime(),
        ctime_nsec: metadata.ctime_nsec(),
    }
}

fn private_fallback_root(root: &Path) -> Option<PrivateDirectoryIdentity> {
    if goatr::config::absolute(root).is_err()
        || fs::canonicalize(root).ok().as_deref() != Some(root)
    {
        return None;
    }
    let metadata = fs::symlink_metadata(root).ok()?;
    if !metadata.is_dir() || metadata.uid() != goatr::pairing::uid() || metadata.mode() & 0o077 != 0
    {
        return None;
    }
    Some(PrivateDirectoryIdentity {
        dev: metadata.dev(),
        ino: metadata.ino(),
        uid: metadata.uid(),
        mode: metadata.mode() & 0o7777,
    })
}

// The sealed token attributes this file to the capture but does not authenticate a peer;
// the independent native identity admission remains the Unix-socket collector.
fn read_producer_fallback(file: &ProducerFallbackFile) -> ProducerFallback {
    let missing = || ProducerFallback::default();
    let inconclusive = || ProducerFallback {
        integrity: "inconclusive",
        ..ProducerFallback::default()
    };
    if file.path.parent() != Some(file.root.as_path()) || !valid_capture_token(&file.token) {
        return inconclusive();
    }
    let Some(root_before) = private_fallback_root(&file.root) else {
        return inconclusive();
    };
    let path_before = match fs::symlink_metadata(&file.path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return missing(),
        Err(_) => return inconclusive(),
    };
    if !path_before.is_file()
        || path_before.uid() != goatr::pairing::uid()
        || path_before.mode() & 0o7777 != 0o600
        || path_before.nlink() != 1
        || path_before.len() > PRODUCER_FALLBACK_MAX_BYTES as u64
        || fs::canonicalize(&file.path).ok().as_deref() != Some(file.path.as_path())
    {
        return inconclusive();
    }
    let before = private_identity(&path_before);
    let mut handle = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&file.path)
    {
        Ok(handle) => handle,
        Err(_) => return inconclusive(),
    };
    if private_identity(&match handle.metadata() {
        Ok(metadata) => metadata,
        Err(_) => return inconclusive(),
    }) != before
    {
        return inconclusive();
    }
    let mut bytes = Vec::new();
    if std::io::Read::by_ref(&mut handle)
        .take(PRODUCER_FALLBACK_MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() > PRODUCER_FALLBACK_MAX_BYTES
    {
        return inconclusive();
    }
    let path_after = match fs::symlink_metadata(&file.path) {
        Ok(metadata) => metadata,
        Err(_) => return inconclusive(),
    };
    if private_fallback_root(&file.root) != Some(root_before)
        || private_identity(&path_after) != before
        || handle
            .metadata()
            .ok()
            .map(|metadata| private_identity(&metadata))
            != Some(before)
    {
        return inconclusive();
    }
    parse_producer_fallback(&bytes, &file.token).unwrap_or_else(inconclusive)
}

fn valid_capture_token(token: &str) -> bool {
    token.len() == 64 && token.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn parse_producer_fallback(bytes: &[u8], token: &str) -> Option<ProducerFallback> {
    if !valid_capture_token(token)
        || bytes.is_empty()
        || !bytes.ends_with(b"\n")
        || bytes.len() > PRODUCER_FALLBACK_MAX_BYTES
    {
        return None;
    }
    let text = std::str::from_utf8(bytes).ok()?;
    let records: Vec<_> = text.strip_suffix('\n')?.split('\n').collect();
    if records.is_empty() || records.len() > PRODUCER_FALLBACK_MAX_RECORDS {
        return None;
    }
    let expected = [
        "module_entered",
        "factory_entered",
        "socket_connected",
        "socket_not_found",
        "socket_refused",
        "socket_permission_denied",
        "socket_other",
    ];
    let mut events = Vec::with_capacity(records.len());
    for record in records {
        let event = record.strip_prefix(token)?.strip_prefix(' ')?;
        if !expected.contains(&event) || record != format!("{token} {event}") {
            return None;
        }
        events.push(event);
    }
    if events.first() != Some(&"module_entered")
        || events
            .iter()
            .filter(|event| **event == "module_entered")
            .count()
            != 1
        || events
            .iter()
            .filter(|event| **event == "factory_entered")
            .count()
            > 1
        || events
            .iter()
            .filter(|event| event.starts_with("socket_"))
            .count()
            > 1
    {
        return None;
    }
    if events.len() > 1 && events[1] != "factory_entered" {
        return None;
    }
    if events.len() > 2 && !events[2].starts_with("socket_") {
        return None;
    }
    if events.len() > 3 {
        return None;
    }
    let socket = match events
        .iter()
        .copied()
        .find(|event| event.starts_with("socket_"))
    {
        None => "none",
        Some("socket_connected") => "connected",
        Some("socket_not_found") => "not_found",
        Some("socket_refused") => "refused",
        Some("socket_permission_denied") => "permission_denied",
        Some("socket_other") => "other",
        Some(_) => return None,
    };
    Some(ProducerFallback {
        integrity: "complete",
        module_entered: true,
        factory_entered: events
            .get(1)
            .is_some_and(|event| *event == "factory_entered"),
        socket,
    })
}

fn stock_hook() -> io::Result<String> {
    let path = Path::new(STOCK_HOOK);
    goatr::config::absolute(path)?;
    if fs::canonicalize(path)? != path || fs::symlink_metadata(path)?.uid() != goatr::pairing::uid()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "stock hook identity changed",
        ));
    }
    let mut bytes = Vec::new();
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?
        .take(64 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 64 * 1024 || sha256_hex(&bytes) != STOCK_HOOK_SHA256 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "stock hook bytes changed",
        ));
    }
    String::from_utf8(bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "stock hook is not UTF-8"))
}

fn replace_once(source: String, needle: &str, replacement: &str) -> io::Result<String> {
    if source.matches(needle).count() != 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "stock hook anchor changed",
        ));
    }
    Ok(source.replacen(needle, replacement, 1))
}

fn instrumented_hook(socket: &Path, fallback: &Path, token: &str) -> io::Result<Vec<u8>> {
    goatr::config::absolute(socket)?;
    goatr::config::absolute(fallback)?;
    if !valid_capture_token(token) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid producer token",
        ));
    }
    let socket = serde_json::to_string(socket.to_str().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "producer socket is not UTF-8")
    })?)
    .map_err(io::Error::other)?;
    let fallback = serde_json::to_string(fallback.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "producer fallback is not UTF-8",
        )
    })?)
    .map_err(io::Error::other)?;
    let token = serde_json::to_string(token).map_err(io::Error::other)?;
    let diagnostic = format!(
        r#"
import fs from "node:fs";
const GOATR_PRODUCER_SOCKET = {socket};
const GOATR_PRODUCER_FALLBACK = {fallback};
const GOATR_PRODUCER_TOKEN = {token};
const goatrProducerEvents = new Set<string>();
let goatrProducerSocket: import("node:net").Socket | undefined;
let goatrProducerClosed = false;
let goatrFirstSessionDelivery = false;
let goatrFallbackFd: number | undefined;
let goatrFallbackClosed = false;
let goatrFallbackRecords = 0;
let goatrFirstSocketOutcome = false;
function goatrFallback(event: string): void {{
  try {{
    if (goatrFallbackClosed || goatrFallbackRecords >= 4) return;
    if (goatrFallbackFd === undefined) {{
      goatrFallbackFd = fs.openSync(
        GOATR_PRODUCER_FALLBACK,
        fs.constants.O_WRONLY | fs.constants.O_CREAT | fs.constants.O_EXCL | fs.constants.O_NOFOLLOW,
        0o600,
      );
      fs.fchmodSync(goatrFallbackFd, 0o600);
    }}
    fs.writeSync(goatrFallbackFd, `${{GOATR_PRODUCER_TOKEN}} ${{event}}\n`);
    goatrFallbackRecords += 1;
  }} catch {{
    goatrFallbackClosed = true;
  }}
}}
function goatrSocketOutcome(outcome: "connected" | "not_found" | "refused" | "permission_denied" | "other"): void {{
  if (goatrFirstSocketOutcome) return;
  goatrFirstSocketOutcome = true;
  goatrFallback(`socket_${{outcome}}`);
}}
function goatrSocketError(error: unknown): void {{
  const code = (error as {{ code?: unknown }})?.code;
  goatrSocketOutcome(
    code === "ENOENT" ? "not_found" : code === "ECONNREFUSED" ? "refused" : code === "EACCES" || code === "EPERM" ? "permission_denied" : "other",
  );
}}
function goatrProducer(event: string): void {{
  if (goatrProducerClosed || goatrProducerEvents.has(event)) return;
  goatrProducerEvents.add(event);
  try {{
    if (!goatrProducerSocket) {{
      goatrProducerSocket = net.createConnection(GOATR_PRODUCER_SOCKET);
      goatrProducerSocket.once("connect", () => goatrSocketOutcome("connected"));
      goatrProducerSocket.once("error", (error) => {{ goatrSocketError(error); goatrProducerClosed = true; }});
      goatrProducerSocket.once("close", () => {{ goatrSocketOutcome("other"); goatrProducerClosed = true; }});
      goatrProducerSocket.unref?.();
    }}
    goatrProducerSocket.write(`${{GOATR_PRODUCER_TOKEN}} ${{event}}\n`);
  }} catch (error) {{
    goatrSocketError(error);
    goatrProducerClosed = true;
  }}
}}
function goatrSessionDelivery(request: any, outcome: string): void {{
  if (goatrFirstSessionDelivery || request?.method !== "pane.report_agent_session") return;
  goatrFirstSessionDelivery = true;
  goatrProducer(`delivery_${{outcome}}`);
}}
goatrFallback("module_entered");
"#
    );
    let mut source = stock_hook()?;
    source = replace_once(
        source,
        "import path from \"node:path\";\n",
        &format!("import path from \"node:path\";{diagnostic}"),
    )?;
    source = replace_once(
        source,
        "export default function (pi) {\n",
        "export default function (pi) {\n  goatrFallback(\"factory_entered\");\n",
    )?;
    source = replace_once(
        source,
        "  if (!enabled()) {\n    return;\n  }\n\n  let agentActive",
        "  if (!enabled()) {\n    goatrProducer(HERDR_ENV !== \"1\" ? \"startup_disabled_env\" : !socketPath ? \"startup_disabled_socket\" : !paneId ? \"startup_disabled_pane\" : \"startup_disabled_nested\");\n    return;\n  }\n  goatrProducer(\"startup_loaded\");\n\n  let agentActive",
    )?;
    source = replace_once(
        source,
        "    if (ctx?.hasUI !== true) {\n      return false;\n    }",
        "    if (ctx?.hasUI !== true) {\n      goatrProducer(\"startup_callback_not_ui\");\n      return false;\n    }\n    goatrProducer(\"startup_callback\");",
    )?;
    source = replace_once(
        source,
        "  if (!sessionRef) {\n    return Promise.resolve();\n  }",
        "  if (!sessionRef) {\n    goatrProducer(\"reference_absent\");\n    return Promise.resolve();\n  }\n  goatrProducer(currentAgentSessionPath ? \"reference_path\" : \"reference_id\");",
    )?;
    source = replace_once(
        source,
        "function sendRequest(request: unknown): Promise<void> {",
        "function sendRequest(request: unknown): Promise<void> {\n  if ((request as any)?.method === \"pane.report_agent_session\" && !goatrFirstSessionDelivery) goatrProducer(\"delivery_queued\");",
    )?;
    source = replace_once(
        source,
        "const finish = (delivered: boolean) => {",
        "const finish = (delivered: boolean, outcome = delivered ? \"response_bytes\" : \"socket_error\") => {",
    )?;
    source = replace_once(
        source,
        "      resolve(delivered);",
        "      goatrSessionDelivery(request, outcome);\n      resolve(delivered);",
    )?;
    source = replace_once(
        source,
        "socket.on(\"error\", () => finish(false));",
        "socket.on(\"error\", () => finish(false, \"socket_error\"));",
    )?;
    source = replace_once(
        source,
        "socket.on(\"data\", () => finish(true));",
        "socket.on(\"data\", () => finish(true, \"response_bytes\"));",
    )?;
    source = replace_once(
        source,
        "socket.on(\"end\", () => finish(false));",
        "socket.on(\"end\", () => finish(false, \"eof\"));",
    )?;
    source = replace_once(
        source,
        "timeout = setTimeout(() => finish(false), timeoutMs);",
        "timeout = setTimeout(() => finish(false, \"timeout\"), timeoutMs);",
    )?;
    Ok(source.into_bytes())
}

fn producer_listener(
    listener: UnixListener,
    owner_uid: u32,
    token: String,
    state: Arc<Mutex<CaptureState>>,
    budget: Budget,
    stop: Arc<AtomicBool>,
) {
    let mut connections = 0usize;
    while !stop.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _)) => {
                connections += 1;
                if connections > PRODUCER_MAX_CONNECTIONS
                    || !accepted_producer_stream(stream, owner_uid, &token, &state, &stop)
                {
                    producer_loss(&state, false);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5))
            }
            Err(_) => {
                producer_loss(&state, false);
                break;
            }
        }
    }
    let _ = budget;
}

fn producer_loss(state: &Mutex<CaptureState>, overflow: bool) {
    if let Ok(mut state) = state.lock() {
        state.producer.channel_loss = true;
        state.producer.overflow |= overflow;
    }
}

fn producer_descends_from(
    peer_pid: u32,
    pinned: PinnedDesktop,
) -> Option<goatr::herdr::ProcessEvidence> {
    let Ok(mut process) = goatr::herdr::process_evidence(peer_pid) else {
        return None;
    };
    let original = process.clone();
    for _ in 0..128 {
        if process.pid == pinned.pid && process.start_ticks == pinned.start_ticks {
            return Some(original);
        }
        let Ok(stat) = fs::read_to_string(format!("/proc/{}/stat", process.pid)) else {
            return None;
        };
        let end = stat.rfind(')')?;
        let parent = stat[end + 2..]
            .split_whitespace()
            .nth(1)
            .and_then(|value| value.parse::<u32>().ok())
            .filter(|value| *value != 0)?;
        if goatr::herdr::process_evidence(process.pid).ok().as_ref() != Some(&process) {
            return None;
        }
        let Ok(next) = goatr::herdr::process_evidence(parent) else {
            return None;
        };
        process = next;
    }
    None
}

fn accepted_producer_stream(
    mut stream: UnixStream,
    owner_uid: u32,
    token: &str,
    state: &Mutex<CaptureState>,
    stop: &AtomicBool,
) -> bool {
    let Some((peer_pid, peer_uid)) = peer_credential(&stream) else {
        return false;
    };
    let pinned = state.lock().ok().and_then(|state| state.pinned);
    let Some(pinned) = pinned else {
        return false;
    };
    if peer_uid != owner_uid || stream.set_nonblocking(true).is_err() {
        return false;
    }
    let Some(process) = producer_descends_from(peer_pid, pinned) else {
        return false;
    };
    if let Ok(mut state) = state.lock() {
        if state
            .producer
            .process
            .as_ref()
            .is_some_and(|pinned| pinned != &process)
        {
            return false;
        }
        state.producer.process.get_or_insert(process);
    } else {
        return false;
    }
    let mut frame = Vec::with_capacity(MAX_FRAME);
    let mut frames = 0usize;
    let mut total = 0usize;
    let mut bytes = [0u8; 128];
    loop {
        match stream.read(&mut bytes) {
            Ok(0) => return frame.is_empty(),
            Ok(read) => {
                total += read;
                if total > PRODUCER_MAX_TOTAL_BYTES {
                    producer_loss(state, true);
                    return false;
                }
                for byte in &bytes[..read] {
                    if *byte == b'\n' {
                        frames += 1;
                        if frames > PRODUCER_MAX_FRAMES
                            || !accept_producer_frame(&frame, token, state)
                        {
                            producer_loss(state, frames > PRODUCER_MAX_FRAMES);
                            return false;
                        }
                        frame.clear();
                    } else if frame.len() == MAX_FRAME {
                        producer_loss(state, true);
                        return false;
                    } else {
                        frame.push(*byte);
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if stop.load(Ordering::Acquire) {
                    return frame.is_empty();
                }
                thread::sleep(Duration::from_millis(5));
            }
            Err(_) => return false,
        }
    }
}

fn accept_producer_frame(frame: &[u8], token: &str, state: &Mutex<CaptureState>) -> bool {
    let Ok(frame) = std::str::from_utf8(frame) else {
        return false;
    };
    let Some(event) = frame
        .strip_prefix(token)
        .and_then(|value| value.strip_prefix(' '))
    else {
        return false;
    };
    let Ok(mut state) = state.lock() else {
        return false;
    };
    state.producer.saw_ingress = true;
    match event {
        "startup_loaded" => state.producer.startup = "loaded",
        "startup_disabled" => state.producer.startup = "disabled",
        "startup_disabled_env" => state.producer.startup = "disabled_env",
        "startup_disabled_socket" => state.producer.startup = "disabled_socket",
        "startup_disabled_pane" => state.producer.startup = "disabled_pane",
        "startup_disabled_nested" => state.producer.startup = "disabled_nested",
        "startup_callback_not_ui" => state.producer.startup = "callback_not_ui",
        "startup_callback" => state.producer.startup = "callback",
        "reference_path" => {
            if state.producer.reference == "none" {
                state.producer.reference = "path";
            }
        }
        "reference_id" | "reference_absent" => {
            if state.producer.reference == "none" {
                state.producer.reference = if event == "reference_id" {
                    "id"
                } else {
                    "absent"
                };
            }
        }
        "delivery_queued"
        | "delivery_socket_error"
        | "delivery_response_bytes"
        | "delivery_eof"
        | "delivery_timeout" => {
            let next = match event {
                "delivery_queued" => "queued",
                "delivery_socket_error" => "socket_error",
                "delivery_response_bytes" => "response_bytes",
                "delivery_eof" => "eof",
                "delivery_timeout" => "timeout",
                _ => unreachable!(),
            };
            if state.producer.delivery == "none" || state.producer.delivery == "queued" {
                state.producer.delivery = next;
            }
        }
        _ => return false,
    }
    true
}

fn accepted_stream(
    mut stream: UnixStream,
    owner_uid: u32,
    token: &str,
    budget: &Budget,
    stop: &AtomicBool,
    state: &Mutex<CaptureState>,
) -> bool {
    let Some((peer_pid, peer_uid)) = peer_credential(&stream) else {
        return false;
    };
    // Copy the pin in a separate scope: frame admission takes this mutex too.
    let pinned = {
        let Ok(state) = state.lock() else {
            return false;
        };
        state.pinned
    };
    let Some(pinned) = pinned else {
        return false;
    };
    // Retain start evidence privately with the pin; peer credentials bind this
    // single stream to the real spawned desktop PID and current user.
    let _ = pinned.start_ticks;
    if peer_uid != owner_uid || peer_pid != pinned.pid {
        return false;
    }
    if stream.set_nonblocking(true).is_err() {
        return false;
    }
    let mut frame = Vec::with_capacity(MAX_FRAME);
    let mut discard = false;
    let mut bytes = [0u8; 128];
    // On shutdown, drain bytes already available without waiting for new data.
    // A finite frame bound also prevents a busy peer from extending shutdown.
    let mut frames = 0usize;
    let mut total_bytes = 0usize;
    loop {
        match stream.read(&mut bytes) {
            Ok(0) => {
                if discard || !frame.is_empty() {
                    budget.incomplete("capture_truncated");
                }
                return true;
            }
            Ok(read) => {
                total_bytes += read;
                if total_bytes > 64 * (MAX_FRAME + 1) {
                    budget.incomplete("capture_overflow");
                    return false;
                }
                for byte in &bytes[..read] {
                    if *byte == b'\n' {
                        frames += 1;
                        if frames > 64 {
                            budget.incomplete("capture_overflow");
                            return false;
                        }
                        if discard {
                            discard = false;
                            budget.incomplete("capture_truncated");
                        } else if !accept_frame(&frame, token, budget, state) {
                            return false;
                        }
                        frame.clear();
                    } else if discard {
                        continue;
                    } else if frame.len() == MAX_FRAME {
                        frame.clear();
                        discard = true;
                    } else {
                        frame.push(*byte);
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if stop.load(Ordering::Acquire) {
                    if discard || !frame.is_empty() {
                        budget.incomplete("capture_truncated");
                    }
                    return true;
                }
                thread::sleep(Duration::from_millis(5));
            }
            Err(_) => return false,
        }
    }
}

fn accept_frame(frame: &[u8], token: &str, budget: &Budget, state: &Mutex<CaptureState>) -> bool {
    let Ok(frame) = std::str::from_utf8(frame) else {
        return false;
    };
    let Some(event) = frame
        .strip_prefix(token)
        .and_then(|rest| rest.strip_prefix(' '))
    else {
        return false;
    };
    if let Some(snapshot) = snapshot_event(event) {
        let Ok(mut state) = state.lock() else {
            return false;
        };
        state.snapshot.observe(snapshot);
        return true;
    }
    let Some(kind) = closed_event(event) else {
        return false;
    };
    if kind == EventKind::Overflow {
        budget.incomplete("capture_overflow");
        return true;
    }
    let Ok(mut state) = state.lock() else {
        return false;
    };
    match kind {
        EventKind::IngressState => state.saw_state_ingress = true,
        EventKind::IngressSession => state.saw_session_ingress = true,
        EventKind::DispositionState => {
            // A pre-dispatch rejection is itself proof of receiver ingress.
            state.saw_state_ingress = true;
            state.saw_state_disposition = true;
        }
        EventKind::DispositionSession => {
            state.saw_session_ingress = true;
            state.saw_session_disposition = true;
        }
        EventKind::Overflow => unreachable!(),
    }
    drop(state);
    budget.receiver(event);
    true
}

fn snapshot_event(event: &str) -> Option<SnapshotEvent> {
    match event {
        "snapshot_parsed" => Some(SnapshotEvent::Stage(SnapshotStage::Parsed)),
        "snapshot_enqueued" => Some(SnapshotEvent::Stage(SnapshotStage::Enqueued)),
        "snapshot_dispatched" => Some(SnapshotEvent::Stage(SnapshotStage::Dispatched)),
        "snapshot_enqueue_rejected" => Some(SnapshotEvent::Write(SnapshotWrite {
            guard: "enqueue_rejected",
            io: "none",
        })),
        "snapshot_written" => Some(SnapshotEvent::Write(SnapshotWrite {
            guard: "written",
            io: "none",
        })),
        "snapshot_peer_closed_broken_pipe" => Some(SnapshotEvent::Write(SnapshotWrite {
            guard: "peer_closed",
            io: "broken_pipe",
        })),
        "snapshot_peer_closed_connection_reset" => Some(SnapshotEvent::Write(SnapshotWrite {
            guard: "peer_closed",
            io: "connection_reset",
        })),
        "snapshot_peer_closed_not_connected" => Some(SnapshotEvent::Write(SnapshotWrite {
            guard: "peer_closed",
            io: "not_connected",
        })),
        "snapshot_peer_closed_other" => Some(SnapshotEvent::Write(SnapshotWrite {
            guard: "peer_closed",
            io: "other",
        })),
        "snapshot_write_error_broken_pipe" => Some(SnapshotEvent::Write(SnapshotWrite {
            guard: "write_error",
            io: "broken_pipe",
        })),
        "snapshot_write_error_connection_reset" => Some(SnapshotEvent::Write(SnapshotWrite {
            guard: "write_error",
            io: "connection_reset",
        })),
        "snapshot_write_error_not_connected" => Some(SnapshotEvent::Write(SnapshotWrite {
            guard: "write_error",
            io: "not_connected",
        })),
        "snapshot_write_error_other" => Some(SnapshotEvent::Write(SnapshotWrite {
            guard: "write_error",
            io: "other",
        })),
        _ => None,
    }
}

fn closed_event(event: &str) -> Option<EventKind> {
    match event {
        "receiver_state_ingress" => Some(EventKind::IngressState),
        "receiver_session_ingress" => Some(EventKind::IngressSession),
        "receiver_state_pane_invalid"
        | "receiver_state_agent_invalid"
        | "receiver_session_only"
        | "receiver_recent_exit"
        | "receiver_stale"
        | "receiver_agent_conflict"
        | "receiver_owner_conflict"
        | "receiver_session_conflict"
        | "receiver_retained_pending"
        | "receiver_installed" => Some(EventKind::DispositionState),
        "receiver_session_pane_invalid"
        | "receiver_session_agent_invalid"
        | "receiver_session_missing"
        | "receiver_session_replacement_pending"
        | "receiver_session_start_unrecognized"
        | "receiver_session_sequence_missing"
        | "receiver_session_stale"
        | "receiver_session_retained_pending"
        | "receiver_session_sequence_rejected"
        | "receiver_session_agent_conflict"
        | "receiver_session_process_absent"
        | "receiver_session_owner_conflict"
        | "receiver_session_owner_session_conflict"
        | "receiver_session_replacement_rejected"
        | "receiver_session_reconciled_present"
        | "receiver_session_reconciled_absent"
        | "receiver_session_installed" => Some(EventKind::DispositionSession),
        "overflow" => Some(EventKind::Overflow),
        _ => None,
    }
}

fn peer_credential(stream: &UnixStream) -> Option<(u32, u32)> {
    let mut credential = std::mem::MaybeUninit::<libc::ucred>::zeroed();
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: `credential` points to sufficient writable storage and the FD is live.
    if unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            credential.as_mut_ptr().cast(),
            &mut length,
        )
    } != 0
        || length as usize != std::mem::size_of::<libc::ucred>()
    {
        return None;
    }
    // SAFETY: successful getsockopt initialized exactly `ucred` bytes.
    let credential = unsafe { credential.assume_init() };
    Some((credential.pid.try_into().ok()?, credential.uid))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{net::Shutdown, sync::mpsc};

    fn socket_frames(payload: &[u8]) -> (bool, Arc<Mutex<CaptureState>>, Budget) {
        let (mut sender, receiver) = UnixStream::pair().unwrap();
        let state = Arc::new(Mutex::new(CaptureState {
            pinned: Some(PinnedDesktop {
                pid: std::process::id(),
                start_ticks: 0,
            }),
            saw_state_ingress: false,
            saw_session_ingress: false,
            saw_state_disposition: false,
            saw_session_disposition: false,
            snapshot: SnapshotState::default(),
            producer: ProducerState::default(),
        }));
        let budget = Budget::new(false);
        let reader_state = state.clone();
        let reader_budget = budget.clone();
        let (tx, rx) = mpsc::channel();
        let task = thread::spawn(move || {
            // SAFETY: geteuid has no pointer arguments or preconditions.
            let uid = unsafe { libc::geteuid() };
            let result = accepted_stream(
                receiver,
                uid,
                "token",
                &reader_budget,
                &AtomicBool::new(false),
                &reader_state,
            );
            let _ = tx.send(result);
        });
        sender.write_all(payload).unwrap();
        sender.shutdown(Shutdown::Write).unwrap();
        let result = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("collector must finish without retaining its state mutex");
        task.join().unwrap();
        (result, state, budget)
    }

    #[test]
    fn socket_keeps_session_disposition_distinct_from_state_and_finishes() {
        let (ok, state, budget) = socket_frames(b"token receiver_state_ingress\ntoken receiver_installed\ntoken receiver_session_ingress\ntoken receiver_session_retained_pending\n");
        assert!(ok);
        let state = state.lock().unwrap();
        assert!(state.saw_state_disposition && state.saw_session_disposition);
        budget.finalized(&state);
        let budget = budget.0.lock().unwrap();
        assert_eq!(budget.integrity, Some("capture_complete"));
        assert!(budget.state_disposition && budget.session_disposition);
    }

    #[test]
    fn socket_partial_eof_preserves_truncation_instead_of_complete() {
        let (ok, state, budget) = socket_frames(b"token receiver_session_ingress\ntoken receiver_session_installed\ntoken receiver_session_sta");
        assert!(ok);
        budget.finalized(&state.lock().unwrap());
        assert_eq!(
            budget.0.lock().unwrap().integrity,
            Some("capture_truncated")
        );
    }

    #[test]
    fn socket_rejection_itself_proves_ingress() {
        let (ok, state, budget) = socket_frames(b"token receiver_session_agent_invalid\n");
        assert!(ok);
        budget.finalized(&state.lock().unwrap());
        assert_eq!(budget.0.lock().unwrap().integrity, Some("capture_complete"));
    }

    #[test]
    fn socket_wrong_token_cannot_set_original_report_flags() {
        let (ok, state, budget) = socket_frames(b"wrong receiver_session_installed\n");
        assert!(!ok);
        let state = state.lock().unwrap();
        assert!(!state.saw_session_ingress && !state.saw_session_disposition);
        budget.finalized(&state);
        assert_eq!(
            budget.0.lock().unwrap().integrity,
            Some("capture_no_ingress")
        );
    }

    #[test]
    fn socket_finite_frame_limit_reports_overflow() {
        let payload = b"token receiver_session_ingress\n".repeat(65);
        let (ok, state, budget) = socket_frames(&payload);
        assert!(!ok);
        budget.finalized(&state.lock().unwrap());
        assert_eq!(budget.0.lock().unwrap().integrity, Some("capture_overflow"));
    }

    #[test]
    fn omp_producer_closed_events_retain_no_payload_or_identity() {
        let state = Mutex::new(CaptureState {
            pinned: None,
            saw_state_ingress: false,
            saw_session_ingress: false,
            saw_state_disposition: false,
            saw_session_disposition: false,
            snapshot: SnapshotState::default(),
            producer: ProducerState {
                active: true,
                ..ProducerState::default()
            },
        });
        assert!(accept_producer_frame(
            b"token startup_disabled_socket",
            "token",
            &state
        ));
        assert!(accept_producer_frame(
            b"token reference_path",
            "token",
            &state
        ));
        assert!(accept_producer_frame(
            b"token delivery_response_bytes",
            "token",
            &state
        ));
        assert!(!accept_producer_frame(
            b"token delivery_response_bytes payload",
            "token",
            &state
        ));
        assert!(!accept_producer_frame(
            b"other startup_loaded",
            "token",
            &state
        ));
        let state = state.lock().unwrap();
        assert_eq!(state.producer.startup, "disabled_socket");
        assert_eq!(state.producer.reference, "path");
        assert_eq!(state.producer.delivery, "response_bytes");
    }

    #[test]
    fn omp_producer_proof_rejects_existing_stock_exclusion() {
        let extensions = Vec::<String>::new();
        let disabled_extensions = Vec::<String>::new();
        let value = json!({
            "v": 1,
            "extensions": extensions,
            "disabledExtensions": disabled_extensions,
            "extensionsSha256": proof_array_hash(&[]).unwrap(),
            "disabledExtensionsSha256": proof_array_hash(&[]).unwrap(),
            "stockHookExplicitlyConfigured": false,
            "stockHookDisabled": true,
            "stockHookBroadlyDisabled": false,
        });
        assert!(parse_settings_proof(&serde_json::to_vec(&value).unwrap()).is_err());
    }

    #[test]
    fn shared_budget_reserves_receiver_dispositions_after_one_of_each_companion_observation() {
        let budget = Budget::new(true);
        budget.omp(json!({"gate":"component_test"}));
        budget.phone(json!({"gate":"component_test"}));
        budget.child(json!({"gate":"component_test"}));
        budget.omp(json!({"gate":"component_test"}));
        budget.phone(json!({"gate":"component_test"}));
        budget.child(json!({"gate":"component_test"}));
        for _ in 0..20 {
            budget.receiver("receiver_state_ingress");
            budget.receiver("receiver_session_ingress");
        }
        budget.receiver("receiver_installed");
        budget.receiver("receiver_session_retained_pending");
        budget.incomplete("capture_channel_loss");
        let state = budget.0.lock().unwrap();
        assert_eq!(state.emitted, MAX_LINES);
        assert_eq!(state.companion_emitted, MAX_COMPANION_LINES);
        assert!(state.server_emitted && state.phone_emitted && state.child_emitted);
        assert!(state.state_disposition && state.session_disposition);
        assert_eq!(state.integrity, Some("capture_channel_loss"));
    }

    #[test]
    fn creation_budget_reserves_terminal_after_two_generic_observations() {
        let budget = Budget::new(false);
        for _ in 0..3 {
            budget.omp(json!({"gate":"component_test"}));
        }
        budget.omp(json!({
            "phase":"omp_ready", "guard":"original_agent_absent",
            "code":"NotReady", "io":"none", "frame":"none"
        }));
        budget.phone(json!({"gate":"component_test"}));
        budget.child(json!({"gate":"component_test"}));
        let state = budget.0.lock().unwrap();
        assert_eq!(state.companion_emitted, MAX_COMPANION_LINES);
        assert_eq!(state.creation_nonterminal_lines, 2);
        assert!(state.creation_terminal_emitted);
        assert!(!state.server_emitted && !state.phone_emitted && !state.child_emitted);
    }

    #[test]
    fn creation_budget_retains_shell_probe_terminal_after_two_nonterminal_records() {
        let budget = Budget::new(false);
        budget.omp(json!({"phase":"herdr_launch", "guard":"ack_matched"}));
        budget.omp(json!({"phase":"follower", "guard":"follower_registry_read"}));
        budget.omp(json!({"phase":"follower", "guard":"follower_registry_snapshot"}));
        budget.omp(json!({
            "phase":"shell_probe", "guard":"root_recheck_focus",
            "code":"NotReady", "io":"none", "frame":"none"
        }));

        let state = budget.0.lock().unwrap();
        assert_eq!(state.companion_emitted, MAX_COMPANION_LINES);
        assert_eq!(state.creation_nonterminal_lines, 2);
        assert!(state.creation_terminal_emitted);
    }

    #[test]
    fn creation_budget_reserves_native_guards_and_receiver_dispositions_after_transport_progress() {
        let budget = Budget::new(false);
        for guard in [
            "dispatch_returned",
            "response_queued",
            "output_written_after_queue",
        ] {
            budget.omp(json!({"phase":"transport", "guard":guard}));
        }
        assert_eq!(budget.0.lock().unwrap().companion_emitted, 0);

        budget.omp(json!({"phase":"herdr_launch", "guard":"ack_matched"}));
        budget.omp(json!({"phase":"omp_ready", "guard":"topology_not_ready"}));
        budget.omp(json!({"phase":"omp_ready", "guard":"native_report_absent"}));
        budget.omp(json!({"phase":"follower", "guard":"follower_native_report_absent"}));
        budget.omp(json!({"phase":"omp_ready", "guard":"report_absent_after_interactive_ready"}));
        budget.receiver("receiver_state_ingress");
        budget.receiver("receiver_session_ingress");
        budget.receiver("receiver_installed");
        budget.receiver("receiver_session_retained_pending");
        budget.incomplete("capture_complete");

        let state = budget.0.lock().unwrap();
        assert_eq!(state.companion_emitted, MAX_COMPANION_LINES);
        assert_eq!(state.creation_nonterminal_lines, 2);
        assert!(state.creation_terminal_emitted);
        assert!(state.state_disposition && state.session_disposition);
        assert_eq!(state.integrity, Some("capture_complete"));
        assert_eq!(state.emitted, MAX_LINES);
    }

    #[test]
    fn coarse_shell_budget_is_backend_only_once_and_keeps_creation_arrival_order() {
        let coarse = json!({"phase":"coarse_shell","guard":"installed","code":"none","io":"none","frame":"none","ack":null});
        let creation = Budget::new(false);
        for guard in ["follower_other", "follower_history"] {
            creation.omp(json!({"phase":"follower","guard":guard}));
        }
        creation.omp(coarse.clone());
        creation.omp(json!({"phase":"omp_ready","guard":"report_absent_after_interactive_ready"}));
        assert_eq!(creation.creation_counts(), (3, 2, true));
        assert!(!creation.0.lock().unwrap().coarse_shell_settled);
        let offline = Budget::new(true);
        offline.omp(coarse.clone());
        assert_eq!(offline.0.lock().unwrap().emitted, 0);
        let backend = Budget::new_backend();
        for _ in 0..20 {
            backend.omp(json!({"phase":"follower","guard":"follower_other"}));
        }
        backend.omp(coarse.clone());
        backend.omp(coarse);
        backend.omp(json!({"phase":"herdr_launch","guard":"ack_matched"}));
        backend.omp(json!({"phase":"omp_ready","guard":"topology_not_ready"}));
        let state = backend.0.lock().unwrap();
        assert_eq!(state.emitted, 3);
        assert!(
            state.coarse_shell_settled
                && state.backend_launch_emitted
                && state.backend_readiness_emitted
        );
    }

    #[test]
    fn retained_backend_capture_acknowledges_only_delivered_probe_disposition() {
        let budget = Budget::new_backend();
        assert!(!budget.shell_probe_captured());
        budget.omp(json!({"phase":"coarse_shell","guard":"installed","code":"none","io":"none","frame":"none"}));
        budget.omp(json!({"phase":"herdr_launch","guard":"ack_matched","code":"none","io":"none","frame":"none"}));
        assert!(!budget.shell_probe_captured());
        budget.omp(json!({"phase":"shell_probe","guard":"result_read_failed","code":"NotReady","io":"none","frame":"none"}));
        assert!(budget.shell_probe_captured());
        let before = budget.0.lock().unwrap().emitted;
        budget.omp(json!({"phase":"shell_probe","guard":"result_read_failed","code":"NotReady","io":"none","frame":"none"}));
        assert_eq!(budget.0.lock().unwrap().emitted, before);
        assert!(budget.shell_probe_captured());
    }

    #[test]
    fn omp_launch_capture_backend_budget_reserves_launch_and_readiness_after_generic_chatter() {
        let budget = Budget::new_backend();
        for _ in 0..20 {
            budget.omp(json!({"phase":"follower", "guard":"follower_other"}));
        }
        budget.omp(json!({"phase":"herdr_launch", "guard":"ack_matched"}));
        budget.omp(json!({"phase":"omp_ready", "guard":"topology_not_ready"}));
        for guard in [
            "native_report_absent",
            "native_report_absent",
            "native_report_wrong",
            "no_connection",
        ] {
            budget.omp(json!({"phase":"omp_ready", "guard":guard}));
        }
        budget.omp(json!({"phase":"herdr_launch", "guard":"unavailable"}));
        for guard in [
            "follower_native_report_absent",
            "follower_native_report_wrong",
            "follower_registry_absent",
            "follower_registry_read",
        ] {
            budget.omp(json!({"phase":"follower", "guard":guard}));
        }
        let state = budget.0.lock().unwrap();
        assert_eq!(state.emitted, MAX_LINES);
        assert_eq!(state.companion_emitted, MAX_LINES);
        assert!(state.backend_launch_emitted && state.backend_readiness_emitted);
        assert!(
            state
                .backend_seen
                .contains(&("omp_ready".into(), "native_report_absent".into()))
        );
        assert!(
            state
                .backend_seen
                .contains(&("omp_ready".into(), "native_report_wrong".into()))
        );
        assert!(
            state
                .backend_seen
                .contains(&("omp_ready".into(), "no_connection".into()))
        );
        assert!(
            state
                .backend_seen
                .contains(&("herdr_launch".into(), "unavailable".into()))
        );
        assert!(
            !state
                .backend_seen
                .contains(&("follower".into(), "follower_registry_read".into()))
        );
    }

    #[test]
    fn startup_snapshot_preserves_first_failure_and_monotonic_stage() {
        let mut snapshot = SnapshotState::default();
        snapshot.observe(SnapshotEvent::Stage(SnapshotStage::Parsed));
        snapshot.observe(SnapshotEvent::Stage(SnapshotStage::Dispatched));
        snapshot.observe(SnapshotEvent::Stage(SnapshotStage::Enqueued));
        snapshot.startup_failure(SnapshotFailure {
            phase: "herdr_snapshot",
            guard: "response_timeout",
            io: "timed_out",
        });
        snapshot.startup_failure(SnapshotFailure {
            phase: "herdr_connect",
            guard: "other",
            io: "other",
        });
        assert_eq!(snapshot.stage, SnapshotStage::Dispatched);
        assert_eq!(snapshot.startup_failure.unwrap().guard, "response_timeout");
        assert!(snapshot.out_of_order);
    }

    #[test]
    fn startup_snapshot_written_then_client_timeout_still_reports() {
        let mut snapshot = SnapshotState::default();
        snapshot.observe(SnapshotEvent::Stage(SnapshotStage::Parsed));
        snapshot.observe(SnapshotEvent::Write(SnapshotWrite {
            guard: "written",
            io: "none",
        }));
        snapshot.startup_failure(SnapshotFailure {
            phase: "herdr_snapshot",
            guard: "response_timeout",
            io: "timed_out",
        });
        let summary = snapshot_summary_value(&snapshot, "capture_complete").unwrap();
        assert_eq!(summary["phase"], "herdr_snapshot");
        assert_eq!(summary["guard"], "response_timeout");
        assert_eq!(summary["io"], "timed_out");
        assert_eq!(summary["write"], "written");
    }

    #[test]
    fn startup_snapshot_later_helper_success_has_no_failure_summary() {
        let mut snapshot = SnapshotState::default();
        snapshot.observe(SnapshotEvent::Stage(SnapshotStage::Parsed));
        snapshot.startup_failure(SnapshotFailure {
            phase: "herdr_connect",
            guard: "connection_refused",
            io: "connection_refused",
        });
        snapshot.startup_success();
        assert!(snapshot_summary_value(&snapshot, "capture_complete").is_none());
    }

    #[test]
    fn startup_snapshot_empty_and_dispatched_timeouts_remain_distinct() {
        let failure = SnapshotFailure {
            phase: "herdr_snapshot",
            guard: "response_timeout",
            io: "timed_out",
        };
        let mut empty = SnapshotState::default();
        empty.startup_failure(failure);
        let mut dispatched = SnapshotState::default();
        dispatched.observe(SnapshotEvent::Stage(SnapshotStage::Parsed));
        dispatched.observe(SnapshotEvent::Stage(SnapshotStage::Enqueued));
        dispatched.observe(SnapshotEvent::Stage(SnapshotStage::Dispatched));
        dispatched.startup_failure(failure);
        assert_eq!(
            snapshot_summary_value(&empty, "capture_no_ingress").unwrap()["stage"],
            "none"
        );
        assert_eq!(
            snapshot_summary_value(&dispatched, "capture_complete").unwrap()["stage"],
            "dispatched"
        );
    }

    #[test]
    fn only_closed_receiver_events_are_accepted() {
        assert_eq!(
            closed_event("receiver_session_retained_pending"),
            Some(EventKind::DispositionSession)
        );
        assert_eq!(closed_event("untrusted payload"), None);
    }

    #[test]
    fn ingress_and_disposition_have_distinct_closed_classes() {
        assert_eq!(
            closed_event("receiver_state_ingress"),
            Some(EventKind::IngressState)
        );
        assert_eq!(
            closed_event("receiver_session_installed"),
            Some(EventKind::DispositionSession)
        );
    }
    #[test]
    fn omp_producer_fallback_rejects_partial_or_reordered_records() {
        let token = "a".repeat(64);
        let valid =
            format!("{token} module_entered\n{token} factory_entered\n{token} socket_refused\n");
        let fallback = parse_producer_fallback(valid.as_bytes(), &token).expect("closed fallback");
        assert_eq!(fallback.integrity, "complete");
        assert!(fallback.module_entered && fallback.factory_entered);
        assert_eq!(fallback.socket, "refused");
        assert!(
            parse_producer_fallback(format!("{token} factory_entered\n").as_bytes(), &token)
                .is_none()
        );
        assert!(
            parse_producer_fallback(
                format!("{token} module_entered\n{token} socket_other").as_bytes(),
                &token
            )
            .is_none()
        );
    }

    #[test]
    fn omp_producer_generated_hook_preserves_stock_delivery_calls_and_closed_fallback() {
        let token = "b".repeat(64);
        let hook = String::from_utf8(
            instrumented_hook(
                Path::new("/tmp/goatr-producer.sock"),
                Path::new("/tmp/goatr-producer-fallback.jsonl"),
                &token,
            )
            .expect("generated hook"),
        )
        .expect("hook UTF-8");
        assert_eq!(
            hook.matches("await sendRequestAttempt(request, 500)")
                .count(),
            1
        );
        assert_eq!(
            hook.matches("await sendRequestAttempt(request, 1500)")
                .count(),
            1
        );
        assert!(hook.contains("goatrFallback(\"module_entered\")"));
        assert!(hook.contains("goatrFallback(\"factory_entered\")"));
        assert!(hook.contains("O_EXCL | fs.constants.O_NOFOLLOW"));
        assert!(hook.contains("goatrSocketOutcome(\"connected\")"));
    }

    #[test]
    #[ignore = "requires GOATR_HOOK_TEST_NODE and executes only the generated private fixture"]
    fn omp_producer_generated_hook_reports_local_and_absent_socket_once() {
        use std::os::unix::process::ExitStatusExt;
        use std::time::Instant;
        #[derive(Debug)]
        struct Facts {
            phase: &'static str,
            name: &'static str,
            code: &'static str,
            stderr_present: bool,
            stderr_truncated: bool,
            stderr_integrity: bool,
            received: usize,
            pre_kill: &'static str,
            exit: Option<i32>,
            signal: Option<i32>,
            killed: bool,
            reaped: bool,
            io: &'static str,
        }
        fn runner_source(url: &str) -> Vec<u8> {
            let mut source = String::from(
                r#"process.stderr.write("goatr_hook_runner phase=runner_entered name=none code=none\n");
let phase = "importing";
try {
  const hook = (await import("#,
            );
            source.push_str(&serde_json::to_string(url).expect("fixture URL"));
            source.push_str(r#")).default;
  phase = "factory";
  hook({ events: { on() {} }, on() {} });
  process.stderr.write("goatr_hook_runner phase=factory_returned name=none code=none\n");
} catch (error) {
  let name = "other", code = "other";
  try {
    const n = error?.name, c = error?.code;
    if (["Error", "TypeError", "SyntaxError", "RangeError", "ReferenceError"].includes(n)) name = n;
    if (c === undefined) code = "none";
    else if (["ERR_MODULE_NOT_FOUND", "ERR_UNKNOWN_FILE_EXTENSION", "ERR_UNSUPPORTED_NODE_MODULES_TYPE_STRIPPING", "ERR_UNSUPPORTED_TYPESCRIPT_SYNTAX", "ERR_INVALID_TYPESCRIPT_SYNTAX", "ERR_INVALID_ARG_TYPE", "ENOENT", "EACCES", "ECONNREFUSED"].includes(c)) code = c;
  } catch {}
  process.stderr.write("goatr_hook_runner phase=" + phase + " name=" + name + " code=" + code + "\n");
  process.exitCode = 1;
}
setTimeout(() => process.exit(process.exitCode || 0), 100);
"#);
            source.into_bytes()
        }
        fn nonblocking(fd: i32) -> bool {
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
            flags >= 0 && unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == 0
        }
        fn drain(stderr: &mut std::process::ChildStderr, saved: &mut Vec<u8>, facts: &mut Facts) {
            let mut chunk = [0; 256];
            for _ in 0..64 {
                match stderr.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        facts.stderr_present = true;
                        let retain = n.min(512 - saved.len());
                        saved.extend_from_slice(&chunk[..retain]);
                        facts.stderr_truncated |= retain < n;
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(_) => {
                        facts.io = "stderr";
                        break;
                    }
                }
            }
        }
        fn drain_socket(
            listener: Option<&UnixListener>,
            stream: &mut Option<UnixStream>,
            received: &mut Vec<u8>,
            facts: &mut Facts,
            deadline: Instant,
        ) {
            if facts.io != "none" || Instant::now() >= deadline {
                return;
            }
            if stream.is_none()
                && let Some(listener) = listener
            {
                match listener.accept() {
                    Ok((accepted, _)) => {
                        if accepted.set_nonblocking(true).is_err() {
                            facts.io = "socket_mode";
                        } else {
                            *stream = Some(accepted);
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                    Err(_) => facts.io = "accept",
                }
            }
            if let Some(stream) = stream {
                let mut bytes = [0; 256];
                for _ in 0..8 {
                    if Instant::now() >= deadline {
                        break;
                    }
                    match stream.read(&mut bytes) {
                        Ok(0) => break,
                        Ok(n) => {
                            if received.len() + n > PRODUCER_MAX_TOTAL_BYTES {
                                facts.io = "frame_limit";
                                break;
                            }
                            received.extend_from_slice(&bytes[..n]);
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                        Err(_) => {
                            facts.io = "socket_read";
                            break;
                        }
                    }
                }
            }
        }
        fn run(
            node: &std::ffi::OsStr,
            root: &Path,
            runner: &Path,
            listener: Option<&UnixListener>,
            deadline: Instant,
        ) -> (Vec<u8>, Facts) {
            let mut facts = Facts {
                phase: "none",
                name: "none",
                code: "none",
                stderr_present: false,
                stderr_truncated: false,
                stderr_integrity: true,
                received: 0,
                pre_kill: "not_started",
                exit: None,
                signal: None,
                killed: false,
                reaped: false,
                io: "none",
            };
            if Instant::now() >= deadline {
                return (Vec::new(), facts);
            }
            // Establish the fallible cleanup owner before there is any Child to lose.
            let cleanup_slot = Arc::new(Mutex::new(None::<std::process::Child>));
            let owned_slot = cleanup_slot.clone();
            let (wake_reaper, wakeups) = mpsc::sync_channel::<()>(1);
            let (send_status, receive_status) = mpsc::sync_channel(1);
            if thread::Builder::new()
                .spawn(move || {
                    let mut disconnected = false;
                    loop {
                        if let Some(mut owned) = owned_slot
                            .lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .take()
                        {
                            drop(wakeups);
                            let _ = send_status.send(owned.wait());
                            return;
                        }
                        if disconnected {
                            return;
                        }
                        disconnected = matches!(
                            wakeups.recv_timeout(Duration::from_millis(1)),
                            Err(mpsc::RecvTimeoutError::Disconnected)
                        );
                    }
                })
                .is_err()
            {
                facts.io = "reaper_start";
                return (Vec::new(), facts);
            }
            let mut child = Command::new(node)
                .arg("--experimental-strip-types")
                .arg(runner)
                .current_dir(root)
                .env_remove("OMPCODE")
                .env("HERDR_ENV", "1")
                .env("HERDR_SOCKET_PATH", "fixture")
                .env("HERDR_PANE_ID", "fixture")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap_or_else(|_| panic!("owned hook fixture_spawn_failed"));
            let mut stderr = child.stderr.take().expect("fixture stderr owner");
            let stderr_nonblocking = nonblocking(stderr.as_raw_fd());
            if !stderr_nonblocking {
                facts.io = "stderr_mode";
            }
            let mut saved = Vec::with_capacity(512);
            let mut received = Vec::with_capacity(PRODUCER_MAX_TOTAL_BYTES);
            let mut stream: Option<UnixStream> = None;
            // Cleanup shares this consumer's two-second budget; it gets no extra allowance.
            let observe_until = deadline
                .checked_sub(Duration::from_millis(20))
                .unwrap_or(deadline);
            let mut observed = None;
            while facts.io == "none" && Instant::now() < observe_until {
                drain(&mut stderr, &mut saved, &mut facts);
                drain_socket(listener, &mut stream, &mut received, &mut facts, deadline);
                match child.try_wait() {
                    Ok(Some(status)) => {
                        observed = Some((status, Instant::now()));
                        break;
                    }
                    Ok(None) => {}
                    Err(_) => {
                        facts.io = "status";
                        break;
                    }
                }
                thread::sleep(Duration::from_millis(5));
            }
            let before = observed.or_else(|| {
                child
                    .try_wait()
                    .unwrap_or_else(|_| {
                        facts.io = "status";
                        None
                    })
                    .map(|status| (status, Instant::now()))
            });
            facts.pre_kill = match before {
                Some((status, _)) if status.success() => "exited_ok",
                Some((status, _)) if status.signal().is_some() => "signal",
                Some(_) => "exited_error",
                None => "running",
            };
            let status = if before.is_some() {
                before
            } else {
                facts.killed = child.kill().is_ok();
                *cleanup_slot
                    .lock()
                    .unwrap_or_else(|error| error.into_inner()) = Some(child);
                // This channel carries only a wakeup. The already-established owner retains
                // the actual Child in its slot or stack even if the wakeup send is closed.
                let _ = wake_reaper.send(());
                receive_status
                    .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                    .ok()
                    .and_then(Result::ok)
                    .map(|status| (status, Instant::now()))
            };
            if let Some((status, observed_at)) = status {
                facts.reaped = observed_at <= deadline;
                facts.exit = status.code().filter(|code| (0..=255).contains(code));
                facts.signal = status.signal().filter(|signal| (1..=64).contains(signal));
            }
            // Exit can race an earlier WouldBlock. Accept/read its queued final frame now,
            // nonblocking and only while the same original aggregate still permits it.
            drain_socket(listener, &mut stream, &mut received, &mut facts, deadline);
            if stderr_nonblocking {
                drain(&mut stderr, &mut saved, &mut facts);
            }
            if let Ok(text) = std::str::from_utf8(&saved) {
                for line in text.lines() {
                    let mut fields = line.split_whitespace();
                    if fields.next() != Some("goatr_hook_runner") {
                        facts.stderr_integrity = false;
                        continue;
                    }
                    facts.phase = match fields.next() {
                        Some("phase=runner_entered") => "runner_entered",
                        Some("phase=importing") => "importing",
                        Some("phase=factory") => "factory",
                        Some("phase=factory_returned") => "factory_returned",
                        _ => {
                            facts.stderr_integrity = false;
                            "other"
                        }
                    };
                    facts.name = match fields.next() {
                        Some("name=none") => "none",
                        Some("name=Error") => "Error",
                        Some("name=TypeError") => "TypeError",
                        Some("name=SyntaxError") => "SyntaxError",
                        Some("name=RangeError") => "RangeError",
                        Some("name=ReferenceError") => "ReferenceError",
                        Some("name=other") => "other",
                        _ => {
                            facts.stderr_integrity = false;
                            "other"
                        }
                    };
                    facts.code = match fields.next() {
                        Some("code=none") => "none",
                        Some("code=ERR_MODULE_NOT_FOUND") => "ERR_MODULE_NOT_FOUND",
                        Some("code=ERR_UNKNOWN_FILE_EXTENSION") => "ERR_UNKNOWN_FILE_EXTENSION",
                        Some("code=ERR_UNSUPPORTED_NODE_MODULES_TYPE_STRIPPING") => {
                            "ERR_UNSUPPORTED_NODE_MODULES_TYPE_STRIPPING"
                        }
                        Some("code=ERR_UNSUPPORTED_TYPESCRIPT_SYNTAX") => {
                            "ERR_UNSUPPORTED_TYPESCRIPT_SYNTAX"
                        }
                        Some("code=ERR_INVALID_TYPESCRIPT_SYNTAX") => {
                            "ERR_INVALID_TYPESCRIPT_SYNTAX"
                        }
                        Some("code=ERR_INVALID_ARG_TYPE") => "ERR_INVALID_ARG_TYPE",
                        Some("code=ENOENT") => "ENOENT",
                        Some("code=EACCES") => "EACCES",
                        Some("code=ECONNREFUSED") => "ECONNREFUSED",
                        Some("code=other") => "other",
                        _ => {
                            facts.stderr_integrity = false;
                            "other"
                        }
                    };
                    if fields.next().is_some() {
                        facts.stderr_integrity = false;
                    }
                }
            } else {
                facts.stderr_integrity = false;
            }
            saved.fill(0);
            facts.received = received.len();
            facts.reaped &= Instant::now() <= deadline;
            (received, facts)
        }
        let node = std::env::var_os("GOATR_HOOK_TEST_NODE")
            .expect("set the qualified TypeScript-capable Node executable");
        let root = std::env::temp_dir().join(format!(
            "gpf-{}",
            goatr::pairing::hex(&goatr::pairing::random32().expect("test token")[..8])
        ));
        fs::create_dir(&root).expect("private fixture root");
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))
            .expect("private fixture permissions");
        let listener_path = root.join("producer.sock");
        let listener = UnixListener::bind(&listener_path).expect("local diagnostic socket");
        listener
            .set_nonblocking(true)
            .expect("nonblocking listener");
        let token = "c".repeat(64);
        for (name, socket, expected_socket, local) in [
            ("local", listener_path, "connected", true),
            ("absent", root.join("absent.sock"), "not_found", false),
        ] {
            let fallback = root.join(format!("{name}-fallback.jsonl"));
            let hook = root.join(format!("{name}.ts"));
            write_private_new(
                &hook,
                &instrumented_hook(&socket, &fallback, &token).expect("generated hook"),
            )
            .expect("private generated fixture");
            let runner = root.join(format!("{name}-runner.mjs"));
            write_private_new(
                &runner,
                &runner_source(&format!("file://{}", hook.display())),
            )
            .expect("private fixture runner");
            let deadline = Instant::now() + Duration::from_secs(2);
            let (received, facts) = run(
                &node,
                &root,
                &runner,
                if local { Some(&listener) } else { None },
                deadline,
            );
            let fallback = read_producer_fallback(&ProducerFallbackFile {
                path: fallback,
                root: root.clone(),
                token: token.clone(),
            });
            let failure = format!(
                "case={name} runner={facts:?} fallback_integrity={} module={} factory={} socket={}",
                fallback.integrity,
                fallback.module_entered,
                fallback.factory_entered,
                fallback.socket
            );
            assert!(
                facts.reaped
                    && facts.pre_kill == "exited_ok"
                    && facts.exit == Some(0)
                    && !facts.killed
                    && facts.io == "none"
                    && facts.stderr_integrity
                    && facts.phase == "factory_returned"
                    && facts.name == "none"
                    && facts.code == "none",
                "{failure}"
            );
            assert!(
                if local {
                    received.starts_with(format!("{token} startup_loaded\n").as_bytes())
                } else {
                    received.is_empty()
                },
                "{failure}"
            );
            assert!(
                fallback.integrity == "complete"
                    && fallback.module_entered
                    && fallback.factory_entered
                    && fallback.socket == expected_socket,
                "{failure}"
            );
        }
        let _ = fs::remove_dir_all(root);
    }
}
