// SPDX-License-Identifier: GPL-3.0-or-later
//! Public Herdr protocol22 boundary. Topology is evidence, never native provider readiness.
use crate::{
    config::{self, Config},
    native_json,
    pairing::{
        self, Authorized, CreationAdmissionFailure, CreationAdmissionGuard, Security,
        SecurityResult, hex,
    },
    protocol::*,
    state::{ProviderSnapshot, State},
    storage::*,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    ffi::CString,
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom},
    os::{
        fd::{AsFd, AsRawFd, FromRawFd, OwnedFd},
        unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
    sync::watch,
};
mod worktree_creation;
pub(crate) mod shells;
pub(crate) mod grid;
pub(crate) mod endpoint;
pub mod artifacts;
pub(crate) mod creation_directories;
pub(crate) use creation_directories::CreationSelection;
pub(crate) mod skills;
pub use shells::ShellProof;
use shells::{NativeShellInfo, ShellBinding};
pub use worktree_creation::ExecutionCheckout;
/// One-shot observation prepared before any fixed shell-query input. The original
/// process-info/idle proof moves here rather than being repeated by the consumer.
pub(crate) struct PreparedShell<'a> {
    action:&'a Uuid,
    params:&'a SessionCreateParams,
    selection:&'a CreationSelection,
    deadline:tokio::time::Instant,
    resources:CreationResources,
    focus:Id,
    source:Source,
    info:PaneProcessInfo,
    shell:ProcessEvidence,
    original_reporter:ProcessEvidence,
    executable:PathBuf,
    dialect:ShellDialect,
}

const READ_TIMEOUT: Duration = Duration::from_secs(5);
/// Bounded OMP creation and post-Ready offline diagnostics. Callers supply only closed
/// classifications, so native responses, routes, commands, and provider values cannot reach
/// the helper output.
#[derive(Clone)]
pub(crate) struct OmpCreationReporter(Arc<Mutex<OmpCreationState>>);
#[derive(Clone, Copy, PartialEq, Eq)]
enum OmpReporterMode {
    Creation,
    Backend,
    Offline,
}
#[derive(Clone)]
struct OmpOfflineOriginal {
    target: Target,
    epoch: Uuid,
    server_id: Uuid,
    incarnation: Uuid,
    evidence: ServerEvidence,
    terminal_id: Id,
}
struct ClosedReport {
    code: Option<ErrorCode>,
    io: &'static str,
    frame: &'static str,
    probe_deadline:Option<(&'static str,ProbeObservation,&'static str)>,
}
#[derive(Clone,Copy,Debug,PartialEq,Eq,PartialOrd,Ord)]
enum ProbeObservation {NotRead,EmptyOnly,PartialSeen,CompleteSeen}
impl ProbeObservation {
    fn observe(&mut self,bytes:&[u8]) {
        let observed=if bytes.ends_with(b"\n"){Self::CompleteSeen}else if bytes.is_empty(){Self::EmptyOnly}else{Self::PartialSeen};
        if observed>*self {*self=observed;}
    }
    fn as_str(self)->&'static str {
        match self {Self::NotRead=>"not_read",Self::EmptyOnly=>"empty_only",Self::PartialSeen=>"partial_seen",Self::CompleteSeen=>"complete_seen"}
    }
}
fn probe_entry_budget(now:tokio::time::Instant,deadline:tokio::time::Instant)->&'static str {
    if now>=deadline {"expired"}
    else {let remaining=deadline-now;if remaining<=Duration::from_secs(1){"le_1s"}else if remaining<=Duration::from_secs(5){"le_5s"}else{"gt_5s"}}
}
/// Closed observations from the existing coarse traversal; never retain native values.
#[derive(Clone, Copy)]
enum CoarseShellGuard {
    ConsentAbsent,
    SourceLookup,
    ReporterProcess,
    Connect,
    PeerIdentity,
    ProcessInfo,
    ShellAbsent,
    ReportedProcess,
    ShellExecutable,
    SourceRecheck,
    NoCandidate,
    RecordAbsent,
    Installed,
}
impl CoarseShellGuard {
    fn name(self) -> &'static str {
        match self {
            Self::ConsentAbsent => "consent_absent",
            Self::SourceLookup => "source_lookup",
            Self::ReporterProcess => "reporter_process",
            Self::Connect => "connect",
            Self::PeerIdentity => "peer_identity",
            Self::ProcessInfo => "process_info",
            Self::ShellAbsent => "shell_absent",
            Self::ReportedProcess => "reported_process",
            Self::ShellExecutable => "shell_executable",
            Self::SourceRecheck => "source_recheck",
            Self::NoCandidate => "no_candidate",
            Self::RecordAbsent => "record_absent",
            Self::Installed => "installed",
        }
    }
}
#[derive(Clone, Copy)]
struct CoarseShellFailure {
    guard: CoarseShellGuard,
    code: ErrorCode,
    io: &'static str,
}
impl CoarseShellFailure {
    fn new(guard: CoarseShellGuard) -> Self {
        Self { guard, code: ErrorCode::NotReady, io: "none" }
    }
    fn io(guard: CoarseShellGuard, error: &io::Error) -> Self {
        let kind = match error.kind() {
            io::ErrorKind::NotFound => "not_found",
            io::ErrorKind::PermissionDenied => "permission_denied",
            io::ErrorKind::ConnectionRefused => "connection_refused",
            io::ErrorKind::ConnectionReset => "connection_reset",
            io::ErrorKind::ConnectionAborted => "connection_aborted",
            io::ErrorKind::NotConnected => "not_connected",
            io::ErrorKind::BrokenPipe => "broken_pipe",
            io::ErrorKind::TimedOut => "timed_out",
            io::ErrorKind::UnexpectedEof => "unexpected_eof",
            io::ErrorKind::InvalidData => "invalid_data",
            _ => "other",
        };
        Self {
            guard,
            code: if error.kind() == io::ErrorKind::OutOfMemory { ErrorCode::ResourceLimit } else { ErrorCode::NotReady },
            io: kind,
        }
    }
}
#[derive(Clone, Copy)]
enum BackendPriority {
    Launch,
    Readiness,
}
// Keep backend-only output ordered around the existing closed native families. Generic
// diagnostics are silent until both first native observations have their reserved slots.
fn backend_priority(phase: &'static str, guard: &'static str) -> Option<BackendPriority> {
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
        ("launch_preflight", _) => Some(BackendPriority::Launch),
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
fn backend_report_admits(state: &OmpCreationState, priority: Option<BackendPriority>) -> bool {
    if state.backend_launch_emitted && state.backend_readiness_emitted {
        return state.lines < 8;
    }
    match priority {
        Some(BackendPriority::Launch) => !state.backend_launch_emitted && state.lines < 8,
        Some(BackendPriority::Readiness) => !state.backend_readiness_emitted && state.lines < 8,
        None => state.backend_launch_emitted && state.backend_readiness_emitted && state.lines < 8,
    }
}

fn transport_progress(phase: &'static str, guard: &'static str) -> bool {
    matches!(
        (phase, guard),
        ("transport", "dispatch_returned")
            | ("transport", "response_queued")
            | ("transport", "output_written_after_queue")
    )
}

fn creation_transient_native_report(phase: &'static str, guard: &'static str) -> bool {
    matches!(
        (phase, guard),
        ("omp_ready", "native_report_absent") | ("follower", "follower_native_report_absent")
    )
}

fn creation_terminal_readiness(phase: &'static str, guard: &'static str) -> bool {
    matches!(
        (phase, guard),
        ("launch_preflight", _)
            |
        (
            "omp_ready",
            "report_absent_after_interactive_ready"
                | "report_absent_without_interactive_ready"
                | "original_agent_absent"
        ) | ("shell_probe", _)
    )
}
#[derive(Clone, Copy)]
pub(crate) struct OmpOfflineFailure {
    phase: &'static str,
    guard: &'static str,
    code: Option<ErrorCode>,
    io: &'static str,
    frame: &'static str,
}
impl OmpOfflineFailure {
    pub(crate) const fn new(
        phase: &'static str,
        guard: &'static str,
        code: ErrorCode,
        io: &'static str,
    ) -> Self {
        Self {
            phase,
            guard,
            code: Some(code),
            io,
            frame: "none",
        }
    }
}
struct OmpCreationState {
    mode: OmpReporterMode,
    lines: u8,
    progress_lines: u8,
    backend_launch_emitted: bool,
    backend_readiness_emitted: bool,
    coarse_shell_settled: bool,
    seen: HashSet<(&'static str, &'static str)>,
    agent_start_seen: bool,
    launch_preflight_seen: bool,
    original: Option<OmpOfflineOriginal>,
    invalidated: bool,
    ready: bool,
    pending: Option<OmpOfflineFailure>,
    transport_connection: Option<Uuid>,
    transport_emitted: bool,
    creation_terminal_emitted: bool,
    creation_nonterminal_lines: u8,
    creation_topology_not_ready_pending: bool,
    creation_original_agent_present: Option<bool>,
    creation_native_report_absent: bool,
    creation_interactive_ready_seen: bool,
}
impl OmpCreationReporter {
    pub(crate) fn new() -> Self {
        Self::with_mode(OmpReporterMode::Creation)
    }
    pub(crate) fn new_offline() -> Self {
        Self::with_mode(OmpReporterMode::Offline)
    }
    /// Backend-only capture uses the installed Herdr. Suppress transport progress so the
    /// first native launch disposition and readiness rejection retain the finite budget.
    pub(crate) fn new_backend() -> Self {
        Self::with_mode(OmpReporterMode::Backend)
    }
    fn with_mode(mode: OmpReporterMode) -> Self {
        Self(Arc::new(Mutex::new(OmpCreationState {
            mode,
            lines: 0,
            progress_lines: 0,
            backend_launch_emitted: false,
            backend_readiness_emitted: false,
            coarse_shell_settled: false,
            seen: HashSet::new(),
            agent_start_seen: false,
            launch_preflight_seen: false,
            original: None,
            invalidated: false,
            ready: false,
            pending: None,
            transport_connection: None,
            transport_emitted: false,
            creation_terminal_emitted: false,
            creation_nonterminal_lines: 0,
            creation_topology_not_ready_pending: false,
            creation_original_agent_present: None,
            creation_native_report_absent: false,
            creation_interactive_ready_seen: false,
        })))
    }
    fn emit(
        state: &mut OmpCreationState,
        phase: &'static str,
        guard: &'static str,
        code: Option<ErrorCode>,
        io: &'static str,
        ack: Option<(bool, bool, bool, bool, bool, bool)>,
    ) {
        Self::emit_with_frame(
            state,
            phase,
            guard,
            ClosedReport {
                code,
                io,
                frame: "none",
                probe_deadline:None,
            },
            ack,
        );
    }
    fn emit_with_frame(
        state: &mut OmpCreationState,
        phase: &'static str,
        guard: &'static str,
        metadata: ClosedReport,
        ack: Option<(bool, bool, bool, bool, bool, bool)>,
    ) {
        let progress = matches!(
            (phase, guard),
            ("herdr_launch", "ack_matched")
                | ("transport", "dispatch_returned")
                | ("transport", "response_queued")
                | ("transport", "output_written_after_queue")
        );
        if matches!(
            state.mode,
            OmpReporterMode::Creation | OmpReporterMode::Backend
        ) && transport_progress(phase, guard)
        {
            return;
        }
        let coarse_shell = phase == "coarse_shell";
        if coarse_shell && state.mode != OmpReporterMode::Backend {
            return;
        }
        let creation_terminal = creation_terminal_readiness(phase, guard);
        if state.mode == OmpReporterMode::Creation {
            if creation_transient_native_report(phase, guard) {
                return;
            }
            if (creation_terminal && state.creation_terminal_emitted)
                || (!creation_terminal && state.creation_nonterminal_lines >= 2)
            {
                return;
            }
        }
        let backend_priority = backend_priority(phase, guard);
        if state.mode == OmpReporterMode::Backend
            && !coarse_shell
            && !(phase == "shell_probe" && metadata.code.is_some())
            && !backend_report_admits(state, backend_priority)
        {
            return;
        }
        // Keep room for the first native admission or connection failure even if normal
        // launch/output observations arrive before it.
        let reserved = u8::from(!state.backend_launch_emitted) + u8::from(!state.backend_readiness_emitted);
        if state.lines >= 8
            || (coarse_shell && state.mode == OmpReporterMode::Backend && state.lines.saturating_add(reserved) >= 8)
            || (progress && state.progress_lines >= 3)
            || !state.seen.insert((phase, guard))
        {
            return;
        }
        state.lines += 1;
        if state.mode == OmpReporterMode::Backend {
            match backend_priority {
                Some(BackendPriority::Launch) => state.backend_launch_emitted = true,
                Some(BackendPriority::Readiness) => state.backend_readiness_emitted = true,
                None => {}
            }
        }
        if state.mode == OmpReporterMode::Creation {
            if creation_terminal {
                state.creation_terminal_emitted = true;
            } else {
                state.creation_nonterminal_lines += 1;
            }
        }
        if progress {
            state.progress_lines += 1;
        }
        let code = metadata
            .code
            .map_or("none".to_owned(), |code| format!("{code:?}"));
        // Retiring a diagnostic receiver must not change the native action's outcome.
        use std::io::Write as _;
        if let Some((name, kind, pane, terminal, workspace, argv)) = ack {
            let _ = writeln!(
                std::io::stderr().lock(),
                "goatr_omp_first_guard phase={phase} guard={guard} code={code} io={} frame={} name={} kind={} pane={} terminal={} workspace={} argv={}",
                metadata.io,
                metadata.frame,
                u8::from(name),
                u8::from(kind),
                u8::from(pane),
                u8::from(terminal),
                u8::from(workspace),
                u8::from(argv),
            );
        } else if let Some((site,observation,budget))=metadata.probe_deadline {
            let _=writeln!(std::io::stderr().lock(),
                "goatr_omp_first_guard phase={phase} guard={guard} code={code} io={} frame={} deadlineSite={site} observation={} entryBudget={budget}",
                metadata.io,metadata.frame,observation.as_str());
        } else {
            let _ = writeln!(
                std::io::stderr().lock(),
                "goatr_omp_first_guard phase={phase} guard={guard} code={code} io={} frame={}",
                metadata.io, metadata.frame
            );
        }
    }
    /// One Backend-only settlement from the unchanged startup traversal. Creation,
    /// Offline and ordinary runs retain their existing capture budgets and stay silent.
    fn report_coarse_shell(&self, guard: CoarseShellGuard, code: Option<ErrorCode>, io: &'static str) {
        let mut state = self.0.lock().expect("OMP first-guard reporter mutex");
        if state.mode != OmpReporterMode::Backend || state.coarse_shell_settled
        {
            return;
        }
        state.coarse_shell_settled = true;
        Self::emit(&mut state, "coarse_shell", guard.name(), code, io, None);
    }

    /// Buffer only the retrying source-not-ready observation. Other topology guards remain
    /// immediate because they can return before the failed-settlement summary is reached.
    pub(crate) fn defer_creation_topology_not_ready(&self) {
        let mut state = self.0.lock().expect("OMP first-guard reporter mutex");
        match state.mode {
            OmpReporterMode::Creation => state.creation_topology_not_ready_pending = true,
            OmpReporterMode::Backend => Self::emit(
                &mut state,
                "omp_ready",
                "topology_not_ready",
                Some(ErrorCode::NotReady),
                "none",
                None,
            ),
            OmpReporterMode::Offline => {}
        }
    }

    /// Retain only closed state from the existing selected creation snapshot. This is
    /// diagnostic evidence and never participates in admission or authority decisions.
    pub(crate) fn creation_agent_observation(&self, agent: Option<&Agent>) {
        let mut state = self.0.lock().expect("OMP first-guard reporter mutex");
        if state.mode != OmpReporterMode::Creation {
            return;
        }
        let Some(agent) = agent else {
            state.creation_original_agent_present = Some(false);
            state.creation_native_report_absent = false;
            return;
        };
        state.creation_original_agent_present = Some(true);
        state.creation_interactive_ready_seen |= agent.interactive_ready;
        state.creation_native_report_absent = agent.agent_session.is_none();
    }

    /// Emit the reserved terminal observation at the existing failed readiness settlement,
    /// before receipt projection discards the selected native snapshot.
    pub(crate) fn creation_readiness_failed(&self) {
        let mut state = self.0.lock().expect("OMP first-guard reporter mutex");
        if state.mode != OmpReporterMode::Creation || state.creation_terminal_emitted {
            return;
        }
        if state.creation_topology_not_ready_pending && state.creation_nonterminal_lines < 2 {
            Self::emit(
                &mut state,
                "omp_ready",
                "topology_not_ready",
                Some(ErrorCode::NotReady),
                "none",
                None,
            );
        }
        let guard = match (
            state.creation_original_agent_present,
            state.creation_native_report_absent,
            state.creation_interactive_ready_seen,
        ) {
            (Some(false), _, _) => "original_agent_absent",
            (Some(true), true, true) => "report_absent_after_interactive_ready",
            (Some(true), true, false) => "report_absent_without_interactive_ready",
            _ => return,
        };
        Self::emit(
            &mut state,
            "omp_ready",
            guard,
            Some(ErrorCode::NotReady),
            "none",
            None,
        );
    }

    pub(crate) fn report(&self, phase: &'static str, guard: &'static str, code: Option<ErrorCode>) {
        let mut state = self.0.lock().expect("OMP first-guard reporter mutex");
        if matches!(
            state.mode,
            OmpReporterMode::Creation | OmpReporterMode::Backend
        ) {
            Self::emit(&mut state, phase, guard, code, "none", None);
        }
    }
    /// Reports the first existing pre-intent launch failure without another observation.
    pub(crate) fn report_launch_preflight(&self, guard: &'static str, code: ErrorCode, io: &'static str) {
        let mut state = self.0.lock().expect("OMP first-guard reporter mutex");
        if !matches!(state.mode, OmpReporterMode::Creation | OmpReporterMode::Backend)
            || state.launch_preflight_seen
        {
            return;
        }
        state.launch_preflight_seen = true;
        Self::emit(&mut state, "launch_preflight", guard, Some(code), io, None);
    }
    /// Shell-probe capture observes only the existing pre-intent return boundary. The caller
    /// supplies static guards and already-observed error categories; it never changes admission.
    pub(crate) fn report_shell_probe(
        &self,
        guard: &'static str,
        code: ErrorCode,
        io: &'static str,
    ) {
        let mut state = self.0.lock().expect("OMP first-guard reporter mutex");
        if matches!(
            state.mode,
            OmpReporterMode::Creation | OmpReporterMode::Backend
        ) {
            Self::emit(&mut state, "shell_probe", guard, Some(code), io, None);
        }
    }
    fn backend_capture(&self)->bool {
        self.0.lock().expect("OMP first-guard reporter mutex").mode==OmpReporterMode::Backend
    }
    fn report_probe_deadline(&self,site:&'static str,detail:Option<(ProbeObservation,&'static str)>) {
        let Some((observation,budget))=detail else {
            self.report_shell_probe("result_deadline",ErrorCode::Timeout,"none");return;
        };
        let mut state=self.0.lock().expect("OMP first-guard reporter mutex");
        if state.mode==OmpReporterMode::Backend {
            Self::emit_with_frame(&mut state,"shell_probe","result_deadline",
                ClosedReport{code:Some(ErrorCode::Timeout),io:"none",frame:"none",probe_deadline:Some((site,observation,budget))},None);
        }
    }
    pub(crate) fn report_agent(&self, guard: &'static str, code: Option<ErrorCode>) {
        let mut state = self.0.lock().expect("OMP first-guard reporter mutex");
        if !matches!(
            state.mode,
            OmpReporterMode::Creation | OmpReporterMode::Backend
        ) || (state.mode == OmpReporterMode::Creation && state.agent_start_seen)
            || (state.mode == OmpReporterMode::Backend
                && state.agent_start_seen
                && !(state.backend_launch_emitted && state.backend_readiness_emitted))
        {
            return;
        }
        state.agent_start_seen = true;
        Self::emit(&mut state, "herdr_launch", guard, code, "none", None);
    }
    pub(crate) fn report_agent_io(&self, guard: &'static str, error: &io::Error) {
        let mut state = self.0.lock().expect("OMP first-guard reporter mutex");
        if !matches!(
            state.mode,
            OmpReporterMode::Creation | OmpReporterMode::Backend
        ) || (state.mode == OmpReporterMode::Creation && state.agent_start_seen)
            || (state.mode == OmpReporterMode::Backend
                && state.agent_start_seen
                && !(state.backend_launch_emitted && state.backend_readiness_emitted))
        {
            return;
        }
        state.agent_start_seen = true;
        let io = match error.kind() {
            io::ErrorKind::TimedOut => "timed_out",
            io::ErrorKind::UnexpectedEof => "unexpected_eof",
            io::ErrorKind::ConnectionReset => "connection_reset",
            io::ErrorKind::ConnectionAborted => "connection_aborted",
            io::ErrorKind::BrokenPipe => "broken_pipe",
            io::ErrorKind::NotConnected => "not_connected",
            io::ErrorKind::PermissionDenied => "permission_denied",
            io::ErrorKind::InvalidData => "invalid_data",
            _ => "other",
        };
        Self::emit(&mut state, "herdr_launch", guard, None, io, None);
    }
    pub(crate) fn report_agent_ack(
        &self,
        guard: &'static str,
        fields: (bool, bool, bool, bool, bool, bool),
    ) {
        let mut state = self.0.lock().expect("OMP first-guard reporter mutex");
        if !matches!(
            state.mode,
            OmpReporterMode::Creation | OmpReporterMode::Backend
        ) || (state.mode == OmpReporterMode::Creation && state.agent_start_seen)
            || (state.mode == OmpReporterMode::Backend
                && state.agent_start_seen
                && !(state.backend_launch_emitted && state.backend_readiness_emitted))
        {
            return;
        }
        state.agent_start_seen = true;
        Self::emit(
            &mut state,
            "herdr_launch",
            guard,
            None,
            "none",
            Some(fields),
        );
    }
    /// Bind the diagnostic to the first authenticated OMP creation connection.
    /// The identity stays private and later connections cannot retarget it.
    pub(crate) fn arm_transport(&self, connection: &Uuid) {
        let Ok(mut state) = self.0.lock() else {
            return;
        };
        if state.transport_connection.is_none() {
            state.transport_connection = Some(connection.clone());
        }
    }
    /// Emit one closed server-side initiating condition for that pinned connection.
    pub(crate) fn transport_failure(
        &self,
        connection: &Uuid,
        guard: &'static str,
        code: Option<ErrorCode>,
        io: &'static str,
        frame: &'static str,
    ) {
        let Ok(mut state) = self.0.lock() else {
            return;
        };
        if state.transport_emitted || state.transport_connection.as_ref() != Some(connection) {
            return;
        }
        match state.mode {
            OmpReporterMode::Creation => {
                state.transport_emitted = true;
                Self::emit_with_frame(
                    &mut state,
                    "transport",
                    guard,
                    ClosedReport { code, io, frame, probe_deadline:None },
                    None,
                );
            }
            OmpReporterMode::Backend => {}
            OmpReporterMode::Offline if !state.invalidated && state.original.is_some() => {
                state.transport_emitted = true;
                Self::record_offline(
                    &mut state,
                    OmpOfflineFailure {
                        phase: "transport",
                        guard,
                        code,
                        io,
                        frame,
                    },
                );
            }
            OmpReporterMode::Offline => {}
        }
    }
    /// Freeze the first installation candidate privately. Confirmation requires
    /// successful Ready installation; later observations cannot retarget it.
    pub(crate) fn prepare_offline(
        &self,
        target: &Target,
        epoch: &Uuid,
        source: &Source,
        terminal_id: &Id,
    ) {
        let Ok(mut state) = self.0.lock() else {
            return;
        };
        if state.mode == OmpReporterMode::Offline && state.original.is_none() {
            state.original = Some(OmpOfflineOriginal {
                target: target.clone(),
                epoch: epoch.clone(),
                server_id: source.server_id.clone(),
                incarnation: source.incarnation.clone(),
                evidence: source.evidence.clone(),
                terminal_id: terminal_id.clone(),
            });
        }
    }
    // HostChanged can be published during installation. Retain a matching loss
    // privately until installation confirms that this original binding was Ready.
    fn confirm_offline(&self, target: &Target, epoch: &Uuid) {
        let Ok(mut state) = self.0.lock() else {
            return;
        };
        if state.mode == OmpReporterMode::Offline
            && state
                .original
                .as_ref()
                .is_some_and(|original| original.target == *target && original.epoch == *epoch)
        {
            state.ready = true;
            if let Some(failure) = state.pending.take() {
                Self::emit_with_frame(
                    &mut state,
                    failure.phase,
                    failure.guard,
                    ClosedReport {
                        code: failure.code,
                        io: failure.io,
                        frame: failure.frame,
                        probe_deadline:None,
                    },
                    None,
                );
            }
        }
    }
    fn record_offline(state: &mut OmpCreationState, failure: OmpOfflineFailure) {
        state.invalidated = true;
        if state.ready {
            Self::emit_with_frame(
                state,
                failure.phase,
                failure.guard,
                ClosedReport {
                    code: failure.code,
                    io: failure.io,
                    frame: failure.frame,
                    probe_deadline:None,
                },
                None,
            );
        } else {
            state.pending = Some(failure);
        }
    }
    fn offline_server(&self, server: &Uuid, evidence: &ServerEvidence, failure: OmpOfflineFailure) {
        let Ok(mut state) = self.0.lock() else {
            return;
        };
        if state.mode == OmpReporterMode::Offline
            && !state.invalidated
            && state.original.as_ref().is_some_and(|original| {
                original.server_id == *server && original.evidence == *evidence
            })
        {
            Self::record_offline(&mut state, failure);
        }
    }
    fn matches_original(
        original: &OmpOfflineOriginal,
        target: &Target,
        epoch: &Uuid,
        source: &Source,
        terminal_id: &Id,
    ) -> bool {
        original.target == *target
            && original.epoch == *epoch
            && original.server_id == source.server_id
            && original.incarnation == source.incarnation
            && original.evidence == source.evidence
            && original.terminal_id == *terminal_id
    }
    fn matches_binding(
        original: &OmpOfflineOriginal,
        target: &Target,
        epoch: &Uuid,
        key: &StableSessionKey,
    ) -> bool {
        original.target == *target
            && original.epoch == *epoch
            && original.server_id == key.server_id
            && original.incarnation == key.server_incarnation
            && original.terminal_id == key.terminal_id
    }
    pub(crate) fn offline_source(
        &self,
        target: &Target,
        epoch: &Uuid,
        source: &Source,
        terminal_id: &Id,
        failure: OmpOfflineFailure,
    ) {
        let Ok(mut state) = self.0.lock() else {
            return;
        };
        if state.mode == OmpReporterMode::Offline
            && !state.invalidated
            && state.original.as_ref().is_some_and(|original| {
                Self::matches_original(original, target, epoch, source, terminal_id)
            })
        {
            Self::record_offline(&mut state, failure);
        }
    }
    pub(crate) fn offline_binding(
        &self,
        target: &Target,
        epoch: &Uuid,
        key: &StableSessionKey,
        failure: OmpOfflineFailure,
    ) {
        let Ok(mut state) = self.0.lock() else {
            return;
        };
        if state.mode == OmpReporterMode::Offline
            && !state.invalidated
            && state
                .original
                .as_ref()
                .is_some_and(|original| Self::matches_binding(original, target, epoch, key))
        {
            Self::record_offline(&mut state, failure);
        }
    }
}
fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn policy(error: io::Error) -> ErrorCode {
    if error.kind() == io::ErrorKind::OutOfMemory {
        ErrorCode::ResourceLimit
    } else {
        ErrorCode::NotReady
    }
}
pub(crate) fn codex_invalidation_io(error: &io::Error) -> (&'static str, i32) {
    let io = match error.kind() {
        io::ErrorKind::NotFound => "not_found",
        io::ErrorKind::PermissionDenied => "permission_denied",
        io::ErrorKind::ConnectionRefused => "connection_refused",
        io::ErrorKind::ConnectionReset => "connection_reset",
        io::ErrorKind::ConnectionAborted => "connection_aborted",
        io::ErrorKind::NotConnected => "not_connected",
        io::ErrorKind::BrokenPipe => "broken_pipe",
        io::ErrorKind::TimedOut => "timed_out",
        io::ErrorKind::UnexpectedEof => "unexpected_eof",
        io::ErrorKind::InvalidData => "invalid_data",
        _ => "other",
    };
    let errno = error
        .raw_os_error()
        .filter(|value| (-4095..=4095).contains(value))
        .unwrap_or(0);
    (io, errno)
}
fn codex_shell_probe_guard(guard: &'static str) -> bool {
    matches!(
        guard,
        "query_effect_dropped"
            | "query_rejected_known"
            | "query_rejected_other"
            | "query_source_changed"
            | "query_unavailable"
            | "result_deadline"
            | "result_authorization"
            | "result_shell_changed"
            | "result_read_failed"
            | "result_invalid"
            | "final_source"
            | "final_root"
            | "final_root_server_id"
            | "final_root_incarnation"
            | "final_root_evidence"
            | "final_root_socket_path"
            | "final_root_invalidation"
            | "workspace_confirm_connect"
            | "workspace_confirm_evidence_epoch"
            | "workspace_confirm_snapshot"
            | "workspace_confirm_pane"
            | "workspace_confirm_created_workspace"
            | "workspace_confirm_terminal"
            | "workspace_confirm_focus"
            | "final_source_changed"
            | "final_connect"
            | "final_identity"
            | "final_process_info"
            | "final_shell_changed"
            | "final_shell_reporter"
            | "final_shell_absent"
            | "final_shell_not_idle"
            | "final_shell_process"
            | "final_shell_executable"
            | "final_shell_reporter_recheck"
            | "final_result_view"
            | "final_deadline"
            | "final_authorization"
            | "result_rejected"
            | "result_rejected_kind"
            | "result_rejected_path"
            | "result_unsupported"
    )
}
/// Explicitly enabled, bounded diagnostic for the first original Codex binding only.
/// It emits closed classifications and never exposes a target, native report, or error text.
#[derive(Clone)]
pub(crate) struct CodexInvalidationReporter(Arc<Mutex<CodexInvalidationState>>);
/// Closed evidence for the one original `thread/start` callback used by an
/// explicitly enabled diagnostic. It carries no native identifiers or payloads.
#[derive(Clone)]
pub(crate) struct CodexThreadStartObservation(Arc<CodexThreadStartWindow>);
struct CodexThreadStartWindow {
    active: AtomicBool,
    state: Mutex<CodexThreadStartState>,
}
#[derive(Clone, Copy)]
enum CodexThreadStartMilestone {
    Unseen,
    Matched,
    Delivered,
    ReceiverClosed,
}
impl CodexThreadStartMilestone {
    const fn name(self) -> &'static str {
        match self {
            Self::Unseen => "unseen",
            Self::Matched => "matched",
            Self::Delivered => "delivered",
            Self::ReceiverClosed => "receiver_closed",
        }
    }
}
#[derive(Clone, Copy)]
enum CodexStartSendElapsed {
    Unobserved,
    LtQuarter,
    QuarterToHalf,
    HalfToThreeQuarters,
    ThreeQuartersToBudget,
    AtOrOverBudget,
}
impl CodexStartSendElapsed {
    const fn name(self) -> &'static str {
        match self {
            Self::Unobserved => "unobserved",
            Self::LtQuarter => "lt_quarter",
            Self::QuarterToHalf => "quarter_to_half",
            Self::HalfToThreeQuarters => "half_to_three_quarters",
            Self::ThreeQuartersToBudget => "three_quarters_to_budget",
            Self::AtOrOverBudget => "at_or_over_budget",
        }
    }
}
#[derive(Clone, Copy)]
pub(crate) struct CodexThreadStartSnapshot {
    milestone: CodexThreadStartMilestone,
    decoded_frame: bool,
    unmatched_response: bool,
    error: Option<ErrorCode>,
    start_send_elapsed: CodexStartSendElapsed,
    complete_data_frame: bool,
    methods_before: u8,
    methods_after: u8,
}
struct CodexThreadStartState {
    closed: bool,
    snapshot: CodexThreadStartSnapshot,
}
impl CodexThreadStartObservation {
    fn new() -> Self {
        Self(Arc::new(CodexThreadStartWindow {
            active: AtomicBool::new(false),
            state: Mutex::new(CodexThreadStartState {
                closed: false,
                snapshot: CodexThreadStartSnapshot {
                    milestone: CodexThreadStartMilestone::Unseen,
                    decoded_frame: false,
                    unmatched_response: false,
                    error: None,
                    start_send_elapsed: CodexStartSendElapsed::Unobserved,
                    complete_data_frame: false,
                    methods_before: 0,
                    methods_after: 0,
                },
            }),
        }))
    }
    /// Called once, under the existing callback lock after original registration.
    pub(crate) fn activate(&self) {
        let state = self.0.state.lock().unwrap_or_else(|error| error.into_inner());
        if !state.closed {
            self.0.active.store(true, Ordering::Release);
        }
    }
    pub(crate) fn decoded_frame(&self) {
        if !self.0.active.load(Ordering::Acquire) {
            return;
        }
        let mut state = self.0.state.lock().unwrap_or_else(|error| error.into_inner());
        if !state.closed && self.0.active.load(Ordering::Acquire) {
            // Decoding proves a complete data frame is available in this window,
            // even if registration raced with the reader's earlier socket yield.
            state.snapshot.complete_data_frame = true;
            state.snapshot.decoded_frame = true;
        }
    }
    pub(crate) fn complete_data_frame(&self) {
        if !self.0.active.load(Ordering::Acquire) {
            return;
        }
        let mut state = self.0.state.lock().unwrap_or_else(|error| error.into_inner());
        if !state.closed && self.0.active.load(Ordering::Acquire) {
            state.snapshot.complete_data_frame = true;
        }
    }
    pub(crate) fn start_send_elapsed(&self, elapsed: Duration) {
        let mut state = self.0.state.lock().unwrap_or_else(|error| error.into_inner());
        // A fast callback can close frame observation before the send returns.
        // The elapsed bucket may still be recorded, until the snapshot is frozen.
        if !state.closed && matches!(state.snapshot.start_send_elapsed, CodexStartSendElapsed::Unobserved) {
            let budget = pairing::BOUNDARY_TIMEOUT;
            state.snapshot.start_send_elapsed = if elapsed < budget / 4 {
                CodexStartSendElapsed::LtQuarter
            } else if elapsed < budget / 2 {
                CodexStartSendElapsed::QuarterToHalf
            } else if elapsed < budget * 3 / 4 {
                CodexStartSendElapsed::HalfToThreeQuarters
            } else if elapsed < budget {
                CodexStartSendElapsed::ThreeQuartersToBudget
            } else {
                CodexStartSendElapsed::AtOrOverBudget
            };
        }
    }
    /// Records only a closed family for a decoded method-bearing frame while
    /// the original callback remains active. The before/after division is the
    /// observation order of the original send future's completion marker.
    pub(crate) fn method(&self, method: &str, has_id: bool) {
        let mut state = self.0.state.lock().unwrap_or_else(|error| error.into_inner());
        if !state.closed && self.0.active.load(Ordering::Acquire) {
            let family = match (has_id, method) {
                (false, "configWarning") => 1 << 0,
                (false, "mcpServer/startupStatus/updated") => 1 << 1,
                (false, "thread/started") => 1 << 2,
                (true, "account/chatgptAuthTokens/refresh") => 1 << 3,
                (true, "attestation/generate") => 1 << 4,
                (true, "mcpServer/elicitation/request") => 1 << 5,
                (false, _) => 1 << 6,
                (true, _) => 1 << 7,
            };
            state.snapshot.complete_data_frame = true;
            state.snapshot.decoded_frame = true;
            if matches!(state.snapshot.start_send_elapsed, CodexStartSendElapsed::Unobserved) {
                state.snapshot.methods_before |= family;
            } else {
                state.snapshot.methods_after |= family;
            }
        }
    }
    pub(crate) fn unmatched_response(&self) {
        if !self.0.active.load(Ordering::Acquire) {
            return;
        }
        let mut state = self.0.state.lock().unwrap_or_else(|error| error.into_inner());
        if !state.closed && self.0.active.load(Ordering::Acquire) {
            state.snapshot.complete_data_frame = true;
            state.snapshot.decoded_frame = true;
            state.snapshot.unmatched_response = true;
        }
    }
    pub(crate) fn matched(&self, error: Option<ErrorCode>) {
        let mut state = self.0.state.lock().unwrap_or_else(|error| error.into_inner());
        // Removing the original callback ends its frame-observation window.
        self.0.active.store(false, Ordering::Release);
        if !state.closed {
            state.snapshot.complete_data_frame = true;
            state.snapshot.decoded_frame = true;
            state.snapshot.milestone = CodexThreadStartMilestone::Matched;
            state.snapshot.error = error;
        }
    }
    pub(crate) fn delivered(&self, delivered: bool) {
        let mut state = self.0.state.lock().unwrap_or_else(|error| error.into_inner());
        if !state.closed {
            state.snapshot.milestone = if delivered {
                CodexThreadStartMilestone::Delivered
            } else {
                CodexThreadStartMilestone::ReceiverClosed
            };
        }
    }
    /// Freezes exactly once. Late callbacks and frames cannot rewrite this view.
    pub(crate) fn freeze(&self) -> CodexThreadStartSnapshot {
        let mut state = self.0.state.lock().unwrap_or_else(|error| error.into_inner());
        self.0.active.store(false, Ordering::Release);
        state.closed = true;
        state.snapshot
    }
}
struct CodexInvalidationState {
    original: Option<(Target, Uuid)>,
    original_server: Option<(Uuid, ServerEvidence)>,
    prebinding_reported: bool,
    invalidated: bool,
    transport_connection: Option<Uuid>,
    transport_reported: bool,
    lines: u8,
    owner_progress_action: Option<Uuid>,
    owner_progress_seen: u8,
}
impl CodexInvalidationReporter {
    pub(crate) fn new() -> Self {
        Self(Arc::new(Mutex::new(CodexInvalidationState {
            original: None,
            original_server: None,
            prebinding_reported: false,
            invalidated: false,
            transport_connection: None,
            transport_reported: false,
            lines: 0,
            owner_progress_action: None,
            owner_progress_seen: 0,
        })))
    }
    pub(crate) fn arm_owner_progress(&self, action: &Uuid) {
        let Ok(mut state) = self.0.lock() else { return };
        if state.owner_progress_action.is_none() && state.original.is_none()
            && !state.prebinding_reported && !state.invalidated && !state.transport_reported
        {
            state.owner_progress_action = Some(action.clone());
        }
    }
    pub(crate) fn owner_progress(&self, action: &Uuid, guard: &'static str) {
        let bit = match guard {
            "owner_start_settled" => 1,
            "owner_child_status_returned" => 2,
            "owner_native_initialize_entered" => 4,
            _ => return,
        };
        let Ok(mut state) = self.0.lock() else { return };
        if state.owner_progress_action.as_ref() != Some(action)
            || state.original.is_some() || state.prebinding_reported || state.invalidated
            || state.transport_reported || state.owner_progress_seen & bit != 0
            || state.lines >= 3
        {
            return;
        }
        // Progress shares the finite budget while reserving five failure slots.
        state.owner_progress_seen |= bit;
        state.lines += 1;
        eprintln!(
            "goatr_codex_first_guard phase=owner_progress guard={guard} code=none io=none errno=0"
        );
    }
    /// Attaches only to the existing original thread/start callback.
    pub(crate) fn thread_start_observation(&self) -> CodexThreadStartObservation {
        CodexThreadStartObservation::new()
    }
    /// Emits the existing first thread-effect-drop line with its closed callback
    /// observation, provided no earlier prebinding cause has won the latch.
    pub(crate) fn thread_start_effect_drop(&self, snapshot: CodexThreadStartSnapshot) {
        let Ok(mut state) = self.0.lock() else { return };
        if state.original.is_some() || state.prebinding_reported || state.lines >= 8 {
            return;
        }
        state.prebinding_reported = true;
        state.lines += 1;
        let milestone = snapshot.milestone.name();
        let result = snapshot.error.map_or("none".to_owned(), |result| format!("{result:?}"));
        eprintln!(
            "goatr_codex_first_guard phase=thread guard=thread_effect_drop_start_reply code=NotReady io=none errno=0 start={milestone} decoded={} unmatched={} result={result} elapsed={} data={} before={} after={}",
            u8::from(snapshot.decoded_frame),
            u8::from(snapshot.unmatched_response),
            snapshot.start_send_elapsed.name(),
            u8::from(snapshot.complete_data_frame),
            snapshot.methods_before,
            snapshot.methods_after,
        );
    }
    pub(crate) fn arm(
        &self,
        target: &Target,
        epoch: &Uuid,
        server: &Uuid,
        evidence: &ServerEvidence,
    ) {
        let Ok(mut state) = self.0.lock() else { return };
        if state.original.is_none() {
            state.original = Some((target.clone(), epoch.clone()));
            state.original_server = Some((server.clone(), evidence.clone()));
        }
    }
    fn emit(
        state: &mut CodexInvalidationState,
        phase: &'static str,
        guard: &'static str,
        code: ErrorCode,
        io: &'static str,
        errno: i32,
        report: Option<(bool, bool, bool, bool, bool)>,
    ) {
        if state.lines >= 8 {
            return;
        }
        state.lines += 1;
        match report {
            Some((before_present, after_present, agent_equal, kind_equal, equal)) => eprintln!(
                "goatr_codex_first_guard phase={phase} guard={guard} code={code:?} io={io} errno={errno} before_present={} after_present={} agent_equal={} kind_equal={} equal={}",
                u8::from(before_present),
                u8::from(after_present),
                u8::from(agent_equal),
                u8::from(kind_equal),
                u8::from(equal)
            ),
            None => eprintln!(
                "goatr_codex_first_guard phase={phase} guard={guard} code={code:?} io={io} errno={errno}"
            ),
        }
    }
    pub(crate) fn prebinding(
        &self,
        phase: &'static str,
        guard: &'static str,
        code: ErrorCode,
        io: &'static str,
    ) {
        let Ok(mut state) = self.0.lock() else { return };
        if state.original.is_none() && !state.prebinding_reported {
            state.prebinding_reported = true;
            Self::emit(&mut state, phase, guard, code, io, 0, None);
        }
    }
    fn shell_probe_prebinding(
        &self,
        guard: &'static str,
        code: ErrorCode,
        io: &'static str,
    ) {
        if !codex_shell_probe_guard(guard) {
            return;
        }
        self.prebinding("shell_probe", guard, code, io);
    }
    fn shell_probe_prebinding_error(
        &self,
        guard: &'static str,
        code: ErrorCode,
        error: &io::Error,
    ) {
        if codex_shell_probe_guard(guard) {
            self.prebinding_error("shell_probe", guard, code, error);
        }
    }
    pub(crate) fn prebinding_error(
        &self,
        phase: &'static str,
        guard: &'static str,
        code: ErrorCode,
        error: &io::Error,
    ) {
        let (io, errno) = codex_invalidation_io(error);
        let Ok(mut state) = self.0.lock() else { return };
        if state.original.is_none() && !state.prebinding_reported {
            state.prebinding_reported = true;
            Self::emit(&mut state, phase, guard, code, io, errno, None);
        }
    }
    /// One closed result from the explicitly selected, pre-launch-pinned terminal read.
    /// It shares the original latch and cannot displace an earlier diagnostic failure.
    fn startup_terminal(
        &self,
        guard: &'static str,
        code: ErrorCode,
        io: &'static str,
        errno: i32,
    ) {
        let Ok(mut state) = self.0.lock() else { return };
        if state.original.is_none() && !state.prebinding_reported {
            state.prebinding_reported = true;
            Self::emit(&mut state, "herdr", guard, code, io, errno, None);
        }
    }
    /// The transport diagnostic is separately pinned to the first authenticated Codex
    /// create connection, before topology can install its resulting target.
    pub(crate) fn arm_transport(&self, connection: &Uuid) {
        let Ok(mut state) = self.0.lock() else { return };
        if state.original.is_none() && state.transport_connection.is_none() {
            state.transport_connection = Some(connection.clone());
        }
    }
    pub(crate) fn transport(
        &self,
        connection: &Uuid,
        guard: &'static str,
        code: Option<ErrorCode>,
        io: &'static str,
        errno: i32,
        frame: &'static str,
    ) {
        let Ok(mut state) = self.0.lock() else { return };
        if state.transport_connection.as_ref() != Some(connection) || state.transport_reported {
            return;
        }
        state.transport_reported = true;
        if state.lines >= 8 {
            return;
        }
        state.lines += 1;
        let code = code.map_or("none".to_owned(), |code| format!("{code:?}"));
        eprintln!(
            "goatr_codex_first_guard phase=transport guard={guard} code={code} io={io} errno={errno} frame={frame}"
        );
    }
    /// Main has no connection handle. It may report only after transport pinned the
    /// authenticated original create connection above.
    pub(crate) fn supervisor(
        &self,
        guard: &'static str,
        code: Option<ErrorCode>,
        io: &'static str,
        errno: i32,
    ) {
        let Ok(mut state) = self.0.lock() else { return };
        if state.transport_connection.is_none() || state.transport_reported {
            return;
        }
        state.transport_reported = true;
        if state.lines >= 8 {
            return;
        }
        state.lines += 1;
        let code = code.map_or("none".to_owned(), |code| format!("{code:?}"));
        eprintln!(
            "goatr_codex_first_guard phase=supervisor guard={guard} code={code} io={io} errno={errno}"
        );
    }
    pub(crate) fn supervisor_error(&self, guard: &'static str, error: &io::Error) {
        let (io, errno) = codex_invalidation_io(error);
        self.supervisor(guard, None, io, errno);
    }
    pub(crate) fn target(
        &self,
        target: &Target,
        epoch: &Uuid,
        phase: &'static str,
        guard: &'static str,
        code: ErrorCode,
        io: &'static str,
    ) {
        self.target_closed(target, epoch, phase, guard, code, (io, 0));
    }
    pub(crate) fn target_closed(
        &self,
        target: &Target,
        epoch: &Uuid,
        phase: &'static str,
        guard: &'static str,
        code: ErrorCode,
        detail: (&'static str, i32),
    ) {
        let (io, errno) = detail;
        let Ok(mut state) = self.0.lock() else { return };
        if state.original.as_ref() == Some(&(target.clone(), epoch.clone())) && !state.invalidated {
            state.invalidated = true;
            Self::emit(&mut state, phase, guard, code, io, errno, None);
        }
    }
    pub(crate) fn native_report(
        &self,
        target: &Target,
        epoch: &Uuid,
        phase: &'static str,
        before: &Option<(Option<String>, Option<NativeSession>)>,
        after: &Option<(Option<String>, Option<NativeSession>)>,
    ) {
        let Ok(mut state) = self.0.lock() else { return };
        if state.original.as_ref() != Some(&(target.clone(), epoch.clone())) || state.invalidated {
            return;
        }
        state.invalidated = true;
        fn session(
            report: &Option<(Option<String>, Option<NativeSession>)>,
        ) -> Option<&NativeSession> {
            report.as_ref().and_then(|(_, session)| session.as_ref())
        }
        let before_session = session(before);
        let after_session = session(after);
        Self::emit(
            &mut state,
            phase,
            "native_report_changed",
            ErrorCode::StaleSession,
            "none",
            0,
            Some((
                before_session.is_some(),
                after_session.is_some(),
                before.as_ref().map(|(agent, _)| agent) == after.as_ref().map(|(agent, _)| agent),
                before_session.map(|session| &session.kind)
                    == after_session.map(|session| &session.kind),
                before == after,
            )),
        );
    }
    /// The recovery task has no binding lookup when it receives an I/O failure. Its
    /// reporter context is pinned during installation, so a replacement server or
    /// evidence cannot claim the original diagnostic latch.
    pub(crate) fn subscription_error(
        &self,
        server: &Uuid,
        evidence: Option<&ServerEvidence>,
        guard: &'static str,
        error: &io::Error,
    ) {
        let (io, errno) = codex_invalidation_io(error);
        let code = if error.kind() == io::ErrorKind::OutOfMemory {
            ErrorCode::ResourceLimit
        } else {
            ErrorCode::NotReady
        };
        let Ok(mut state) = self.0.lock() else { return };
        let matches_original = state
            .original_server
            .as_ref()
            .is_some_and(|(expected, proof)| {
                expected == server && evidence.is_none_or(|observed| observed == proof)
            });
        if matches_original && !state.invalidated {
            state.invalidated = true;
            Self::emit(&mut state, "herdr", guard, code, io, errno, None);
        }
    }
    pub(crate) fn revalidation(&self, server: &Uuid, evidence: &ServerEvidence, code: ErrorCode) {
        let Ok(mut state) = self.0.lock() else { return };
        if state
            .original_server
            .as_ref()
            .is_some_and(|(expected, proof)| expected == server && proof == evidence)
            && !state.invalidated
        {
            state.invalidated = true;
            Self::emit(
                &mut state,
                "herdr",
                "effective_topology_revalidation",
                code,
                "none",
                0,
                None,
            );
        }
    }
}
fn limit() -> io::Error {
    io::Error::new(io::ErrorKind::OutOfMemory, "Herdr resource limit")
}
fn store_io(error: StorageError) -> io::Error {
    match error {
        StorageError::Policy(ErrorCode::ResourceLimit | ErrorCode::ActionCapacity) => limit(),
        StorageError::Policy(ErrorCode::Busy) => {
            io::Error::new(io::ErrorKind::WouldBlock, "SQLite owner queue is busy")
        }
        _ => invalid("Retained SQLite state is unavailable"),
    }
}
fn id_valid(id: &Id) -> io::Result<()> {
    id.validate()
        .map_err(|_| invalid("invalid Herdr identifier"))
}

// Minimal projections of Herdr's public response, not parallel Goatr wire models.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Workspace {
    pub workspace_id: Id,
    pub number: Uint,
    pub label: String,
    pub focused: bool,
    pub active_tab_id: String,
    pub agent_status: NativeAgentStatus,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Tab {
    pub tab_id: Id,
    pub workspace_id: Id,
    pub number: Uint,
    pub label: String,
    pub focused: bool,
    pub agent_status: NativeAgentStatus,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct NativeSession {
    pub source: String,
    pub agent: String,
    pub kind: String,
    pub value: String,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Pane {
    pub pane_id: Id,
    pub terminal_id: Id,
    pub workspace_id: Id,
    pub tab_id: Id,
    pub cwd: Option<String>,
    pub foreground_cwd: Option<String>,
    pub title: Option<String>,
    pub label: Option<String>,
    pub terminal_title_stripped: Option<String>,
    pub display_agent: Option<String>,
    pub restore_error: Option<String>,
    pub agent: Option<String>,
    pub agent_session: Option<NativeSession>,
    pub focused: bool,
    pub agent_status: NativeAgentStatus,
    #[serde(default)]
    pub read_identity: Option<PaneReadIdentity>,
    #[serde(default)]
    pub shell: Option<NativeShellInfo>,
    #[serde(default,deserialize_with="optional_artifact_shell_scope")]pub artifact_shell_scope:Option<artifacts::NativeArtifactScope>,
}
/// Host-local opaque native epoch values. They never enter the Goatr wire model.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct PaneReadIdentity {
    /// Opaque 16-byte native runtime identity, encoded as exactly 32 lowercase hex digits.
    pub herdr_incarnation: String,
    pub runtime_epoch: u64,
    pub binding_epoch: u64,
}
impl PaneReadIdentity {
    fn validate(&self) -> io::Result<()> {
        if self.herdr_incarnation.len() != 32
            || !self.herdr_incarnation.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(invalid("invalid native read incarnation"));
        }
        Ok(())
    }
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PaneReadBoundResult {
    pub pane_id: Id,
    pub terminal_id: Id,
    pub workspace_id: Id,
    pub tab_id: Id,
    pub read_identity: PaneReadIdentity,
    pub text: String,
    pub truncated: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct WorktreeReadScope {
    pub pane_id: Id,
    pub terminal_id: Id,
    pub workspace_id: Id,
    pub tab_id: Id,
    pub read_identity: PaneReadIdentity,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Agent {
    pub terminal_id: Id,
    pub workspace_id: Id,
    pub tab_id: Id,
    pub pane_id: Id,
    pub name: Option<String>,
    pub agent: Option<String>,
    pub title: Option<String>,
    pub agent_session: Option<NativeSession>,
    #[serde(default)]
    pub launch_pending: bool,
    #[serde(default)]
    pub interactive_ready: bool,
}
fn launch_ack_matches(
    agent: &Agent,
    argv_matches: bool,
    kind: &str,
    alias: &str,
    pane: &Id,
    terminal: &Id,
    workspace: Option<&Id>,
) -> bool {
    argv_matches
        && agent.pane_id == *pane
        && agent.terminal_id == *terminal
        && agent.name.as_deref() == Some(alias)
        && (agent.launch_pending || agent.agent.as_deref() == Some(kind))
        && workspace == Some(&agent.workspace_id)
}
fn agent_shell_capability<'de,D:serde::Deserializer<'de>>(deserializer:D)->Result<bool,D::Error>{let value=Value::deserialize(deserializer)?;Ok(value.as_bool().unwrap_or(false))}
fn optional_artifact_shell_scope<'de,D:serde::Deserializer<'de>>(deserializer:D)->Result<Option<artifacts::NativeArtifactScope>,D::Error>{let value=Value::deserialize(deserializer)?;Ok(serde_json::from_value::<artifacts::NativeArtifactScope>(value).ok().filter(|scope|scope.validate().is_ok()))}
fn agent_shell_epoch<'de,D:serde::Deserializer<'de>>(deserializer:D)->Result<Option<u64>,D::Error>{let value=Value::deserialize(deserializer)?;Ok(value.as_u64().filter(|epoch|*epoch>0))}
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct NativeLayout {
    pub workspace_id: Id,
    pub tab_id: Id,
    pub area: endpoint::Rect,
    pub focused_pane_id: Id,
    pub panes: Vec<NativeLayoutPane>,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct NativeLayoutPane {
    pub pane_id: Id,
    pub rect: endpoint::Rect,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Snapshot {
    // Read-only qualification for the explicitly private legacy smoke mode, never stock authority.
    #[serde(default)] pub pane_grid_subscribe_bound: bool,
    #[serde(default)] pub shell_grid_subscribe_bound: bool,
    #[serde(default,deserialize_with="agent_shell_capability")]pub artifact_read_bound:bool,
    #[serde(default,deserialize_with="agent_shell_epoch")]pub artifact_permission_epoch:Option<u64>,
    pub version: String,
    pub protocol: u32,
    #[serde(default)]
    pub shell_read_bound: bool,
    pub focused_workspace_id: Option<Id>,
    pub focused_tab_id: Option<Id>,
    pub focused_pane_id: Option<Id>,
    pub workspaces: Vec<Workspace>,
    pub tabs: Vec<Tab>,
    pub panes: Vec<Pane>,
    pub agents: Vec<Agent>,
    #[serde(default)]
    pub layouts: Vec<NativeLayout>,
}
impl Snapshot {
    pub fn validate(&self) -> io::Result<()> {
        if self.protocol != 22 {
            return Err(invalid("unsupported Herdr topology protocol"));
        }
        if self.panes.len() > 512
            || self.agents.len() > 512
            || self.workspaces.len() > 512
            || self.tabs.len() > 512
        {
            return Err(limit());
        }
        let mut workspaces = HashSet::new();
        for w in &self.workspaces {
            id_valid(&w.workspace_id)?;
            w.number
                .validate()
                .map_err(|_| invalid("invalid workspace number"))?;
            if !workspaces.insert(&w.workspace_id) {
                return Err(invalid("duplicate workspace"));
            }
        }
        let mut tabs = HashMap::new();
        for t in &self.tabs {
            id_valid(&t.tab_id)?;
            t.number
                .validate()
                .map_err(|_| invalid("invalid tab number"))?;
            if !workspaces.contains(&t.workspace_id)
                || tabs.insert(&t.tab_id, &t.workspace_id).is_some()
            {
                return Err(invalid("invalid tab routing"));
            }
        }
        for w in &self.workspaces {
            if !w.active_tab_id.is_empty() {
                let active =
                    Id::new(&w.active_tab_id).map_err(|_| invalid("invalid active tab"))?;
                if tabs.get(&active) != Some(&&w.workspace_id) {
                    return Err(invalid("active tab routing disagrees with workspace"));
                }
            } else if tabs.values().any(|workspace| **workspace == w.workspace_id) {
                return Err(invalid("active tab is unavailable in nonempty workspace"));
            }
        }
        let mut panes = HashMap::new();
        let mut terminals = HashSet::new();
        for p in &self.panes {
            id_valid(&p.pane_id)?;
            id_valid(&p.terminal_id)?;
            if let Some(identity) = &p.read_identity {
                identity.validate()?;
            }
            if tabs.get(&p.tab_id) != Some(&&p.workspace_id)
                || panes.insert(&p.pane_id, p).is_some()
                || !terminals.insert(&p.terminal_id)
            {
                return Err(invalid("invalid or duplicate pane/terminal routing"));
            }
        }
        let mut agents = HashSet::new();
        for a in &self.agents {
            let p = panes
                .get(&a.pane_id)
                .ok_or_else(|| invalid("agent pane absent"))?;
            if p.terminal_id != a.terminal_id
                || p.workspace_id != a.workspace_id
                || p.tab_id != a.tab_id
                || !agents.insert(&a.terminal_id)
            {
                return Err(invalid("agent routing disagrees with pane"));
            }
        }
        if self
            .focused_workspace_id
            .as_ref()
            .is_some_and(|id| !workspaces.contains(id))
        {
            return Err(invalid("focused workspace absent"));
        }
        if self
            .focused_tab_id
            .as_ref()
            .is_some_and(|id| !tabs.contains_key(id))
            || self
                .focused_pane_id
                .as_ref()
                .is_some_and(|id| !panes.contains_key(id))
        {
            return Err(invalid("focused tab/pane absent"));
        }
        Ok(())
    }
    pub fn pane(&self, id: &Id) -> io::Result<&Pane> {
        self.panes
            .iter()
            .find(|p| &p.pane_id == id)
            .ok_or_else(|| invalid("pane absent"))
    }
    /// Optional terminal geometry never gates structured agents or ordinary inventory.
    pub(crate) fn layout_seed(&self, selected:&Pane)->io::Result<(u16,u16)> {
        if self.layouts.len()>512 {return Err(limit());}
        let tabs:HashMap<_,_>=self.tabs.iter().map(|tab|(&tab.tab_id,&tab.workspace_id)).collect();
        let panes:HashMap<_,_>=self.panes.iter().map(|pane|(&pane.pane_id,pane)).collect();
        let mut layout_tabs=HashSet::new();
        let mut layout_panes=0usize;
        let mut seed=None;
        for layout in &self.layouts {
            if tabs.get(&layout.tab_id)!=Some(&&layout.workspace_id)||!layout_tabs.insert(&layout.tab_id)
                ||layout.area.width>4096||layout.area.height>4096
                ||usize::from(layout.area.width)*usize::from(layout.area.height)>1_000_000
            {return Err(invalid("invalid native layout"));}
            layout_panes=layout_panes.checked_add(layout.panes.len()).ok_or_else(limit)?;
            if layout_panes>512 {return Err(limit());}
            let mut members=HashSet::new();
            for member in &layout.panes {
                let pane=panes.get(&member.pane_id).ok_or_else(||invalid("layout pane absent"))?;
                if pane.tab_id!=layout.tab_id||pane.workspace_id!=layout.workspace_id
                    ||!members.insert(&member.pane_id)||!layout.area.contains(member.rect)
                {return Err(invalid("layout pane routing/geometry changed"));}
            }
            if !members.contains(&layout.focused_pane_id) {return Err(invalid("focused layout pane absent"));}
            if layout.tab_id==selected.tab_id&&layout.workspace_id==selected.workspace_id&&members.contains(&selected.pane_id) {
                if layout.area.width==0||layout.area.height==0 {return Err(invalid("initial terminal geometry unavailable"));}
                seed=Some((layout.area.width,layout.area.height));
            }
        }
        seed.ok_or_else(||invalid("initial terminal layout unavailable"))
    }
    pub fn no_focus(&self) -> io::Result<()> {
        if self.focused_workspace_id.is_none() {
            Err(invalid(
                "Herdr requires an existing active workspace to guarantee no-focus creation",
            ))
        } else {
            Ok(())
        }
    }
}

fn native_label(
    value: Option<&str>,
    unavailable: &mut Option<NativeDisplayUnavailable>,
) -> Option<Label> {
    let value = value?;
    match Label::new(value) {
        Ok(label) => Some(label),
        Err(_) => {
            *unavailable = Some(if value.len() > 512 || value.chars().count() > 128 {
                NativeDisplayUnavailable::TooLarge
            } else {
                NativeDisplayUnavailable::Unsafe
            });
            None
        }
    }
}

impl Snapshot {
    fn hierarchy(&self, server: &Uuid, incarnation: &Uuid, current: bool) -> NativeHierarchy {
        let workspaces = self
            .workspaces
            .iter()
            .map(|w| {
                let mut unavailable = None;
                NativeWorkspace {
                    workspace_id: w.workspace_id.clone(),
                    number: w.number,
                    label: native_label(Some(&w.label), &mut unavailable),
                    focused: w.focused,
                    active_tab_id: (!w.active_tab_id.is_empty())
                        .then(|| Id(w.active_tab_id.clone())),
                    agent_status: w.agent_status,
                    display_unavailable: unavailable,
                }
            })
            .collect();
        let tabs = self
            .tabs
            .iter()
            .map(|t| {
                let mut unavailable = None;
                NativeTab {
                    tab_id: t.tab_id.clone(),
                    workspace_id: t.workspace_id.clone(),
                    number: t.number,
                    label: native_label(Some(&t.label), &mut unavailable),
                    focused: t.focused,
                    agent_status: t.agent_status,
                    display_unavailable: unavailable,
                }
            })
            .collect();
        let agents: HashMap<_, _> = self.agents.iter().map(|a| (&a.terminal_id, a)).collect();
        let panes = self
            .panes
            .iter()
            .map(|p| {
                let mut unavailable = None;
                NativePane {
                    pane_id: p.pane_id.clone(),
                    terminal_id: p.terminal_id.clone(),
                    workspace_id: p.workspace_id.clone(),
                    tab_id: p.tab_id.clone(),
                    label: native_label(p.label.as_deref(), &mut unavailable),
                    title: native_label(
                        p.title.as_deref().or_else(|| {
                            agents.get(&p.terminal_id).and_then(|a| a.title.as_deref())
                        }),
                        &mut unavailable,
                    ),
                    terminal_title: native_label(
                        p.terminal_title_stripped.as_deref(),
                        &mut unavailable,
                    ),
                    display_agent: native_label(p.display_agent.as_deref(), &mut unavailable),
                    agent: native_label(
                        p.agent.as_deref().or_else(|| {
                            agents.get(&p.terminal_id).and_then(|a| a.agent.as_deref())
                        }),
                        &mut unavailable,
                    ),
                    agent_name: native_label(
                        agents.get(&p.terminal_id).and_then(|a| a.name.as_deref()),
                        &mut unavailable,
                    ),
                    focused: p.focused,
                    agent_status: p.agent_status,
                    // Raw native restore diagnostics can contain private paths/process reports.
                    restore_reason: p
                        .restore_error
                        .as_ref()
                        .map(|_| NativeRestoreReason::RestoreFailed),
                    display_unavailable: unavailable,
                    target: None,
                    unbound_row_id: None,
                    shell_target: None,
                }
            })
            .collect();
        NativeHierarchy {
            server_id: server.clone(),
            server_incarnation: incarnation.clone(),
            freshness: if current {
                NativeFreshness::Current
            } else {
                NativeFreshness::Stale
            },
            reason: (!current).then_some(NativeUnavailableReason::SourceUnavailable),
            workspaces,
            tabs,
            panes,
        }
    }
}
#[derive(Clone, Debug, Deserialize)]
pub struct PaneProcessInfo {
    pub pane_id: Id,
    pub shell_pid: Option<u32>,
    pub foreground_process_group_id: Option<u32>,
    pub tty: Option<String>,
    #[serde(default)]
    pub foreground_processes: Vec<PaneProcess>,
}
#[derive(Clone, Debug, Deserialize)]
pub struct PaneProcess {
    pub pid: u32,
    pub name: String,
    pub argv0: Option<String>,
    pub argv: Option<Vec<String>>,
    pub cwd: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessEvidence {
    pub linux_boot_id: Uuid,
    pub pid: u32,
    pub start_ticks: u64,
    pub namespaces: ProcessNamespaces,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessNamespaces {
    pub pid: (u64, u64),
    pub mount: (u64, u64),
    pub user: (u64, u64),
    /// Ordered visible host/proc-view to innermost namespace PIDs; not a guessed join.
    pub nspids: Vec<u32>,
}
pub fn namespace_pids(status: &str, visible_pid: u32) -> io::Result<Vec<u32>> {
    let line = status
        .lines()
        .find_map(|line| line.strip_prefix("NSpid:"))
        .ok_or_else(|| invalid("PID namespace translation unavailable"))?;
    let mut result = Vec::new();
    for token in line.split_whitespace() {
        if result.len() == 32 {
            return Err(invalid("PID namespace nesting limit"));
        }
        result.push(
            token
                .parse::<u32>()
                .ok()
                .filter(|pid| *pid != 0)
                .ok_or_else(|| invalid("invalid namespace PID"))?,
        );
    }
    if result.first() != Some(&visible_pid) {
        return Err(invalid(
            "visible process PID disagrees with proc namespace view",
        ));
    }
    Ok(result)
}
fn namespaces(root: &Path, visible_pid: u32) -> io::Result<ProcessNamespaces> {
    let identity = |name| -> io::Result<(u64, u64)> {
        let m = fs::metadata(root.join(format!("ns/{name}")))?;
        Ok((m.dev(), m.ino()))
    };
    Ok(ProcessNamespaces {
        pid: identity("pid")?,
        mount: identity("mnt")?,
        user: identity("user")?,
        nspids: namespace_pids(&fs::read_to_string(root.join("status"))?, visible_pid)?,
    })
}
/// Linux stat field22; comm may contain spaces and ')' characters.
pub fn process_start_ticks(stat: &str, pid: u32) -> io::Result<u64> {
    let (number, _) = stat
        .split_once(' ')
        .ok_or_else(|| invalid("invalid process stat"))?;
    if number.parse::<u32>().ok() != Some(pid) {
        return Err(invalid("process PID mismatch"));
    }
    let close = stat
        .rfind(')')
        .ok_or_else(|| invalid("invalid process comm"))?;
    stat[close + 1..]
        .split_whitespace()
        .nth(19)
        .and_then(|n| n.parse().ok())
        .filter(|n| *n != 0)
        .ok_or_else(|| invalid("process start ticks unavailable"))
}
/// Before using a kernel peer PID as a proc pathname, prove procfs uses the caller's
/// actual PID numbering. Otherwise an equal number could name an unrelated host process.
pub fn validate_procfs_view(stat: &str, status: &str, actual_pid: u32) -> io::Result<()> {
    process_start_ticks(stat, actual_pid)?;
    namespace_pids(status, actual_pid)?;
    Ok(())
}
fn procfs_view() -> io::Result<(u64, ProcessNamespaces)> {
    let actual_pid = std::process::id();
    // SAFETY: getpid has no pointer/lifetime preconditions.
    if u32::try_from(unsafe { libc::getpid() }).ok() != Some(actual_pid) {
        return Err(invalid("actual caller PID unavailable"));
    }
    let stat = fs::read_to_string("/proc/self/stat")?;
    let status = fs::read_to_string("/proc/self/status")?;
    validate_procfs_view(&stat, &status, actual_pid)?;
    Ok((
        process_start_ticks(&stat, actual_pid)?,
        namespaces(Path::new("/proc/self"), actual_pid)?,
    ))
}
pub fn process_evidence(pid: u32) -> io::Result<ProcessEvidence> {
    if pid == 0 {
        return Err(invalid("peer PID unavailable in namespace"));
    }
    let caller_view = procfs_view()?;
    let root = PathBuf::from(format!("/proc/{pid}"));
    if fs::metadata(&root)?.uid() != pairing::uid() {
        return Err(invalid("process UID mismatch"));
    }
    let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id")?;
    let linux_boot_id =
        Uuid::new(boot.trim()).map_err(|_| invalid("Linux boot identity unavailable"))?;
    let first = process_start_ticks(&fs::read_to_string(root.join("stat"))?, pid)?;
    let before = namespaces(&root, pid)?;
    let after = namespaces(&root, pid)?;
    let second = process_start_ticks(&fs::read_to_string(root.join("stat"))?, pid)?;
    if first != second
        || before != after
        || procfs_view()? != caller_view
        || fs::metadata(&root)?.uid() != pairing::uid()
    {
        return Err(invalid(
            "process changed or procfs PID view unavailable during evidence capture",
        ));
    }
    Ok(ProcessEvidence {
        linux_boot_id,
        pid,
        start_ticks: first,
        namespaces: before,
    })
}
/// Join an actual kernel/proc-evidenced candidate in the reporter's native namespace,
/// or in an independently attested caller-visible reporting namespace. No name/cwd match.
pub fn reported_pid_matches(
    reporter: &ProcessEvidence,
    candidate: &ProcessEvidence,
    reported: u32,
) -> bool {
    let Ok((_, caller)) = procfs_view() else {
        return false;
    };
    (reporter.namespaces.pid == candidate.namespaces.pid
        && candidate.namespaces.nspids.last() == Some(&reported))
        || (reporter.namespaces.pid == caller.pid
            && candidate.namespaces.nspids.first() == Some(&candidate.pid)
            && reported == candidate.pid)
}
fn socket_identity(path: &Path) -> io::Result<(u64, u64)> {
    config::socket_parent(path)?;
    let m = fs::symlink_metadata(path)?;
    if !m.file_type().is_socket()
        || m.uid() != pairing::uid()
        || m.mode() & 0o7777 != 0o600
        || fs::canonicalize(path)? != path
    {
        return Err(invalid("unsafe Herdr socket"));
    }
    Ok((m.dev(), m.ino()))
}
/// Bounded NDJSON, newline required; no unbounded read_line allocation.
pub async fn read_line<R: tokio::io::AsyncBufRead + Unpin>(reader: &mut R) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return Err(invalid("truncated Herdr line"));
        }
        let newline = available.iter().position(|b| *b == b'\n');
        let count = newline.map_or(available.len(), |n| n + 1);
        if bytes.len() + count > FRAME_MAX {
            return Err(limit());
        }
        bytes.extend_from_slice(&available[..count]);
        reader.consume(count);
        if newline.is_some() {
            bytes.pop();
            return Ok(bytes);
        }
    }
}
#[derive(Debug)]
pub enum ApiError {
    Rejected(String),
    Changed(Value),
    Unavailable(io::Error),
}
impl From<io::Error> for ApiError {
    fn from(e: io::Error) -> Self {
        Self::Unavailable(e)
    }
}
#[derive(Debug)]
pub(crate) struct ProcessInfoNotFound;
impl std::fmt::Display for ProcessInfoNotFound {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Herdr pane.process_info rejected: pane_not_found")
    }
}
impl std::error::Error for ProcessInfoNotFound {}
/// Exact ID/discriminant and success/error exclusivity, with recursive duplicate-key guards.
pub fn response(bytes: &[u8], request_id: &str, discriminant: &str) -> Result<Value, ApiError> {
    let mut value = native_json::parse(bytes, native_json::NATIVE)
        .map_err(|_| invalid("invalid Herdr JSON"))?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| invalid("Herdr response is not an object"))?;
    if object.len() != 2 || object.get("id").and_then(Value::as_str) != Some(request_id) {
        return Err(invalid("Herdr response ID/envelope mismatch").into());
    }
    if let Some(error) = object.get("error") {
        let error = error
            .as_object()
            .ok_or_else(|| invalid("invalid Herdr error"))?;
        if error.len() != 2 || error.get("message").and_then(Value::as_str).is_none() {
            return Err(invalid("invalid Herdr error body").into());
        }
        return Err(ApiError::Rejected(
            error
                .get("code")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("invalid Herdr error code"))?
                .to_owned(),
        ));
    }
    let result = object
        .remove("result")
        .ok_or_else(|| invalid("Herdr result missing"))?;
    if result.get("type").and_then(Value::as_str) != Some(discriminant) {
        return Err(invalid("Herdr result discriminant mismatch").into());
    }
    Ok(result)
}
pub struct Client {
    path: PathBuf,
    reader: BufReader<UnixStream>,
    pub evidence: ServerEvidence,
}
impl Client {
    pub async fn connect(path: &Path) -> io::Result<Self> {
        let before = socket_identity(path)?;
        let stream = tokio::time::timeout(READ_TIMEOUT, UnixStream::connect(path)).await??;
        let cred = stream.peer_cred()?;
        if cred.uid() != pairing::uid() {
            return Err(invalid("Herdr peer UID mismatch"));
        }
        let process = process_evidence(
            cred.pid()
                .and_then(|pid| u32::try_from(pid).ok())
                .ok_or_else(|| invalid("Herdr peer PID unavailable"))?,
        )?;
        if socket_identity(path)? != before {
            return Err(invalid("Herdr socket changed during connect"));
        }
        Ok(Self {
            path: path.to_owned(),
            reader: BufReader::new(stream),
            evidence: ServerEvidence {
                linux_boot_id: process.linux_boot_id,
                pid: process.pid,
                pid_start_ticks: process.start_ticks,
                socket_device: before.0,
                socket_inode: before.1,
            },
        })
    }
    pub fn recheck(&self) -> io::Result<()> {
        let process = process_evidence(self.evidence.pid)?;
        if process.linux_boot_id != self.evidence.linux_boot_id
            || process.start_ticks != self.evidence.pid_start_ticks
            || socket_identity(&self.path)?
                != (self.evidence.socket_device, self.evidence.socket_inode)
        {
            return Err(invalid("Herdr process/socket epoch changed"));
        }
        Ok(())
    }
    pub fn fence_socket(&self) -> io::Result<OwnedFd> {
        self.reader.get_ref().as_fd().try_clone_to_owned()
    }
    pub fn metadata(&self) -> StageMetadata {
        StageMetadata {
            linux_boot_id: Some(self.evidence.linux_boot_id.clone()),
            pid: Some(self.evidence.pid),
            pid_start_ticks: Some(self.evidence.pid_start_ticks),
            socket_path: self.path.to_str().map(str::to_owned),
            ..Default::default()
        }
    }
    async fn send(&mut self, method: &str, params: Value) -> io::Result<String> {
        self.recheck()?;
        let id = Uuid::random().0;
        let mut request = serde_json::to_vec(&json!({"id":id,"method":method,"params":params}))?;
        if request.len() + 1 > FRAME_MAX {
            return Err(limit());
        }
        request.push(b'\n');
        self.reader.get_mut().write_all(&request).await?;
        Ok(id)
    }
    pub async fn request(
        self,
        method: &str,
        params: Value,
        discriminant: &str,
    ) -> Result<Value, ApiError> {
        self.request_reported(method, params, discriminant, None)
            .await
    }
    pub(crate) async fn request_reported(
        mut self,
        method: &str,
        params: Value,
        discriminant: &str,
        reporter: Option<OmpCreationReporter>,
    ) -> Result<Value, ApiError> {
        let selected =
            method == "agent.start" && params.get("kind").and_then(Value::as_str) == Some("omp");
        let result = tokio::time::timeout(READ_TIMEOUT, async {
            let id = self.send(method, params).await?;
            let bytes = read_line(&mut self.reader).await?;
            let value = response(&bytes, &id, discriminant)?;
            if self.recheck().is_err() {
                return Err(ApiError::Changed(value));
            }
            Ok(value)
        })
        .await;
        match result {
            Err(_) => {
                if selected && let Some(reporter) = &reporter {
                    reporter.report_agent("herdr_response_timeout", None);
                }
                Err(ApiError::Unavailable(invalid("Herdr response timeout")))
            }
            Ok(Ok(value)) => Ok(value),
            Ok(Err(ApiError::Unavailable(error))) => {
                if selected && let Some(reporter) = &reporter {
                    reporter.report_agent_io("herdr_response_io", &error);
                }
                Err(ApiError::Unavailable(error))
            }
            Ok(Err(ApiError::Changed(value))) => {
                if selected && let Some(reporter) = &reporter {
                    reporter.report_agent("source_changed", None);
                }
                Err(ApiError::Changed(value))
            }
            Ok(Err(ApiError::Rejected(code))) => Err(ApiError::Rejected(code)),
        }
    }
    pub async fn snapshot(self) -> io::Result<(ServerEvidence, Snapshot)> {
        let evidence = self.evidence.clone();
        let mut result = self
            .request("session.snapshot", json!({}), "session_snapshot")
            .await
            .map_err(api_io)?;
        let snapshot: Snapshot = serde_json::from_value(
            result
                .get_mut("snapshot")
                .ok_or_else(|| invalid("snapshot absent"))?
                .take(),
        )?;
        snapshot.validate()?;
        Ok((evidence, snapshot))
    }
    pub async fn read_bound(
        self,
        pane: &Id,
        terminal: &Id,
        identity: &PaneReadIdentity,
    ) -> io::Result<PaneReadBoundResult> {
        let mut result = self
            .request(
                "pane.read_bound",
                json!({
                    "pane_id": pane,
                    "expected_terminal_id": terminal,
                    "expected_read_identity": {
                        "herdr_incarnation": identity.herdr_incarnation,
                        "runtime_epoch": identity.runtime_epoch,
                        "binding_epoch": identity.binding_epoch,
                    },
                }),
                "pane_read_bound",
            )
            .await
            .map_err(api_io)?;
        let read: PaneReadBoundResult = serde_json::from_value(
            result
                .get_mut("read")
                .ok_or_else(|| invalid("bounded pane read absent"))?
                .take(),
        )?;
        read.read_identity.validate()?;
        if read.pane_id != *pane
            || read.terminal_id != *terminal
            || &read.read_identity != identity
            || read.text.len() > 16 * 1024
        {
            return Err(invalid("bounded pane read identity/text mismatch"));
        }
        Ok(read)
    }
    pub async fn agent_rename(self,pane:&Id,name:&AgentName)->Result<Agent,ApiError> {
        let mut result=self.request("agent.rename",json!({"target":pane,"name":name.0}),"agent_info").await?;
        let agent=result.get_mut("agent").ok_or_else(||ApiError::Unavailable(invalid("native agent result absent")))?.take();
        serde_json::from_value(agent).map_err(|_|ApiError::Unavailable(invalid("invalid native agent result")))
    }
    pub async fn workspace_rename(self,workspace:&Id,label:&Label)->Result<Workspace,ApiError> {
        let mut result=self.request("workspace.rename",json!({"workspace_id":workspace,"label":label.0}),"workspace_info").await?;
        let workspace=result.get_mut("workspace").ok_or_else(||ApiError::Unavailable(invalid("native workspace result absent")))?.take();
        serde_json::from_value(workspace).map_err(|_|ApiError::Unavailable(invalid("invalid native workspace result")))
    }
    /// Public reported PIDs remain in Herdr's namespace; callers must prove translation
    /// using actual kernel peer/process evidence before treating them as proc pathnames.
    pub async fn process_info(self, pane: &Id) -> io::Result<PaneProcessInfo> {
        let mut result = self
            .request(
                "pane.process_info",
                json!({"pane_id":pane}),
                "pane_process_info",
            )
            .await
            .map_err(|error| match error {
                ApiError::Rejected(code) if code == "pane_not_found" => {
                    io::Error::new(io::ErrorKind::InvalidData, ProcessInfoNotFound)
                }
                error => api_io(error),
            })?;
        let info: PaneProcessInfo = serde_json::from_value(
            result
                .get_mut("process_info")
                .ok_or_else(|| invalid("process info absent"))?
                .take(),
        )?;
        if &info.pane_id != pane || info.foreground_processes.len() > 512 {
            return Err(invalid("pane process mismatch/limit"));
        }
        Ok(info)
    }
}
fn api_io(error: ApiError) -> io::Error {
    match error {
        ApiError::Rejected(_) => invalid("Herdr rejected public request"),
        ApiError::Changed(_) => invalid("Herdr source changed after response"),
        ApiError::Unavailable(e) => e,
    }
}
const EVENTS: &[&str] = &[
    "workspace.created",
    "workspace.updated",
    "workspace.metadata_updated",
    "workspace.renamed",
    "workspace.moved",
    "workspace.reordered",
    "workspace.closed",
    "workspace.focused",
    "tab.created",
    "tab.closed",
    "tab.focused",
    "tab.renamed",
    "tab.moved",
    "pane.created",
    "pane.closed",
    "pane.updated",
    "pane.focused",
    "pane.moved",
    "pane.exited",
    "pane.agent_detected",
    "layout.updated",
];
struct Subscription {
    reader: BufReader<UnixStream>,
    id: String,
    evidence: ServerEvidence,
}
impl Subscription {
    async fn connect(path: &Path, panes: &[Id]) -> io::Result<Self> {
        let mut client = Client::connect(path).await?;
        let mut subscriptions: Vec<Value> =
            EVENTS.iter().map(|event| json!({"type":event})).collect();
        subscriptions.extend(
            panes
                .iter()
                .map(|pane| json!({"type":"pane.agent_status_changed","pane_id":pane})),
        );
        let id = tokio::time::timeout(
            READ_TIMEOUT,
            client.send("events.subscribe", json!({"subscriptions":subscriptions})),
        )
        .await??;
        let bytes = tokio::time::timeout(READ_TIMEOUT, read_line(&mut client.reader)).await??;
        response(&bytes, &id, "subscription_started").map_err(api_io)?;
        client.recheck()?;
        Ok(Self {
            reader: client.reader,
            id,
            evidence: client.evidence,
        })
    }
    async fn event(&mut self) -> io::Result<()> {
        let value = parse_raw_json(&read_line(&mut self.reader).await?)
            .map_err(|_| invalid("invalid event JSON"))?;
        if value.get("error").is_some() {
            return Err(invalid("Herdr events lost/server unavailable"));
        }
        let object = value
            .as_object()
            .ok_or_else(|| invalid("invalid event envelope"))?;
        if object.len() != 2
            || object.get("event").and_then(Value::as_str).is_none()
            || !object.get("data").is_some_and(Value::is_object)
            || object.contains_key("id")
            || self.id.is_empty()
        {
            return Err(invalid("standalone Herdr event required"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveredSlot {
    pub name: String,
    pub default: bool,
    pub running: bool,
    pub socket_path: PathBuf,
    pub session_dir: PathBuf,
}
/// Shell-free discovery, bounded child output, deadline, and no server start.
pub async fn discover(config: &Config) -> io::Result<Vec<DiscoveredSlot>> {
    config::executable(&config.executables.herdr)?;
    let mut child = tokio::process::Command::new(&config.executables.herdr)
        .args(["session", "list", "--json"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| invalid("discovery stdout unavailable"))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| invalid("discovery stderr unavailable"))?;
    let result = tokio::time::timeout(READ_TIMEOUT, async {
        let out = async {
            let mut bytes = Vec::new();
            (&mut stdout)
                .take((FRAME_MAX + 1) as u64)
                .read_to_end(&mut bytes)
                .await?;
            Ok::<_, io::Error>(bytes)
        };
        let err = async {
            let mut bytes = Vec::new();
            (&mut stderr)
                .take((FRAME_MAX + 1) as u64)
                .read_to_end(&mut bytes)
                .await?;
            Ok::<_, io::Error>(bytes)
        };
        let (bytes, errors) = tokio::try_join!(out, err)?;
        if bytes.len() > FRAME_MAX || errors.len() > FRAME_MAX {
            return Err(limit());
        }
        if !child.wait().await?.success() {
            return Err(invalid("configured Herdr discovery failed"));
        }
        Ok(bytes)
    })
    .await;
    let bytes = match result {
        Ok(value) => value?,
        Err(_) => {
            child.kill().await?;
            return Err(invalid("Herdr discovery timeout"));
        }
    };
    let value = parse_raw_json(&bytes).map_err(|_| invalid("invalid Herdr discovery JSON"))?;
    let object = value
        .as_object()
        .ok_or_else(|| invalid("invalid discovery envelope"))?;
    if object.len() != 1 {
        return Err(invalid("invalid discovery envelope"));
    }
    let mut slots: Vec<DiscoveredSlot> = serde_json::from_value(
        object
            .get("sessions")
            .cloned()
            .ok_or_else(|| invalid("discovery sessions absent"))?,
    )?;
    for socket in &config.herdr_sockets {
        if !slots.iter().any(|s| s.socket_path == *socket) {
            slots.push(DiscoveredSlot {
                name: "Configured Herdr".into(),
                default: false,
                running: socket.exists(),
                socket_path: socket.clone(),
                session_dir: socket
                    .parent()
                    .ok_or_else(|| invalid("socket parent absent"))?
                    .to_owned(),
            });
        }
    }
    canonical_slots(slots)
}
pub fn canonical_slots(slots: Vec<DiscoveredSlot>) -> io::Result<Vec<DiscoveredSlot>> {
    let mut result = Vec::new();
    let mut paths = HashSet::new();
    let mut objects = HashSet::new();
    for slot in slots {
        config::socket_path(&slot.socket_path)?;
        Label::new(&slot.name).map_err(|_| invalid("invalid named-server label"))?;
        let parent = slot
            .socket_path
            .parent()
            .ok_or_else(|| invalid("socket parent absent"))?;
        if parent.exists() && fs::canonicalize(parent)? != parent {
            return Err(invalid("noncanonical discovery socket parent"));
        }
        // Deduplicate verified physical aliases as well as canonical pathname aliases.
        // Socket object equality is grouping only; peer/process proof still occurs at connect.
        if let Ok(identity) = socket_identity(&slot.socket_path)
            && !objects.insert(identity)
        {
            continue;
        }
        if paths.insert(slot.socket_path.clone()) {
            result.push(slot);
        }
    }
    if result.len() > 64 {
        return Err(limit());
    }
    Ok(result)
}

#[derive(Clone, Debug)]
pub struct Source {
    pub server_id: Uuid,
    pub incarnation: Uuid,
    pub socket_path: PathBuf,
    pub evidence: ServerEvidence,
    pub snapshot: Arc<Snapshot>,
    revision: u64,
    invalidation: Arc<AtomicU64>,
}
impl Source {
    /// Exact source owner, independent of a merely advanced inventory revision.
    pub(crate) fn same_physical_identity(&self,current:&Source)->bool {
        self.server_id==current.server_id&&self.incarnation==current.incarnation&&self.socket_path==current.socket_path&&self.evidence==current.evidence&&Arc::ptr_eq(&self.invalidation,&current.invalidation)
    }
    pub fn recheck(&self) -> io::Result<()> {
        if self.recheck_readiness()? {
            Ok(())
        } else {
            Err(invalid("Herdr topology requires authoritative refresh"))
        }
    }
    /// Read-only readiness: only an authoritative revision gap is pending.
    /// Physical I/O/identity errors are preserved rather than converted into a wait.
    pub(crate) fn recheck_readiness(&self) -> io::Result<bool> {
        if !self.revision_current() {
            return Ok(false);
        }
        self.recheck_identity()?;
        Ok(true)
    }
    fn revision_current(&self) -> bool {
        self.invalidation.load(Ordering::Acquire) == self.revision
    }
    /// Display commit fence only. Physical/native proof precedes the State lock; this
    /// reuses the revision-sensitive atomic predicate and cannot authorize native effects.
    pub(crate) fn fence_inventory(&self, graph: &mut NativeHierarchy) -> SecurityResult<()> {
        if graph.server_id != self.server_id || graph.server_incarnation != self.incarnation {
            return Err(ErrorCode::StaleSession);
        }
        if !self.revision_current() {
            if graph.freshness != NativeFreshness::Unavailable {
                graph.freshness = NativeFreshness::Stale;
            }
            graph.reason = Some(NativeUnavailableReason::SourceUnavailable);
            for pane in &mut graph.panes {
                pane.target = None;
                pane.unbound_row_id = None;
                pane.shell_target = None;
            }
        }
        Ok(())
    }
    /// Physical identity alone is not topology authority: an old revision still fails recheck.
    pub(crate) fn recheck_identity(&self) -> io::Result<()> {
        let p = process_evidence(self.evidence.pid)?;
        if p.linux_boot_id != self.evidence.linux_boot_id
            || p.start_ticks != self.evidence.pid_start_ticks
            || socket_identity(&self.socket_path)?
                != (self.evidence.socket_device, self.evidence.socket_inode)
        {
            return Err(invalid("Herdr source epoch changed"));
        }
        Ok(())
    }
}

/// Private, immutable provenance retained before the original native launch. This is
/// intentionally not a Goatr model or wire payload: it exists only for the selected
/// owned diagnostic's one post-settlement native terminal read.
#[derive(Serialize)]
struct CodexStartupTerminalPin {
    action: Uuid,
    server_id: Uuid,
    source_incarnation: Uuid,
    source_evidence: ServerEvidence,
    socket_path: PathBuf,
    workspace_id: Id,
    tab_id: Id,
    pane_id: Id,
    terminal_id: Id,
    read_identity: PaneReadIdentity,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellDialect {
    Bash,
    Zsh,
}
struct CoarseShell {
    shell: ProcessEvidence,
    reporter: ProcessEvidence,
    executable: PathBuf,
    dialect: ShellDialect,
    incarnation: Uuid,
}

/// Closed classifications for the existing root check.  This retains the first failed
/// predicate without making another topology observation for diagnostics.
#[derive(Clone, Copy)]
enum ProbeRootGuard {
    SourceRecheck,
    Pane,
    Workspace,
    Terminal,
    Focus,
    PaneFocused,
    Agent,
    AgentSession,
    AgentStatus,
    Agents,
}

/// Closed classifications for the existing source/root identity comparison.
/// These retain its first failed predicate without another topology observation.
#[derive(Clone, Copy)]
enum CreationRootGuard {
    ServerId,
    Incarnation,
    Evidence,
    SocketPath,
    Invalidation,
}
impl CreationRootGuard {
    const fn shell_probe_guard(self) -> &'static str {
        match self {
            Self::ServerId => "final_root_server_id",
            Self::Incarnation => "final_root_incarnation",
            Self::Evidence => "final_root_evidence",
            Self::SocketPath => "final_root_socket_path",
            Self::Invalidation => "final_root_invalidation",
        }
    }
}
impl ProbeRootGuard {
    const fn deadline_guard(self) -> &'static str {
        match self {
            Self::SourceRecheck => "root_source_recheck_at_deadline",
            Self::Pane => "root_pane_at_deadline",
            Self::Workspace => "root_workspace_at_deadline",
            Self::Terminal => "root_terminal_at_deadline",
            Self::Focus => "root_focus_at_deadline",
            Self::PaneFocused => "root_pane_focused_at_deadline",
            Self::Agent => "root_agent_at_deadline",
            Self::AgentSession => "root_agent_session_at_deadline",
            Self::AgentStatus => "root_agent_status_at_deadline",
            Self::Agents => "root_agents_at_deadline",
        }
    }
    const fn immediate_guard(self) -> &'static str {
        match self {
            Self::SourceRecheck => "root_recheck_source",
            Self::Pane => "root_recheck_pane",
            Self::Workspace => "root_recheck_workspace",
            Self::Terminal => "root_recheck_terminal",
            Self::Focus => "root_recheck_focus",
            Self::PaneFocused => "root_recheck_pane_focused",
            Self::Agent => "root_recheck_agent",
            Self::AgentSession => "root_recheck_agent_session",
            Self::AgentStatus => "root_recheck_agent_status",
            Self::Agents => "root_recheck_agents",
        }
    }
}

#[derive(Clone, Copy)]
enum ProbeShellGuard {
    Reporter,
    ShellAbsent,
    ShellNotIdle,
    ShellProcess,
    ShellExecutable,
    ReporterRecheck,
}
impl ProbeShellGuard {
    const fn final_shell_probe_guard(self) -> &'static str {
        match self {
            Self::Reporter => "final_shell_reporter",
            Self::ShellAbsent => "final_shell_absent",
            Self::ShellNotIdle => "final_shell_not_idle",
            Self::ShellProcess => "final_shell_process",
            Self::ShellExecutable => "final_shell_executable",
            Self::ReporterRecheck => "final_shell_reporter_recheck",
        }
    }
    const fn shell_probe_guard(self) -> &'static str {
        match self {
            Self::Reporter => "shell_reporter",
            Self::ShellAbsent => "shell_absent",
            Self::ShellNotIdle => "shell_not_idle",
            Self::ShellProcess => "shell_process",
            Self::ShellExecutable => "shell_executable",
            Self::ReporterRecheck => "shell_reporter_recheck",
        }
    }
}
type ProbeShellCheck = Result<
    (ProcessEvidence, ProcessEvidence, PathBuf, ShellDialect),
    (ErrorCode, &'static str, ProbeShellGuard),
>;
pub fn reported_process(reporter: &ProcessEvidence, reported: u32) -> io::Result<ProcessEvidence> {
    if reported == 0 {
        return Err(invalid("reported shell PID is unavailable"));
    }
    if let Ok(candidate) = process_evidence(reported)
        && reported_pid_matches(reporter, &candidate, reported)
    {
        return Ok(candidate);
    }
    let mut found = None;
    let mut count = 0;
    for entry in fs::read_dir("/proc")? {
        let entry = entry?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        count += 1;
        if count > 8192 {
            return Err(limit());
        }
        if let Ok(candidate) = process_evidence(pid)
            && reported_pid_matches(reporter, &candidate, reported)
        {
            if found.is_some() {
                return Err(invalid("reported shell PID is ambiguous"));
            }
            found = Some(candidate);
        }
    }
    found.ok_or_else(|| invalid("reporting/native shell namespace translation is unproved"))
}
fn shell_executable(shell: &ProcessEvidence) -> io::Result<(PathBuf, ShellDialect)> {
    if process_evidence(shell.pid)? != *shell {
        return Err(invalid("shell process epoch changed"));
    }
    let proc = PathBuf::from(format!("/proc/{}/exe", shell.pid));
    let executable = fs::read_link(&proc)?;
    config::executable(&executable)?;
    let mut file = File::open(&proc)?;
    let identity = file.metadata()?;
    let mut magic = [0; 4];
    file.read_exact(&mut magic)?;
    if magic != *b"\x7fELF"
        || (identity.dev(), identity.ino()) != object(&executable)?
        || process_object(shell, &executable)? != object(&executable)?
        || process_evidence(shell.pid)? != *shell
    {
        return Err(invalid("shell executable object is unproved"));
    }
    let dialect = shell_source_dialect(&executable)?;
    Ok((executable, dialect))
}
fn shell_source_dialect(executable: &Path) -> io::Result<ShellDialect> {
    let output = executable
        .parent()
        .filter(|parent| parent.file_name().and_then(|s| s.to_str()) == Some("bin"))
        .and_then(Path::parent)
        .filter(|output| output.parent() == Some(Path::new("/nix/store")));
    let package = output
        .and_then(Path::file_name)
        .and_then(|s| s.to_str())
        .and_then(|s| s.split_once('-'))
        .filter(|(hash, _)| {
            hash.len() == 32
                && hash
                    .bytes()
                    .all(|b| b"0123456789abcdfghijklmnpqrsvwxyz".contains(&b))
        })
        .map(|(_, package)| package);
    // Package spelling admits the inspected source set; it is not byte attestation.
    match (package, executable.file_name().and_then(|s| s.to_str())) {
        (Some("bash-5.3p9" | "bash-interactive-5.3p9"), Some("bash")) => Ok(ShellDialect::Bash),
        (Some("zsh-5.9" | "zsh-5.9.1"), Some("zsh")) => Ok(ShellDialect::Zsh),
        _ => Err(invalid(
            "shell package/version is outside the approved source set; only GNU bash-5.3p9/bash-interactive-5.3p9 bin/bash or zsh-5.9/zsh-5.9.1 bin/zsh is supported",
        )),
    }
}
fn shell_word(value: &str) -> io::Result<String> {
    if value.chars().any(char::is_control) {
        return Err(invalid("probe literal contains control characters"));
    }
    Ok(format!("'{}'", value.replace('\'', "'\\''")))
}
/// One fixed read-only compound in an inherited subshell; never provider invocation/eval.
pub fn shell_probe_command(
    dialect: ShellDialect,
    provider: Provider,
    nonce: &str,
    wrapper: &Path,
    result: &Path,
) -> io::Result<String> {
    if nonce.len() != 64
        || !nonce
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
    {
        return Err(invalid("probe nonce is invalid"));
    }
    config::absolute(wrapper)?;
    config::absolute(result)?;
    let wrapper = shell_word(
        wrapper
            .to_str()
            .ok_or_else(|| invalid("wrapper path is not UTF-8"))?,
    )?;
    let result = shell_word(
        result
            .to_str()
            .ok_or_else(|| invalid("result path is not UTF-8"))?,
    )?;
    let kind = match provider {
        Provider::Omp => "omp",
        Provider::Codex => "codex",
    };
    let ok = format!("'builtin' 'printf' '1|{nonce}|%s|file|1|done\\n' \"$$\"");
    let rejected_kind = format!("'builtin' 'printf' '1|{nonce}|%s|rejected_kind|0|done\\n' \"$$\"");
    let rejected_path = format!("'builtin' 'printf' '1|{nonce}|%s|rejected_path|0|done\\n' \"$$\"");
    let unsupported = format!("'builtin' 'printf' '1|{nonce}|%s|unsupported|0|done\\n' \"$$\"");
    let body = match dialect {
        ShellDialect::Bash => format!(
            "if [[ \"$-\" != *[xv]* && \"${{BASH_VERSINFO[0]-}}\" == '5' && \"${{BASH_VERSINFO[1]-}}\" == '3' && \"${{BASH_VERSINFO[2]-}}\" == '9' && \"$('builtin' 'type' '-t' 'builtin')\" == 'builtin' && \"$('builtin' 'type' '-t' 'type')\" == 'builtin' && \"$('builtin' 'type' '-t' 'printf')\" == 'builtin' ]]; then case \"$('builtin' 'type' '-t' '{kind}')\" in 'file') if [[ \"$('builtin' 'type' '-p' '{kind}')\" -ef {wrapper} ]]; then {ok}; else {rejected_path}; fi ;; *) {rejected_kind} ;; esac; else {unsupported}; fi"
        ),
        ShellDialect::Zsh => format!(
            "if [[ \"$-\" != *[xv]* && ( \"$ZSH_VERSION\" == '5.9' || \"$ZSH_VERSION\" == '5.9.1' ) && \"$('builtin' 'whence' '-w' 'whence')\" == 'whence: builtin' && \"$('builtin' 'whence' '-w' 'printf')\" == 'printf: builtin' ]]; then case \"$('builtin' 'whence' '-w' '{kind}')\" in '{kind}: command'|'{kind}: hashed') if [[ \"$('builtin' 'whence' '{kind}')\" -ef {wrapper} ]]; then {ok}; else {rejected_path}; fi ;; *) {rejected_kind} ;; esac; else {unsupported}; fi"
        ),
    };
    // Subshell-local resolution/hash state cannot rewrite the parent's command cache.
    let command = format!(" ( {body} ) >| {result} 2>/dev/null");
    if command.len() > 8192 || command.chars().any(char::is_control) {
        return Err(limit());
    }
    Ok(command)
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum ShellProbeResult {
    File(bool),
    Rejected,
    RejectedKind,
    RejectedPath,
    Unsupported,
}
fn shell_probe_result_category(
    bytes: &[u8],
    nonce: &str,
    shell: &ProcessEvidence,
) -> io::Result<ShellProbeResult> {
    if bytes.is_empty() || bytes.len() > 8192 || !bytes.ends_with(b"\n") {
        return Err(invalid("probe result is incomplete or oversized"));
    }
    let line = std::str::from_utf8(&bytes[..bytes.len() - 1])
        .map_err(|_| invalid("probe result is not UTF-8"))?;
    let mut parts = line.split('|');
    let fields: [&str; 6] = std::array::from_fn(|_| parts.next().unwrap_or(""));
    if parts.next().is_some()
        || fields[0] != "1"
        || fields[1] != nonce
        || fields[5] != "done"
        || !matches!(
            fields[3],
            "file" | "rejected" | "rejected_kind" | "rejected_path" | "unsupported"
        )
        || !matches!(fields[4], "0" | "1")
        || fields[2].is_empty()
        || fields[2].starts_with('0')
        || !fields[2].bytes().all(|b| b.is_ascii_digit())
        || fields[2].parse::<u32>().ok().as_ref() != shell.namespaces.nspids.last()
        || (fields[4] == "1" && fields[3] != "file")
    {
        return Err(invalid("probe nonce/PID/category binding does not match"));
    }
    Ok(match fields[3] {
        "file" => ShellProbeResult::File(fields[4] == "1"),
        "rejected" => ShellProbeResult::Rejected,
        "rejected_kind" => ShellProbeResult::RejectedKind,
        "rejected_path" => ShellProbeResult::RejectedPath,
        "unsupported" => ShellProbeResult::Unsupported,
        _ => unreachable!("validated shell-probe category"),
    })
}
pub fn shell_probe_result(bytes: &[u8], nonce: &str, shell: &ProcessEvidence) -> io::Result<bool> {
    Ok(matches!(
        shell_probe_result_category(bytes, nonce, shell)?,
        ShellProbeResult::File(true)
    ))
}
struct ShellProbeEffectDropWitness {
    reporter: Option<OmpCreationReporter>,
    codex_reporter: Option<CodexInvalidationReporter>,
    armed: bool,
}
impl ShellProbeEffectDropWitness {
    fn new(
        reporter: Option<OmpCreationReporter>,
        codex_reporter: Option<CodexInvalidationReporter>,
    ) -> Self {
        Self {
            reporter,
            codex_reporter,
            armed: true,
        }
    }
    fn disarm(&mut self) {
        self.armed = false;
    }
}
impl Drop for ShellProbeEffectDropWitness {
    fn drop(&mut self) {
        if self.armed {
            if let Some(reporter) = &self.reporter {
                reporter.report_shell_probe("query_effect_dropped", ErrorCode::NotReady, "none");
            }
            if let Some(reporter) = &self.codex_reporter {
                reporter.shell_probe_prebinding(
                    "query_effect_dropped",
                    ErrorCode::NotReady,
                    "none",
                );
            }
        }
    }
}
struct ProbeArtifact {
    dir: PathBuf,
    path: PathBuf,
    provider_path: PathBuf,
    file: File,
    dir_object: (u64, u64),
    file_object: (u64, u64),
}
impl ProbeArtifact {
    fn create(config: &Config, attempt: &Uuid, shell: &ProcessEvidence) -> io::Result<Self> {
        use std::os::unix::fs::DirBuilderExt;
        pairing::private_dir(&config.runtime_dir, false)?;
        let name = format!("probe-{}", attempt.0);
        let dir = config.runtime_dir.join(&name);
        fs::DirBuilder::new().mode(0o700).create(&dir)?;
        let path = dir.join("result");
        let file = match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&path)
        {
            Ok(file) => file,
            Err(error) => {
                let _ = fs::remove_dir(&dir);
                return Err(error);
            }
        };
        let provider_path = config.runtime_dir_agent.join(name).join("result");
        let artifact = Self {
            dir_object: object(&dir)?,
            file_object: (file.metadata()?.dev(), file.metadata()?.ino()),
            dir,
            path,
            provider_path,
            file,
        };
        artifact.recheck(shell)?;
        Ok(artifact)
    }
    fn recheck(&self, shell: &ProcessEvidence) -> io::Result<()> {
        pairing::private_dir(&self.dir, false)?;
        let file = fs::symlink_metadata(&self.path)?;
        if !file.is_file()
            || file.uid() != pairing::uid()
            || file.mode() & 0o7777 != 0o600
            || file.nlink() != 1
            || (file.dev(), file.ino()) != self.file_object
            || object(&self.dir)? != self.dir_object
            || process_object(shell, &self.provider_path)? != self.file_object
            || process_object(
                shell,
                self.provider_path
                    .parent()
                    .ok_or_else(|| invalid("probe parent is absent"))?,
            )? != self.dir_object
            || process_evidence(shell.pid)? != *shell
        {
            return Err(invalid("private shell result object/view changed"));
        }
        Ok(())
    }
    fn read(&mut self, shell: &ProcessEvidence) -> io::Result<Vec<u8>> {
        self.recheck(shell)?;
        self.file.seek(SeekFrom::Start(0))?;
        let mut bytes = Vec::new();
        (&mut self.file).take(8193).read_to_end(&mut bytes)?;
        self.recheck(shell)?;
        if bytes.len() > 8192 {
            return Err(limit());
        }
        Ok(bytes)
    }
}
impl Drop for ProbeArtifact {
    fn drop(&mut self) {
        if fs::symlink_metadata(&self.path).is_ok_and(|m| {
            m.is_file()
                && m.nlink() == 1
                && m.uid() == pairing::uid()
                && (m.dev(), m.ino()) == self.file_object
        }) && object(&self.dir).is_ok_and(|o| o == self.dir_object)
        {
            let _ = fs::remove_file(&self.path);
            let _ = fs::remove_dir(&self.dir);
        }
    }
}
struct ServerRecord {
    slot: String,
    name: Label,
    path: PathBuf,
    server_id: Uuid,
    source: Option<Source>,
    reason: Option<String>,
    coarse_shell: Option<CoarseShell>,
}
struct Binding {
    key: StableSessionKey,
    target: Target,
    source_epoch: Uuid,
    herdr_report: Option<(Option<String>, Option<NativeSession>)>,
    terminal_read_pin: Option<PaneReadIdentity>,
}
/// Original logical rename scope; document/read grants are independent.
#[derive(Clone)]
pub(crate) struct RenameContext {
    source_epoch:Uuid,pane:Id,terminal:Id,workspace:Id,tab:Id,
}
/// Measured in the actual pane shell, including aliases/functions; not a remote/config flag.
/// The caller supplies the captured absolute result of shell resolution, never executes the
/// resolved raw child. Provider namespace object equivalence is rechecked through proc/root.
pub struct ShellMeasurement {
    pub execution_directory: PathBuf,
    pub server_incarnation: Uuid,
    pub pane_id: Id,
    pub shell: ProcessEvidence,
    pub resolved_outer_wrapper: PathBuf,
    pub project_id: Uuid,
    pub action_id: Uuid,
}
struct WrapperProof {
    action_id: Uuid,
    server_incarnation: Uuid,
    provider: Provider,
    shell: ProcessEvidence,
    reporter: ProcessEvidence,
    pane_id: Id,
    wrapper: PathBuf,
    project: PathBuf,
    authorization_project: PathBuf,
    project_id: Uuid,
    wrapper_object: (u64, u64),
    runtime_object: (u64, u64),
    project_object: (u64, u64),
    wrapper_stamp: (u64, i64, i64, i64, i64),
}
fn object(path: &Path) -> io::Result<(u64, u64)> {
    let m = fs::metadata(path)?;
    Ok((m.dev(), m.ino()))
}
fn wrapper_stamp(path: &Path) -> io::Result<(u64, i64, i64, i64, i64)> {
    let m = fs::metadata(path)?;
    Ok((
        m.len(),
        m.mtime(),
        m.mtime_nsec(),
        m.ctime(),
        m.ctime_nsec(),
    ))
}
fn root_object(root: &File, path: &Path) -> io::Result<(u64, u64)> {
    config::absolute(path)?;
    let path = CString::new(path.as_os_str().as_encoded_bytes())
        .map_err(|_| invalid("invalid provider pathname"))?;
    // SAFETY: the libc ABI is zero-filled; both pathname and root descriptor live across syscall.
    let mut how: libc::open_how = unsafe { std::mem::zeroed() };
    how.flags = (libc::O_PATH | libc::O_CLOEXEC) as u64;
    how.resolve = libc::RESOLVE_IN_ROOT | libc::RESOLVE_NO_MAGICLINKS;
    let fd = unsafe {
        libc::syscall(
            libc::SYS_openat2,
            root.as_raw_fd(),
            path.as_ptr(),
            &how,
            std::mem::size_of::<libc::open_how>(),
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful openat2 returns a new descriptor exclusively owned here.
    let file = unsafe { File::from_raw_fd(fd as libc::c_int) };
    let metadata = file.metadata()?;
    Ok((metadata.dev(), metadata.ino()))
}
pub fn process_object(shell: &ProcessEvidence, path: &Path) -> io::Result<(u64, u64)> {
    let root = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC)
        .open(format!("/proc/{}/root", shell.pid))?;
    root_object(&root, path)
}
struct Inner {
    servers: HashMap<Uuid, ServerRecord>,
    bindings: HashMap<Uuid, Binding>,
    shell_bindings: HashMap<Uuid, ShellBinding>,
    proofs: Vec<WrapperProof>,
    unbound_ids: HashMap<(Uuid, Uuid, Id, Provider), Uuid>,
    unbound_reasons: HashMap<(Uuid, Uuid, Id, Provider), (UnboundState, String)>,
}
/// One actual topology/config owner. Native providers install only proven existing states.
pub struct Topology {
    pub config: Config,
    storage: Arc<StorageOwner>,
    state: Arc<State>,
    host_id: EndpointId,
    artifact_leases:Mutex<artifacts::ArtifactLeases>,
    creation_roots:HashMap<Uuid,Option<Arc<crate::project_read::Root>>>,
    pub(crate) skill_documents:Arc<skills::Documents>,
    terminal_jobs:Mutex<tokio::task::JoinSet<()>>,
    terminal_slots:Arc<tokio::sync::Semaphore>,
    terminal_stopping:AtomicBool,
    terminal_safety:SecurityResult<()>,
    terminal_unproved_servers:HashSet<Uuid>,
    inner: Mutex<Inner>,
    // ponytail: serialize the short cross-owner identity commit, never native readiness/I/O.
    updates: tokio::sync::Mutex<()>,
    omp_creation_reason: Mutex<Option<String>>,
    codex_creation_reason: Mutex<Option<String>>,
    omp_offline_reporter: Arc<Mutex<Option<OmpCreationReporter>>>,
    codex_invalidation_reporter: Arc<Mutex<Option<CodexInvalidationReporter>>>,
}
impl Topology {
    pub async fn new(
        config: Config,
        storage: Arc<StorageOwner>,
        state: Arc<State>,
        host_id: EndpointId,
    ) -> SecurityResult<Arc<Self>> {
        config.preflight().map_err(policy)?;
        let (servers, sessions, shells) = storage
            .call(|s| Ok((s.persisted_servers()?, s.persisted_sessions()?,s.persisted_shells()?)))
            .await
            .map_err(pairing::store_error)?;
        let restored_fences=storage.call(|store|store.creation_shell_fences()).await;
        let (terminal_safety,terminal_unproved_servers)=match restored_fences {
            Ok((fences,unproved))=>{for(peer,action,fence)in fences{state.creation_shell_fence(peer,action,Some(fence));}(Ok(()),unproved)}
            Err(error)=>(Err(pairing::store_error(error)),HashSet::new()),
        };
        // Hold project objects once; creation, terminal and read grants stay independent.
        // An unavailable root disables only operations using that project, never chat.
        let creation_roots=config.projects.iter().map(|project| {
            let root=if config.allow_scoped_shell_probe||config.allow_terminal_control||project.allow_read_tools {
                crate::project_read::Root::open(&project.path,std::time::Instant::now()+Duration::from_secs(5)).ok().map(Arc::new)
            } else { None };
            (project.id.clone(),root)
        }).collect();
        let skill_documents=skills::Documents::new(&config);
        let owner = Arc::new(Self {
            config,
            storage,
            state,
            host_id,
            artifact_leases:Mutex::new(artifacts::ArtifactLeases::default()),
            creation_roots,
            skill_documents,
            terminal_jobs:Mutex::new(tokio::task::JoinSet::new()),
            terminal_slots:Arc::new(tokio::sync::Semaphore::new(shells::CREATION_JOBS as usize)),
            terminal_stopping:AtomicBool::new(false),
            terminal_safety,
            terminal_unproved_servers,
            inner: Mutex::new(Inner {
                servers: HashMap::new(),
                bindings: HashMap::new(),
                shell_bindings: HashMap::new(),
                proofs: Vec::new(),
                unbound_ids: HashMap::new(),
                unbound_reasons: HashMap::new(),
            }),
            updates: tokio::sync::Mutex::new(()),
            omp_creation_reason: Mutex::new(Some(
                "OMP creation unavailable: companion-owned loopback relay is not ready".into(),
            )),
            codex_creation_reason: Mutex::new(Some(
                "Codex creation unavailable: independent owner/service/source prerequisites unproved".into(),
            )),
            omp_offline_reporter: Arc::new(Mutex::new(None)),
            codex_invalidation_reporter: Arc::new(Mutex::new(None)),
        });
        for server in servers {
            // Existing pre-topology test/caller slots need not contain a runtime path.
            let Ok((path, _)): Result<(PathBuf, String), _> = serde_json::from_str(&server.slot)
            else {
                continue;
            };
            let name = server.name.ok_or(ErrorCode::NotReady)?;
            owner.inner.lock().expect("topology mutex").servers.insert(
                server.server_id.clone(),
                ServerRecord {
                    slot: server.slot,
                    name,
                    path,
                    server_id: server.server_id,
                    source: None,
                    reason: Some("Herdr source has not been revalidated".into()),
                    coarse_shell: None,
                },
            );
        }
        for persisted in sessions {
            if persisted.key.host_id != owner.host_id {
                return Err(ErrorCode::NotReady);
            }
            if let Some(display) = persisted.display {
                let source_epoch = owner.state.begin_install(display.clone())?;
                owner.state.finish_install(
                    &source_epoch,
                    ProviderSnapshot {
                        state: display,
                        history: vec![],
                        pending: vec![],
                        older_available: false,
                        native_older: None,
                    },
                )?;
                owner.inner.lock().expect("topology mutex").bindings.insert(
                    persisted.target.session_id.clone(),
                    Binding {
                        key: persisted.key,
                        target: persisted.target,
                        source_epoch,
                        herdr_report: None,
                        terminal_read_pin: None,
                    },
                );
            }
        }
        owner.restore_shells(shells)?;
        // One weak maintenance owner bounds idle metadata retention without a
        // Topology -> provider Runtime -> Topology lifetime cycle.
        let artifact_owner=Arc::downgrade(&owner);
        tokio::spawn(async move{loop{tokio::time::sleep(Duration::from_secs(1)).await;let Some(owner)=artifact_owner.upgrade()else{break};owner.artifact_leases.lock().expect("artifact leases").maintain();owner.state.prune_creation_directories();drop(owner);}});
        owner.publish()?;
        Ok(owner)
    }
    /// The owned diagnostic is installed once by the Codex runtime. It has no topology effect.
    pub(crate) fn set_codex_invalidation_reporter(&self, reporter: CodexInvalidationReporter) {
        if let Ok(mut current) = self.codex_invalidation_reporter.lock()
            && current.is_none()
        {
            *current = Some(reporter);
        }
    }
    /// The OMP runtime installs this one-run reporter before topology startup.
    /// Only Backend reports coarse settlement; Offline awaits one Ready target.
    pub(crate) fn set_omp_offline_reporter(&self, reporter: OmpCreationReporter) {
        if let Ok(mut current) = self.omp_offline_reporter.lock()
            && current.is_none()
        {
            *current = Some(reporter);
        }
    }
    fn prepare_omp_offline_reporter(
        &self,
        target: &Target,
        epoch: &Uuid,
        source: &Source,
        terminal_id: &Id,
    ) {
        if let Ok(current) = self.omp_offline_reporter.lock()
            && let Some(reporter) = current.as_ref()
        {
            reporter.prepare_offline(target, epoch, source, terminal_id);
        }
    }
    fn confirm_omp_offline_reporter(&self, target: &Target, epoch: &Uuid) {
        if let Ok(current) = self.omp_offline_reporter.lock()
            && let Some(reporter) = current.as_ref()
        {
            reporter.confirm_offline(target, epoch);
        }
    }
    fn report_omp_recovery_failure(&self, server: &Uuid, failure: OmpOfflineFailure) {
        let reporter = self
            .omp_offline_reporter
            .lock()
            .ok()
            .and_then(|current| current.clone());
        let bindings = if let Ok(inner) = self.inner.lock() {
            inner
                .bindings
                .values()
                .filter(|binding| binding.key.server_id == *server)
                .map(|binding| {
                    (
                        binding.target.clone(),
                        binding.source_epoch.clone(),
                        binding.key.clone(),
                    )
                })
                .collect()
        } else {
            Vec::new()
        };
        if let Some(reporter) = reporter {
            for (target, epoch, key) in bindings {
                reporter.offline_binding(&target, &epoch, &key, failure);
            }
        }
    }
    fn report_omp_recovery_io(&self, server: &Uuid, guard: &'static str, error: &io::Error) {
        let code = if error.kind() == io::ErrorKind::OutOfMemory {
            ErrorCode::ResourceLimit
        } else {
            ErrorCode::NotReady
        };
        self.report_omp_recovery_failure(
            server,
            OmpOfflineFailure::new("herdr", guard, code, codex_invalidation_io(error).0),
        );
    }
    pub(crate) fn arm_codex_invalidation_reporter(
        &self,
        target: &Target,
        epoch: &Uuid,
        server: &Uuid,
        evidence: &ServerEvidence,
    ) {
        if let Ok(current) = self.codex_invalidation_reporter.lock()
            && let Some(reporter) = current.as_ref()
        {
            reporter.arm(target, epoch, server, evidence);
        }
    }
    fn report_codex_shell_probe(
        &self,
        provider: Provider,
        guard: &'static str,
        code: ErrorCode,
        io: &'static str,
    ) {
        if provider != Provider::Codex {
            return;
        }
        let reporter = self
            .codex_invalidation_reporter
            .lock()
            .ok()
            .and_then(|current| current.clone());
        if let Some(reporter) = reporter {
            reporter.shell_probe_prebinding(guard, code, io);
        }
    }
    fn report_codex_shell_probe_error(
        &self,
        provider: Provider,
        guard: &'static str,
        code: ErrorCode,
        error: &io::Error,
    ) {
        if provider != Provider::Codex {
            return;
        }
        let reporter = self
            .codex_invalidation_reporter
            .lock()
            .ok()
            .and_then(|current| current.clone());
        if let Some(reporter) = reporter {
            reporter.shell_probe_prebinding_error(guard, code, error);
        }
    }
    pub fn source(&self, server: &Uuid) -> SecurityResult<Source> {
        self.source_readiness(server)?.ok_or(ErrorCode::NotReady)
    }
    pub(crate) fn source_readiness(&self, server: &Uuid) -> SecurityResult<Option<Source>> {
        let inner = self.inner.lock().expect("topology mutex");
        let source = inner
            .servers
            .get(server)
            .ok_or(ErrorCode::NotFound)?
            .source
            .clone();
        let Some(source) = source else {
            return Ok(None);
        };
        if source.recheck_readiness().map_err(policy)? {
            Ok(Some(source))
        } else {
            Ok(None)
        }
    }
    pub fn sources(&self) -> Vec<Source> {
        self.inner
            .lock()
            .expect("topology mutex")
            .servers
            .values()
            .filter_map(|s| s.source.clone())
            .filter(|s| s.recheck().is_ok())
            .collect()
    }


    fn logical_rename_context(&self,target:&Target)->SecurityResult<(Source,RenameContext,SessionSummary)> {
        let (server,incarnation,terminal,epoch)={
            let inner=self.inner.lock().expect("topology mutex");
            let binding=inner.bindings.get(&target.session_id).filter(|binding|binding.target==*target).ok_or(ErrorCode::StaleSession)?;
            (binding.key.server_id.clone(),binding.key.server_incarnation.clone(),binding.key.terminal_id.clone(),binding.source_epoch.clone())
        };
        let source=self.source(&server)?;
        if source.incarnation!=incarnation{return Err(ErrorCode::StaleSession)}
        let state=self.state.session_state(target,&epoch)?;
        if state.summary.connectivity!=SessionConnectivity::Ready{return Err(ErrorCode::NotReady)}
        let pane=source.snapshot.pane(&state.summary.pane_id).map_err(policy)?;
        if state.summary.server_id!=server||pane.terminal_id!=terminal
            ||pane.terminal_id!=state.summary.terminal_id||pane.workspace_id!=state.summary.workspace_id {
            return Err(ErrorCode::StaleSession);
        }
        let context=RenameContext{source_epoch:epoch,pane:pane.pane_id.clone(),terminal:pane.terminal_id.clone(),
            workspace:pane.workspace_id.clone(),tab:pane.tab_id.clone()};
        source.recheck().map_err(policy)?;
        Ok((source,context,state.summary))
    }
    fn validate_logical_rename_context(&self,target:&Target,original:&Source,context:&RenameContext,ack:bool)->SecurityResult<(Source,SessionSummary)> {
        original.recheck_identity().map_err(policy)?;
        let source={
            let inner=self.inner.lock().expect("topology mutex");
            let binding=inner.bindings.get(&target.session_id).filter(|binding| {
                binding.target==*target&&binding.source_epoch==context.source_epoch
                    &&binding.key.server_id==original.server_id&&binding.key.server_incarnation==original.incarnation
                    &&binding.key.terminal_id==context.terminal
            }).ok_or(ErrorCode::StaleSession)?;
            let current=inner.servers.get(&binding.key.server_id).and_then(|record|record.source.as_ref()).ok_or(ErrorCode::NotReady)?;
            if !original.same_physical_identity(current){return Err(ErrorCode::StaleSession)}
            current.clone()
        };
        let state=self.state.session_state(target,&context.source_epoch)?;
        let pane=source.snapshot.pane(&context.pane).map_err(policy)?;
        if state.summary.pane_id!=context.pane||state.summary.terminal_id!=context.terminal
            ||state.summary.workspace_id!=context.workspace||state.summary.server_id!=source.server_id
            ||pane.terminal_id!=context.terminal||pane.workspace_id!=context.workspace||pane.tab_id!=context.tab {
            return Err(ErrorCode::StaleSession);
        }
        if !ack {
            original.recheck().map_err(policy)?;
            source.recheck().map_err(policy)?;
            if state.summary.connectivity!=SessionConnectivity::Ready{return Err(ErrorCode::NotReady)}
        }
        Ok((source,state.summary))
    }
    pub(crate) async fn workspace_rename_context(&self,target:&Target)->SecurityResult<(Source,RenameContext)> {
        let _update=self.updates.lock().await;
        let (source,context,summary)=self.logical_rename_context(target)?;
        if !summary.capabilities.workspace_rename{return Err(ErrorCode::NotReady)}
        Ok((source,context))
    }
    pub(crate) fn validate_workspace_rename_context_now(&self,target:&Target,source:&Source,context:&RenameContext)->SecurityResult<()> {
        let _update=self.updates.try_lock().map_err(|_|ErrorCode::Busy)?;
        let (_,summary)=self.validate_logical_rename_context(target,source,context,false)?;
        if !summary.capabilities.workspace_rename{return Err(ErrorCode::NotReady)}
        Ok(())
    }

    pub fn creation_reason(
        &self,
        server: &Uuid,
        provider: Provider,
        project: Option<&Uuid>,
    ) -> String {
        match self.config.backend_reason(provider) {
            Ok(Some(reason)) => return reason.into(),
            Err(_) => return "Configured backend prerequisites are unsafe or unavailable".into(),
            Ok(None) => {}
        }
        let Ok(source) = self.source(server) else {
            return "Herdr process/socket/namespace evidence is unavailable".into();
        };
        if source.snapshot.no_focus().is_err() {
            return "Herdr has no active workspace; focus:false cannot guarantee no desktop focus change".into();
        }
        if !self.config.allow_scoped_shell_probe {
            return "Creation disabled: explicit local allow_scoped_shell_probe consent is absent"
                .into();
        }
        let inner = self.inner.lock().expect("topology mutex");
        let Some(coarse) = inner
            .servers
            .get(server)
            .and_then(|r| r.coarse_shell.as_ref())
        else {
            return "Creation disabled: actual Herdr shell/source proof for GNU Bash5.3p9 (bash or bash-interactive) or zsh5.9/5.9.1, reporting namespace and integration views are unproved".into();
        };
        let available = self
            .config
            .projects
            .iter()
            .filter(|p| project.is_none_or(|id| &p.id == id))
            .any(|p| {
                self.creation_roots.get(&p.id).is_some_and(Option::is_some)
                    && self.validate_coarse(&source, coarse, provider, &p.id).is_ok()
            });
        if !available {
            return "Creation disabled: configured project/wrapper/runtime object views or shell epoch are unavailable".into();
        }
        if provider == Provider::Omp {
            self.omp_creation_reason
                .lock()
                .expect("OMP capability mutex")
                .clone()
                .unwrap_or_default()
        } else {
            self.codex_creation_reason
                .lock()
                .expect("Codex capability mutex")
                .clone()
                .unwrap_or_default()
        }
    }
    pub fn omp_availability(&self, reason: Option<&str>) -> SecurityResult<()> {
        *self
            .omp_creation_reason
            .lock()
            .expect("OMP capability mutex") = reason.map(str::to_owned);
        self.publish()
    }
    pub fn codex_availability(&self, reason: Option<&str>) -> SecurityResult<()> {
        *self
            .codex_creation_reason
            .lock()
            .expect("Codex capability mutex") = reason.map(str::to_owned);
        self.publish()
    }
    pub fn unbound_reason(
        &self,
        source: &Source,
        terminal: &Id,
        provider: Provider,
        state: UnboundState,
        reason: &str,
    ) -> SecurityResult<()> {
        source.recheck().map_err(policy)?;
        let key = (
            source.server_id.clone(),
            source.incarnation.clone(),
            terminal.clone(),
            provider,
        );
        {
            let mut inner = self.inner.lock().expect("topology mutex");
            if inner.unbound_reasons.len() >= 512 && !inner.unbound_reasons.contains_key(&key) {
                return Err(ErrorCode::ResourceLimit);
            }
            inner
                .unbound_reasons
                .insert(key, (state, reason.to_owned()));
        }
        self.publish()
    }
    pub fn validate_create(&self, request: &Request) -> SecurityResult<()> {
        let RequestBody::SessionCreate(params) = &request.body else {
            return Err(ErrorCode::InvalidRequest);
        };
        self.config.project(&params.project_id)?;
        self.validate_placement(params)?;
        let source = self.source(&params.server_id)?;
        self.validate_topology_stage(&source, params, None)
    }
    fn recheck_wrapper(&self, source: &Source, proof: &WrapperProof) -> io::Result<bool> {
        if !source.recheck_readiness()? {
            return Ok(false);
        }
        if self.config.backend_reason(proof.provider)?.is_some() {
            return Err(invalid("configured backend prerequisite is unavailable"));
        }
        if process_evidence(source.evidence.pid)? != proof.reporter {
            return Err(invalid(
                "Herdr reporting namespace/process evidence changed",
            ));
        }
        if proof.server_incarnation != source.incarnation
            || process_evidence(proof.shell.pid)? != proof.shell
            || self.config.wrapper(proof.provider) != Some(proof.wrapper.as_path())
            || self
                .config
                .project(&proof.project_id)
                .map_err(|_| invalid("project unavailable"))?
                .path
                != proof.authorization_project
        {
            return Err(invalid("wrapper proof context changed"));
        }
        source.snapshot.pane(&proof.pane_id)?;
        config::executable(&proof.wrapper)?;
        if object(&proof.wrapper)? != proof.wrapper_object
            || wrapper_stamp(&proof.wrapper)? != proof.wrapper_stamp
            || process_object(&proof.shell, &proof.wrapper)? != proof.wrapper_object
            || object(&self.config.runtime_dir)? != proof.runtime_object
            || process_object(&proof.shell, &self.config.runtime_dir_agent)? != proof.runtime_object
            || object(&proof.project)? != proof.project_object
            || process_object(&proof.shell, &proof.project)? != proof.project_object
        {
            return Err(invalid("host/provider object views disagree"));
        }
        if proof.provider == Provider::Codex {
            let owner = self
                .config
                .creation_codex_owner()
                .map_err(|_| invalid("default shared Codex owner selection is unavailable"))?;
            let host_parent = owner
                .socket_host
                .parent()
                .ok_or_else(|| invalid("owner socket parent absent"))?;
            let agent_parent = owner
                .socket_agent
                .parent()
                .ok_or_else(|| invalid("provider owner socket parent absent"))?;
            let bindings_agent = owner
                .bindings_dir
                .strip_prefix(&self.config.runtime_dir)
                .map(|relative| self.config.runtime_dir_agent.join(relative))
                .unwrap_or_else(|_| owner.bindings_dir.clone());
            if object(host_parent)? != process_object(&proof.shell, agent_parent)?
                || object(&owner.bindings_dir)? != process_object(&proof.shell, &bindings_agent)?
            {
                return Err(invalid("Codex owner/binding host/provider views disagree"));
            }
        }
        if process_evidence(proof.shell.pid)? != proof.shell
            || process_evidence(source.evidence.pid)? != proof.reporter
        {
            return Err(invalid(
                "process/namespace evidence changed during object-view proof",
            ));
        }
        source.recheck_readiness()
    }
    pub fn record_shell_measurement(
        &self,
        server: &Uuid,
        provider: Provider,
        measurement: ShellMeasurement,
        process: &PaneProcessInfo,
        reporter: &ProcessEvidence,
    ) -> SecurityResult<()> {
        let source = self.source(server)?;
        if measurement.server_incarnation != source.incarnation {
            return Err(ErrorCode::StaleSession);
        }
        let pane = source.snapshot.pane(&measurement.pane_id).map_err(policy)?;
        if process.pane_id != pane.pane_id
            || process_evidence(source.evidence.pid).map_err(policy)? != *reporter
        {
            return Err(ErrorCode::StaleSession);
        }
        if !process
            .shell_pid
            .is_some_and(|pid| reported_pid_matches(reporter, &measurement.shell, pid))
            || process_evidence(measurement.shell.pid).map_err(policy)? != measurement.shell
        {
            return Err(ErrorCode::NotReady);
        }
        let wrapper = self.config.wrapper(provider).ok_or(ErrorCode::NotReady)?;
        if wrapper != measurement.resolved_outer_wrapper {
            return Err(ErrorCode::NotReady);
        }
        let project = self.config.project(&measurement.project_id)?;
        let execution_directory=measurement.execution_directory.clone();
        let proof = WrapperProof {
            action_id: measurement.action_id,
            server_incarnation: source.incarnation.clone(),
            provider,
            reporter: reporter.clone(),
            shell: measurement.shell,
            pane_id: measurement.pane_id,
            wrapper: wrapper.to_owned(),
            project: measurement.execution_directory,
            authorization_project: project.path.clone(),
            project_id: project.id.clone(),
            wrapper_object: object(wrapper).map_err(policy)?,
            wrapper_stamp: wrapper_stamp(wrapper).map_err(policy)?,
            runtime_object: object(&self.config.runtime_dir).map_err(policy)?,
            project_object: object(&execution_directory).map_err(policy)?,
        };
        if !self.recheck_wrapper(&source, &proof).map_err(policy)? {
            return Err(ErrorCode::NotReady);
        }
        let mut inner = self.inner.lock().expect("topology mutex");
        inner.proofs.retain(|p| {
            !(p.server_incarnation == proof.server_incarnation
                && p.provider == provider
                && p.project_id == proof.project_id
                && p.pane_id == proof.pane_id
                && p.action_id == proof.action_id)
        });
        if inner.proofs.len() >= 64 * 128 * 2 {
            return Err(ErrorCode::ResourceLimit);
        }
        inner.proofs.push(proof);
        Ok(())
    }
    fn validate_topology_stage(
        &self,
        source: &Source,
        params: &SessionCreateParams,
        root: Option<(&CreationResources, &Uuid)>,
    ) -> SecurityResult<()> {
        self.validate_topology_stage_classified(source, params, root)
            .map_err(|(code, _, _)| code)
    }
    /// Classifies the existing predicate sequence without adding a topology read.
    fn validate_topology_stage_classified(
        &self,
        source: &Source,
        params: &SessionCreateParams,
        root: Option<(&CreationResources, &Uuid)>,
    ) -> Result<(), (ErrorCode, &'static str, &'static str)> {
        self.validate_placement(params).map_err(|code|(code,"creation_placement","none"))?;
        let current = self
            .source(&params.server_id)
            .map_err(|code| (code, "topology_source", "none"))?;
        if current.incarnation != source.incarnation || current.evidence != source.evidence {
            return Err((ErrorCode::StaleSession, "topology_source_changed", "none"));
        }
        self.config
            .project(&params.project_id)
            .map_err(|code| (code, "topology_project", "none"))?;
        current.snapshot.no_focus().map_err(|error| {
            let io = codex_invalidation_io(&error).0;
            (policy(error), "topology_focus", io)
        })?;
        if let Some((resources, action)) = root {
            let inner = self.inner.lock().expect("topology mutex");
            let proof = inner
                .proofs
                .iter()
                .find(|p| {
                    p.server_incarnation == current.incarnation
                        && p.provider == params.provider
                        && p.project_id == params.project_id
                        && &p.action_id == action
                        && resources.pane_id.as_ref() == Some(&p.pane_id)
                })
                .ok_or((ErrorCode::NotReady, "measurement_absent", "none"))?;
            if !self.recheck_wrapper(&current, proof).map_err(|error| {
                let io = codex_invalidation_io(&error).0;
                (policy(error), "wrapper_recheck_failure", io)
            })? {
                return Err((ErrorCode::NotReady, "wrapper_recheck_mismatch", "none"));
            }
            let pane = current
                .snapshot
                .pane(resources.pane_id.as_ref().ok_or((ErrorCode::NotReady, "pane_absent", "none"))?)
                .map_err(|error| {
                    let io = codex_invalidation_io(&error).0;
                    (policy(error), "pane_absent", io)
                })?;
            if proof.pane_id != pane.pane_id {
                return Err((ErrorCode::NotReady, "pane_mismatch", "none"));
            }
            if resources.workspace_id.as_ref() != Some(&pane.workspace_id)
                || resources.terminal_id.as_ref() != Some(&pane.terminal_id)
            {
                return Err((ErrorCode::StaleSession, "resource_mismatch", "none"));
            }
        } else {
            if !self.config.allow_scoped_shell_probe {
                return Err((ErrorCode::NotReady, "shell_probe_disabled", "none"));
            }
            let inner = self.inner.lock().expect("topology mutex");
            let coarse = inner
                .servers
                .get(&params.server_id)
                .and_then(|r| r.coarse_shell.as_ref())
                .ok_or((ErrorCode::NotReady, "coarse_measurement_absent", "none"))?;
            self.validate_coarse(&current, coarse, params.provider, &params.project_id)
                .map_err(|code| (code, "coarse_measurement_invalid", "none"))?;
        }
        Ok(())
    }
    pub fn validate_provider_stage(
        &self,
        action: &Uuid,
        params: &SessionCreateParams,
        resources: &CreationResources,
    ) -> SecurityResult<()> {
        let source = self.source(&params.server_id)?;
        self.validate_topology_stage(&source, params, Some((resources, action)))
    }
    /// Readiness reuses this action's measured root without requiring the now-running TUI
    /// to be an idle shell. It cannot admit another provider launch or borrow old-pane proof.
    pub fn validate_created_binding(
        &self,
        action: &Uuid,
        params: &SessionCreateParams,
        resources: &CreationResources,
    ) -> SecurityResult<()> {
        if self.created_binding_readiness(action, params, resources)? {
            Ok(())
        } else {
            Err(ErrorCode::NotReady)
        }
    }
    pub(crate) fn created_binding_readiness(
        &self,
        action: &Uuid,
        params: &SessionCreateParams,
        resources: &CreationResources,
    ) -> SecurityResult<bool> {
        let Some(source) = self.source_readiness(&params.server_id)? else {
            return Ok(false);
        };
        let pane = resources.pane_id.as_ref().ok_or(ErrorCode::NotReady)?;
        let current = source.snapshot.pane(pane).map_err(policy)?;
        if resources.workspace_id.as_ref() != Some(&current.workspace_id)
            || resources.terminal_id.as_ref() != Some(&current.terminal_id)
            || current.focused
        {
            return Err(ErrorCode::StaleSession);
        }
        let inner = self.inner.lock().expect("topology mutex");
        let proof = inner
            .proofs
            .iter()
            .find(|p| {
                &p.action_id == action
                    && &p.pane_id == pane
                    && p.server_incarnation == source.incarnation
                    && p.provider == params.provider
                    && p.project_id == params.project_id
            })
            .ok_or(ErrorCode::NotReady)?;
        self.recheck_wrapper(&source, proof).map_err(policy)
    }
    fn validate_coarse(
        &self,
        source: &Source,
        coarse: &CoarseShell,
        provider: Provider,
        project: &Uuid,
    ) -> SecurityResult<()> {
        if !self.config.allow_scoped_shell_probe
            || coarse.incarnation != source.incarnation
            || process_evidence(source.evidence.pid).map_err(policy)? != coarse.reporter
            || process_evidence(coarse.shell.pid).map_err(policy)? != coarse.shell
            || shell_executable(&coarse.shell).map_err(policy)?
                != (coarse.executable.clone(), coarse.dialect)
            || self
                .config
                .backend_reason(provider)
                .map_err(policy)?
                .is_some()
        {
            return Err(ErrorCode::NotReady);
        }
        let wrapper = self.config.wrapper(provider).ok_or(ErrorCode::NotReady)?;
        let project = self.config.project(project)?;
        if process_object(&coarse.shell, wrapper).map_err(policy)?
            != object(wrapper).map_err(policy)?
            || process_object(&coarse.shell, &self.config.runtime_dir_agent).map_err(policy)?
                != object(&self.config.runtime_dir).map_err(policy)?
            || process_object(&coarse.shell, &project.path).map_err(policy)?
                != object(&project.path).map_err(policy)?
        {
            return Err(ErrorCode::NotReady);
        }
        source.recheck().map_err(policy)
    }
    async fn refresh_coarse(&self, server: &Uuid, revision: u64, invalidation: &Arc<AtomicU64>) {
        // Only the helper's one explicitly configured socket owns this settlement.
        // Other session-list discoveries must not consume its first-observation latch.
        let selected_capture_source = self.config.herdr_sockets.len() == 1
            && self.inner.lock().ok().and_then(|inner| inner.servers.get(server).map(|record| record.path == self.config.herdr_sockets[0])).unwrap_or(false);
        let capture = if selected_capture_source {
            self.omp_offline_reporter.lock().ok().and_then(|current| current.clone())
        } else {
            None
        };
        let report = |guard, code, io| {
            if let Some(capture) = &capture {
                capture.report_coarse_shell(guard, code, io);
            }
        };
        if !self.config.allow_scoped_shell_probe {
            report(CoarseShellGuard::ConsentAbsent, Some(ErrorCode::NotReady), "none");
            return;
        }
        // The existing authoritative recovery cadence may heal a failed coarse probe.
        // A retained proof needs no additional native calls on an unchanged observation.
        if self.inner.lock().expect("topology mutex").servers.get(server)
            .is_some_and(|record| record.coarse_shell.is_some())
        {
            return;
        }
        let source = match self.source(server) {
            Ok(source) => source,
            Err(code) => {
                report(CoarseShellGuard::SourceLookup, Some(code), "none");
                return;
            }
        };
        if source.revision != revision || !Arc::ptr_eq(&source.invalidation, invalidation) {
            return;
        }
        let reporter = match process_evidence(source.evidence.pid) {
            Ok(reporter) => reporter,
            Err(error) => {
                let failure = CoarseShellFailure::io(CoarseShellGuard::ReporterProcess, &error);
                report(failure.guard, Some(failure.code), failure.io);
                return;
            }
        };
        let mut found = None;
        // Keep only the first closed rejection until all existing candidate attempts settle.
        // A later accepted pane makes earlier candidate rejections irrelevant.
        let mut rejected = None;
        for pane in source.snapshot.panes.iter().take(8) {
            let client = match Client::connect(&source.socket_path).await {
                Ok(client) => client,
                Err(error) => {
                    rejected.get_or_insert(CoarseShellFailure::io(CoarseShellGuard::Connect, &error));
                    continue;
                }
            };
            if client.evidence != source.evidence {
                rejected.get_or_insert(CoarseShellFailure::new(CoarseShellGuard::PeerIdentity));
                break;
            }
            let info = match client.process_info(&pane.pane_id).await {
                Ok(info) => info,
                Err(error) => {
                    rejected.get_or_insert(CoarseShellFailure::io(CoarseShellGuard::ProcessInfo, &error));
                    continue;
                }
            };
            let Some(pid) = info.shell_pid else {
                rejected.get_or_insert(CoarseShellFailure::new(CoarseShellGuard::ShellAbsent));
                continue;
            };
            let shell = match reported_process(&reporter, pid) {
                Ok(shell) => shell,
                Err(error) => {
                    rejected.get_or_insert(CoarseShellFailure::io(CoarseShellGuard::ReportedProcess, &error));
                    continue;
                }
            };
            let (executable, dialect) = match shell_executable(&shell) {
                Ok(executable) => executable,
                Err(error) => {
                    rejected.get_or_insert(CoarseShellFailure::io(CoarseShellGuard::ShellExecutable, &error));
                    continue;
                }
            };
            found = Some(CoarseShell {
                shell,
                reporter: reporter.clone(),
                executable,
                dialect,
                incarnation: source.incarnation.clone(),
            });
            break;
        }
        // Native I/O stays outside the owner lock. Commit only into this exact captured
        // observation; an equal replacement can have a different recovery owner/snapshot.
        let _update = self.updates.lock().await;
        if let Err(error) = source.recheck() {
            let failure = CoarseShellFailure::io(CoarseShellGuard::SourceRecheck, &error);
            report(failure.guard, Some(failure.code), failure.io);
            return;
        }
        let installed = {
            let mut inner = self.inner.lock().expect("topology mutex");
            if let Some(record) = inner.servers.get_mut(server) {
                let current = record.source.as_ref();
                if !source.revision_current() || record.coarse_shell.is_some()
                    || !current.is_some_and(|current| {
                        source.same_physical_identity(current)
                            && current.revision == source.revision
                            && Arc::ptr_eq(&current.snapshot, &source.snapshot)
                    })
                {
                    return;
                }
                let installed = found.is_some();
                record.coarse_shell = found;
                Some(installed)
            } else {
                None
            }
        };
        match installed {
            Some(true) => report(CoarseShellGuard::Installed, None, "none"),
            Some(false) => {
                let failure = rejected.unwrap_or_else(|| CoarseShellFailure::new(CoarseShellGuard::NoCandidate));
                report(failure.guard, Some(failure.code), failure.io);
            }
            None => report(CoarseShellGuard::RecordAbsent, Some(ErrorCode::NotReady), "none"),
        }
        let _ = self.publish();
    }
    fn fresh_creation_root(
        &self,
        original: &Source,
        resources: &CreationResources,
        focus: &Id,
    ) -> SecurityResult<Source> {
        self.fresh_creation_root_classified(original, resources, focus)
            .map_err(|(code, _)| code)
    }
    fn fresh_creation_root_classified(
        &self,
        original: &Source,
        resources: &CreationResources,
        focus: &Id,
    ) -> Result<Source, (ErrorCode, Option<CreationRootGuard>)> {
        // Durable intent may outlive an unrelated topology revision. Revalidate
        // this same source/root, never authorize the stale snapshot or a replacement.
        let current = self
            .source(&original.server_id)
            .map_err(|code| (code, None))?;
        let guard = if current.server_id != original.server_id {
            Some(CreationRootGuard::ServerId)
        } else if current.incarnation != original.incarnation {
            Some(CreationRootGuard::Incarnation)
        } else if current.evidence != original.evidence {
            Some(CreationRootGuard::Evidence)
        } else if current.socket_path != original.socket_path {
            Some(CreationRootGuard::SocketPath)
        } else if !Arc::ptr_eq(&current.invalidation, &original.invalidation) {
            Some(CreationRootGuard::Invalidation)
        } else {
            None
        };
        if let Some(guard) = guard {
            return Err((ErrorCode::StaleSession, Some(guard)));
        }
        self.fresh_probe_root(&current, resources, focus)
            .map_err(|code| (code, None))?;
        Ok(current)
    }
    fn fresh_probe_root<'a>(
        &self,
        source: &'a Source,
        resources: &CreationResources,
        focus: &Id,
    ) -> SecurityResult<&'a Pane> {
        self.fresh_probe_root_classified(source, resources, focus)
            .map_err(|(code, _, _)| code)
    }
    fn fresh_probe_root_classified<'a>(
        &self,
        source: &'a Source,
        resources: &CreationResources,
        focus: &Id,
    ) -> Result<&'a Pane, (ErrorCode, &'static str, ProbeRootGuard)> {
        source.recheck().map_err(|error| {
            let io = codex_invalidation_io(&error).0;
            (policy(error), io, ProbeRootGuard::SourceRecheck)
        })?;
        let pane = source
            .snapshot
            .pane(resources.pane_id.as_ref().ok_or((
                ErrorCode::NotReady,
                "none",
                ProbeRootGuard::Pane,
            ))?)
            .map_err(|error| {
                let io = codex_invalidation_io(&error).0;
                (policy(error), io, ProbeRootGuard::Pane)
            })?;
        if resources.workspace_id.as_ref() != Some(&pane.workspace_id) {
            return Err((ErrorCode::NotReady, "none", ProbeRootGuard::Workspace));
        }
        if resources.terminal_id.as_ref() != Some(&pane.terminal_id) {
            return Err((ErrorCode::NotReady, "none", ProbeRootGuard::Terminal));
        }
        if source.snapshot.focused_workspace_id.as_ref() != Some(focus) {
            return Err((ErrorCode::NotReady, "none", ProbeRootGuard::Focus));
        }
        if pane.focused {
            return Err((ErrorCode::NotReady, "none", ProbeRootGuard::PaneFocused));
        }
        if pane.agent.is_some() {
            return Err((ErrorCode::NotReady, "none", ProbeRootGuard::Agent));
        }
        if pane.agent_session.is_some() {
            return Err((ErrorCode::NotReady, "none", ProbeRootGuard::AgentSession));
        }
        if matches!(
            pane.agent_status,
            NativeAgentStatus::Working | NativeAgentStatus::Blocked
        ) {
            return Err((ErrorCode::NotReady, "none", ProbeRootGuard::AgentStatus));
        }
        if source.snapshot.agents.iter().any(|a| {
            a.terminal_id == pane.terminal_id
                && (a.name.is_some()
                    || a.agent.is_some()
                    || a.agent_session.is_some()
                    || a.launch_pending
                    || a.interactive_ready)
        }) {
            return Err((ErrorCode::NotReady, "none", ProbeRootGuard::Agents));
        }
        Ok(pane)
    }
    fn idle_probe_shell_classified(
        &self,
        source: &Source,
        info: &PaneProcessInfo,
    ) -> ProbeShellCheck {
        let reporter = process_evidence(source.evidence.pid).map_err(|error| {
            let io = codex_invalidation_io(&error).0;
            (policy(error), io, ProbeShellGuard::Reporter)
        })?;
        let pid =
            info.shell_pid
                .ok_or((ErrorCode::NotReady, "none", ProbeShellGuard::ShellAbsent))?;
        if info.foreground_process_group_id != Some(pid)
            || info.foreground_processes.is_empty()
            || info.foreground_processes.iter().any(|p| p.pid != pid)
        {
            return Err((ErrorCode::NotReady, "none", ProbeShellGuard::ShellNotIdle));
        }
        let shell = reported_process(&reporter, pid).map_err(|error| {
            let io = codex_invalidation_io(&error).0;
            (policy(error), io, ProbeShellGuard::ShellProcess)
        })?;
        let (executable, dialect) = shell_executable(&shell).map_err(|error| {
            let io = codex_invalidation_io(&error).0;
            (policy(error), io, ProbeShellGuard::ShellExecutable)
        })?;
        if process_evidence(reporter.pid).map_err(|error| {
            let io = codex_invalidation_io(&error).0;
            (policy(error), io, ProbeShellGuard::ReporterRecheck)
        })? != reporter
        {
            return Err((
                ErrorCode::NotReady,
                "none",
                ProbeShellGuard::ReporterRecheck,
            ));
        }
        Ok((shell, reporter, executable, dialect))
    }
    /// Exactly one fixed query in this receipt's new root; ACK is enqueue, not execution.
    pub(crate) async fn shell_probe_once(
        &self,
        security:&Security,
        auth:&Authorized,
        prepared:PreparedShell<'_>,
    )->SecurityResult<Receipt> {
        self.shell_probe_once_reported(security,auth,prepared,None).await
    }
    /// The reported form is used only by the already-authorized OMP creation job.  It observes
    /// existing pre-intent failures and maps them back to the exact public result.
    pub(crate) async fn prepare_created_shell<'a>(
        &self,
        security:&Security,
        auth:&Authorized,
        (action,params,selection):(&'a Uuid,&'a SessionCreateParams,&'a CreationSelection),
        deadline:tokio::time::Instant,
        reporter:Option<&OmpCreationReporter>,
    )->SecurityResult<Option<PreparedShell<'a>>> {
        if tokio::time::Instant::now() >= deadline {
            if let Some(reporter) = reporter {
                reporter.report_shell_probe("deadline", ErrorCode::Timeout, "none");
            }
            return Err(ErrorCode::Timeout);
        }
        let current = self
            .receipt(security, auth, action)
            .await
            .inspect_err(|code| {
                if let Some(reporter) = reporter {
                    reporter.report_shell_probe("receipt", *code, "none");
                }
            })?;
        let Some(progress) = current.creation.as_ref() else {
            if let Some(reporter) = reporter {
                reporter.report_shell_probe("receipt_progress", ErrorCode::InvalidRequest, "none");
            }
            return Err(ErrorCode::InvalidRequest);
        };
        let proof = auth.proof.clone();
        let id = action.clone();
        let local = match self
            .storage
            .call(move |s| s.creation_local(&proof, &id))
            .await
        {
            Ok(Some(local)) => local,
            Ok(None) => {
                if let Some(reporter) = reporter {
                    reporter.report_shell_probe("local_absent", ErrorCode::NotFound, "none");
                }
                return Err(ErrorCode::NotFound);
            }
            Err(error) => {
                let code = pairing::store_error(error);
                if let Some(reporter) = reporter {
                    reporter.report_shell_probe("local_storage", code, "none");
                }
                return Err(code);
            }
        };
        if local
            .stages
            .iter()
            .any(|s| s.intent.workspace_operation == WorkspaceOperation::ShellProbe)
        {
            return Ok(None);
        }
        if current.state != ReceiptState::Dispatching
            || progress.stage != CreationStage::Workspace
            || progress.stage_state != StageState::Confirmed
            || progress.server_id != params.server_id
            || progress.project_id != params.project_id
            || progress.provider != params.provider
            || !self.config.allow_scoped_shell_probe
        {
            if let Some(reporter) = reporter {
                reporter.report_shell_probe("precondition", ErrorCode::NotReady, "none");
            }
            return Err(ErrorCode::NotReady);
        }
        let resources = progress.resources.clone();
        let Some(focus) = local
            .stages
            .first()
            .and_then(|s| s.intent.focused_workspace_id.clone())
        else {
            if let Some(reporter) = reporter {
                reporter.report_shell_probe("focus_missing", ErrorCode::NotReady, "none");
            }
            return Err(ErrorCode::NotReady);
        };
        let source_deadline = deadline.min(tokio::time::Instant::now() + Duration::from_secs(5));
        let source = loop {
            {
                let _guard = security.native_guard(auth).await.inspect_err(|code| {
                    if let Some(reporter) = reporter {
                        reporter.report_shell_probe("authorization", *code, "none");
                    }
                })?;
            }
            let observed = match self.source(&params.server_id) {
                Ok(source) => match self.fresh_probe_root_classified(&source, &resources, &focus) {
                    Ok(_) => Ok(source),
                    Err((_, io, guard)) => Err((guard.deadline_guard(), io)),
                },
                Err(_) => Err(("source_unavailable_at_deadline", "none")),
            };
            match observed {
                Ok(source) => break source,
                Err((guard, io)) if tokio::time::Instant::now() >= source_deadline => {
                    if let Some(reporter) = reporter {
                        reporter.report_shell_probe(guard, ErrorCode::NotReady, io);
                    }
                    return Err(ErrorCode::NotReady);
                }
                Err(_) => {}
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        };
        let pane = match self.fresh_probe_root_classified(&source, &resources, &focus) {
            Ok(pane) => pane,
            Err((code, io, guard)) => {
                if let Some(reporter) = reporter {
                    reporter.report_shell_probe(guard.immediate_guard(), code, io);
                }
                return Err(code);
            }
        };
        let client = match Client::connect(&source.socket_path).await {
            Ok(client) => client,
            Err(error) => {
                let io = codex_invalidation_io(&error).0;
                let code = policy(error);
                if let Some(reporter) = reporter {
                    reporter.report_shell_probe("source_connect", code, io);
                }
                return Err(code);
            }
        };
        if client.evidence != source.evidence {
            if let Some(reporter) = reporter {
                reporter.report_shell_probe("source_identity", ErrorCode::StaleSession, "none");
            }
            return Err(ErrorCode::StaleSession);
        }
        let info = match client.process_info(&pane.pane_id).await {
            Ok(info) => info,
            Err(error) => {
                let io = codex_invalidation_io(&error).0;
                let code = policy(error);
                if let Some(reporter) = reporter {
                    reporter.report_shell_probe("process_info", code, io);
                }
                return Err(code);
            }
        };
        let (shell, original_reporter, executable, dialect) =
            match self.idle_probe_shell_classified(&source, &info) {
                Ok(shell) => shell,
                Err((_, _, ProbeShellGuard::ShellNotIdle)) => {
                    let shell_reporter = match process_evidence(source.evidence.pid) {
                        Ok(value) => value,
                        Err(error) => {
                            let io = codex_invalidation_io(&error).0;
                            let code = policy(error);
                            if let Some(reporter) = reporter {
                                reporter.report_shell_probe("shell_reporter", code, io);
                            }
                            return Err(code);
                        }
                    };
                    let Some(pid) = info.shell_pid else {
                        if let Some(reporter) = reporter {
                            reporter.report_shell_probe("shell_absent", ErrorCode::NotReady, "none");
                        }
                        return Err(ErrorCode::NotReady);
                    };
                    let shell = match reported_process(&shell_reporter, pid) {
                        Ok(value) => value,
                        Err(error) => {
                            let io = codex_invalidation_io(&error).0;
                            let code = policy(error);
                            if let Some(reporter) = reporter {
                                reporter.report_shell_probe("shell_process", code, io);
                            }
                            return Err(code);
                        }
                    };
                    let (executable, dialect) = match shell_executable(&shell) {
                        Ok(value) => value,
                        Err(error) => {
                            let io = codex_invalidation_io(&error).0;
                            let code = policy(error);
                            if let Some(reporter) = reporter {
                                reporter.report_shell_probe("shell_executable", code, io);
                            }
                            return Err(code);
                        }
                    };
                    match process_evidence(shell_reporter.pid) {
                        Ok(current) if current == shell_reporter => {}
                        Ok(_) => {
                            if let Some(reporter) = reporter {
                                reporter.report_shell_probe("shell_reporter_recheck", ErrorCode::NotReady, "none");
                            }
                            return Err(ErrorCode::NotReady);
                        }
                        Err(error) => {
                            let io = codex_invalidation_io(&error).0;
                            let code = policy(error);
                            if let Some(reporter) = reporter {
                                reporter.report_shell_probe("shell_reporter_recheck", code, io);
                            }
                            return Err(code);
                        }
                    }
                    loop {
                        if tokio::time::Instant::now() >= deadline {
                            if let Some(reporter) = reporter {
                                reporter.report_shell_probe("shell_not_idle", ErrorCode::Timeout, "none");
                            }
                            return Err(ErrorCode::Timeout);
                        }
                        {
                            let _guard = security.native_guard(auth).await.inspect_err(|code| {
                                if let Some(reporter) = reporter {
                                    reporter.report_shell_probe("authorization", *code, "none");
                                }
                            })?;
                        }
                        if tokio::time::Instant::now() >= deadline {
                            if let Some(reporter) = reporter {
                                reporter.report_shell_probe("shell_not_idle", ErrorCode::Timeout, "none");
                            }
                            return Err(ErrorCode::Timeout);
                        }
                        let refreshed = match self.fresh_creation_root(&source, &resources, &focus) {
                            Ok(value) => value,
                            Err(code) => {
                                if let Some(reporter) = reporter {
                                    reporter.report_shell_probe("source_identity", code, "none");
                                }
                                return Err(code);
                            }
                        };
                        match process_evidence(shell.pid) {
                            Ok(current) if current == shell => {}
                            Ok(_) => {
                                if let Some(reporter) = reporter {
                                    reporter.report_shell_probe("shell_process", ErrorCode::NotReady, "none");
                                }
                                return Err(ErrorCode::NotReady);
                            }
                            Err(error) => {
                                let io = codex_invalidation_io(&error).0;
                                let code = policy(error);
                                if let Some(reporter) = reporter {
                                    reporter.report_shell_probe("shell_process", code, io);
                                }
                                return Err(code);
                            }
                        }
                        match shell_executable(&shell) {
                            Ok((current, current_dialect)) if current == executable && current_dialect == dialect => {}
                            Ok(_) => {
                                if let Some(reporter) = reporter {
                                    reporter.report_shell_probe("shell_executable", ErrorCode::NotReady, "none");
                                }
                                return Err(ErrorCode::NotReady);
                            }
                            Err(error) => {
                                let io = codex_invalidation_io(&error).0;
                                let code = policy(error);
                                if let Some(reporter) = reporter {
                                    reporter.report_shell_probe("shell_executable", code, io);
                                }
                                return Err(code);
                            }
                        }
                        if tokio::time::Instant::now() >= deadline {
                            if let Some(reporter) = reporter {
                                reporter.report_shell_probe("shell_not_idle", ErrorCode::Timeout, "none");
                            }
                            return Err(ErrorCode::Timeout);
                        }
                        let client = match tokio::time::timeout_at(
                            deadline,
                            Client::connect(&refreshed.socket_path),
                        )
                        .await
                        {
                            Ok(Ok(client)) => client,
                            Ok(Err(error)) => {
                                let io = codex_invalidation_io(&error).0;
                                let code = policy(error);
                                if let Some(reporter) = reporter {
                                    reporter.report_shell_probe("source_connect", code, io);
                                }
                                return Err(code);
                            }
                            Err(_) => {
                                if let Some(reporter) = reporter {
                                    reporter.report_shell_probe("shell_not_idle", ErrorCode::Timeout, "none");
                                }
                                return Err(ErrorCode::Timeout);
                            }
                        };
                        if client.evidence != source.evidence {
                            if let Some(reporter) = reporter {
                                reporter.report_shell_probe("source_identity", ErrorCode::StaleSession, "none");
                            }
                            return Err(ErrorCode::StaleSession);
                        }
                        if tokio::time::Instant::now() >= deadline {
                            if let Some(reporter) = reporter {
                                reporter.report_shell_probe("shell_not_idle", ErrorCode::Timeout, "none");
                            }
                            return Err(ErrorCode::Timeout);
                        }
                        let observed = match tokio::time::timeout_at(
                            deadline,
                            client.process_info(&pane.pane_id),
                        )
                        .await
                        {
                            Ok(Ok(value)) => value,
                            Ok(Err(error)) => {
                                let io = codex_invalidation_io(&error).0;
                                let code = policy(error);
                                if let Some(reporter) = reporter {
                                    reporter.report_shell_probe("process_info", code, io);
                                }
                                return Err(code);
                            }
                            Err(_) => {
                                if let Some(reporter) = reporter {
                                    reporter.report_shell_probe("shell_not_idle", ErrorCode::Timeout, "none");
                                }
                                return Err(ErrorCode::Timeout);
                            }
                        };
                        if observed.shell_pid != Some(pid) {
                            if let Some(reporter) = reporter {
                                reporter.report_shell_probe("shell_process", ErrorCode::NotReady, "none");
                            }
                            return Err(ErrorCode::NotReady);
                        }
                        match process_evidence(shell_reporter.pid) {
                            Ok(current) if current == shell_reporter => {}
                            Ok(_) => {
                                if let Some(reporter) = reporter {
                                    reporter.report_shell_probe("shell_reporter_recheck", ErrorCode::NotReady, "none");
                                }
                                return Err(ErrorCode::NotReady);
                            }
                            Err(error) => {
                                let io = codex_invalidation_io(&error).0;
                                let code = policy(error);
                                if let Some(reporter) = reporter {
                                    reporter.report_shell_probe("shell_reporter_recheck", code, io);
                                }
                                return Err(code);
                            }
                        }
                        match process_evidence(shell.pid) {
                            Ok(current) if current == shell => {}
                            Ok(_) => {
                                if let Some(reporter) = reporter {
                                    reporter.report_shell_probe("shell_process", ErrorCode::NotReady, "none");
                                }
                                return Err(ErrorCode::NotReady);
                            }
                            Err(error) => {
                                let io = codex_invalidation_io(&error).0;
                                let code = policy(error);
                                if let Some(reporter) = reporter {
                                    reporter.report_shell_probe("shell_process", code, io);
                                }
                                return Err(code);
                            }
                        }
                        match shell_executable(&shell) {
                            Ok((current, current_dialect))
                                if current == executable && current_dialect == dialect => {}
                            Ok(_) => {
                                if let Some(reporter) = reporter {
                                    reporter.report_shell_probe("shell_executable", ErrorCode::NotReady, "none");
                                }
                                return Err(ErrorCode::NotReady);
                            }
                            Err(error) => {
                                let io = codex_invalidation_io(&error).0;
                                let code = policy(error);
                                if let Some(reporter) = reporter {
                                    reporter.report_shell_probe("shell_executable", code, io);
                                }
                                return Err(code);
                            }
                        }
                        match self.idle_probe_shell_classified(&refreshed, &observed) {
                            Ok((current_shell, current_reporter, current_executable, current_dialect)) => {
                                {
                                    let _guard = security.native_guard(auth).await.inspect_err(|code| {
                                        if let Some(reporter) = reporter {
                                            reporter.report_shell_probe("authorization", *code, "none");
                                        }
                                    })?;
                                }
                                if tokio::time::Instant::now() >= deadline {
                                    if let Some(reporter) = reporter {
                                        reporter.report_shell_probe("shell_not_idle", ErrorCode::Timeout, "none");
                                    }
                                    return Err(ErrorCode::Timeout);
                                }
                                if let Err(code) = self.fresh_creation_root(&source, &resources, &focus) {
                                    if let Some(reporter) = reporter {
                                        reporter.report_shell_probe("source_identity", code, "none");
                                    }
                                    return Err(code);
                                }
                                match process_evidence(shell_reporter.pid) {
                                    Ok(current) if current == shell_reporter => {}
                                    Ok(_) => {
                                        if let Some(reporter) = reporter {
                                            reporter.report_shell_probe("shell_reporter_recheck", ErrorCode::NotReady, "none");
                                        }
                                        return Err(ErrorCode::NotReady);
                                    }
                                    Err(error) => {
                                        let io = codex_invalidation_io(&error).0;
                                        let code = policy(error);
                                        if let Some(reporter) = reporter {
                                            reporter.report_shell_probe("shell_reporter_recheck", code, io);
                                        }
                                        return Err(code);
                                    }
                                }
                                match process_evidence(shell.pid) {
                                    Ok(current) if current == shell => {}
                                    Ok(_) => {
                                        if let Some(reporter) = reporter {
                                            reporter.report_shell_probe("shell_process", ErrorCode::NotReady, "none");
                                        }
                                        return Err(ErrorCode::NotReady);
                                    }
                                    Err(error) => {
                                        let io = codex_invalidation_io(&error).0;
                                        let code = policy(error);
                                        if let Some(reporter) = reporter {
                                            reporter.report_shell_probe("shell_process", code, io);
                                        }
                                        return Err(code);
                                    }
                                }
                                match shell_executable(&shell) {
                                    Ok((current, current_dialect))
                                        if current == executable && current_dialect == dialect => {}
                                    Ok(_) => {
                                        if let Some(reporter) = reporter {
                                            reporter.report_shell_probe("shell_executable", ErrorCode::NotReady, "none");
                                        }
                                        return Err(ErrorCode::NotReady);
                                    }
                                    Err(error) => {
                                        let io = codex_invalidation_io(&error).0;
                                        let code = policy(error);
                                        if let Some(reporter) = reporter {
                                            reporter.report_shell_probe("shell_executable", code, io);
                                        }
                                        return Err(code);
                                    }
                                }
                                if current_shell != shell || current_reporter != shell_reporter {
                                    if let Some(reporter) = reporter {
                                        reporter.report_shell_probe("shell_process", ErrorCode::NotReady, "none");
                                    }
                                    return Err(ErrorCode::NotReady);
                                }
                                if current_executable != executable || current_dialect != dialect {
                                    if let Some(reporter) = reporter {
                                        reporter.report_shell_probe("shell_executable", ErrorCode::NotReady, "none");
                                    }
                                    return Err(ErrorCode::NotReady);
                                }
                                if tokio::time::Instant::now() >= deadline {
                                    if let Some(reporter) = reporter {
                                        reporter.report_shell_probe("shell_not_idle", ErrorCode::Timeout, "none");
                                    }
                                    return Err(ErrorCode::Timeout);
                                }
                                break (shell, shell_reporter, executable, dialect);
                            }
                            Err((_, _, ProbeShellGuard::ShellNotIdle)) => {}
                            Err((code, io, guard)) => {
                                if let Some(reporter) = reporter {
                                    reporter.report_shell_probe(guard.shell_probe_guard(), code, io);
                                }
                                return Err(code);
                            }
                        }
                        tokio::time::sleep_until(
                            deadline.min(tokio::time::Instant::now() + Duration::from_millis(200)),
                        )
                        .await;
                    }
                }
                Err((code, io, guard)) => {
                    if let Some(reporter) = reporter {
                        reporter.report_shell_probe(guard.shell_probe_guard(), code, io);
                    }
                    return Err(code);
                }
            };
        // This is independent kernel-cwd object proof, not a snapshot cwd or the
        // input-sending wrapper-resolution query. Failure cannot admit that query.
        selection.validate(self,params).and_then(|_|selection.retain_shell(shell.clone())).inspect_err(|code| {
            if let Some(reporter)=reporter {reporter.report_shell_probe("creation_directory",*code,"none");}
        })?;
        Ok(Some(PreparedShell{action,params,selection,deadline,resources,focus,source,info,
            shell,original_reporter,executable,dialect}))
    }
    pub(crate) async fn uncertain_creation(&self,auth:&Authorized,action:&Uuid,code:ErrorCode)->SecurityResult<Receipt> {
        let proof=auth.proof.clone();let id=action.clone();
        self.storage.call(move|store| {
            let current=store.status(&proof,&id,pairing::now_ms())?.ok_or(StorageError::Policy(ErrorCode::NotFound))?;
            if !matches!(current.state,ReceiptState::Reserved|ReceiptState::Dispatching){return Ok(current)}
            if current.creation.as_ref().is_none_or(|creation|creation.resources.workspace_id.is_none()) {
                return Err(StorageError::Policy(code));
            }
            store.transition(&proof,&id,ReceiptTransition{state:ReceiptState::Uncertain,
                proof:TransitionProof::LostOutcome,result:None,error:Some(crate::transport::wire_error(code)),at:pairing::now_ms()})
        }).await.map_err(pairing::store_error)
    }
    pub(crate) async fn shell_probe_once_reported(
        &self,
        security:&Security,
        auth:&Authorized,
        prepared:PreparedShell<'_>,
        reporter:Option<&OmpCreationReporter>,
    )->SecurityResult<Receipt> {
        let PreparedShell{action,params,selection,deadline,resources,focus,source,info,
            shell,original_reporter,executable,dialect}=prepared;
        let pane=source.snapshot.pane(resources.pane_id.as_ref().ok_or(ErrorCode::NotReady)?).map_err(policy)?;
        let attempt = Uuid::random();
        let mut artifact = match ProbeArtifact::create(&self.config, &attempt, &shell) {
            Ok(artifact) => artifact,
            Err(error) => {
                let io = codex_invalidation_io(&error).0;
                let code = policy(error);
                if let Some(reporter) = reporter {
                    reporter.report_shell_probe("artifact_create", code, io);
                }
                return Err(code);
            }
        };
        let mut random = [0; 32];
        if getrandom::fill(&mut random).is_err() {
            if let Some(reporter) = reporter {
                reporter.report_shell_probe("nonce", ErrorCode::NotReady, "none");
            }
            return Err(ErrorCode::NotReady);
        }
        let nonce = hex(&random);
        random.fill(0);
        let Some(wrapper) = self.config.wrapper(params.provider) else {
            if let Some(reporter) = reporter {
                reporter.report_shell_probe("wrapper", ErrorCode::NotReady, "none");
            }
            return Err(ErrorCode::NotReady);
        };
        let command = match shell_probe_command(
            dialect,
            params.provider,
            &nonce,
            wrapper,
            &artifact.provider_path,
        ) {
            Ok(command) => command,
            Err(error) => {
                let io = codex_invalidation_io(&error).0;
                let code = policy(error);
                if let Some(reporter) = reporter {
                    reporter.report_shell_probe("command", code, io);
                }
                return Err(code);
            }
        };
        let Some(workspace_id) = resources.workspace_id.clone() else {
            if let Some(reporter) = reporter {
                reporter.report_shell_probe("metadata_workspace", ErrorCode::NotReady, "none");
            }
            return Err(ErrorCode::NotReady);
        };
        let Some(executable_text) = executable.to_str() else {
            if let Some(reporter) = reporter {
                reporter.report_shell_probe("metadata_executable", ErrorCode::NotReady, "none");
            }
            return Err(ErrorCode::NotReady);
        };
        let Some(result_path) = artifact.path.to_str() else {
            if let Some(reporter) = reporter {
                reporter.report_shell_probe("metadata_result_path", ErrorCode::NotReady, "none");
            }
            return Err(ErrorCode::NotReady);
        };
        let probe = ShellProbeMetadata {
            attempt_id: attempt,
            action_id: action.clone(),
            workspace_id,
            pane_id: pane.pane_id.clone(),
            terminal_id: pane.terminal_id.clone(),
            shell: shell.clone(),
            executable: executable_text.into(),
            result_path: result_path.into(),
            result_device: artifact.file_object.0,
            result_inode: artifact.file_object.1,
            parent_device: artifact.dir_object.0,
            parent_inode: artifact.dir_object.1,
            focused_workspace_id: focus.clone(),
        };
        let client = match Client::connect(&source.socket_path).await {
            Ok(client) => client,
            Err(error) => {
                let io = codex_invalidation_io(&error).0;
                let code = policy(error);
                if let Some(reporter) = reporter {
                    reporter.report_shell_probe("fence_connect", code, io);
                }
                return Err(code);
            }
        };
        if client.evidence != source.evidence {
            if let Some(reporter) = reporter {
                reporter.report_shell_probe("fence_identity", ErrorCode::StaleSession, "none");
            }
            return Err(ErrorCode::StaleSession);
        }
        let fd = match client.fence_socket() {
            Ok(fd) => fd,
            Err(error) => {
                let io = codex_invalidation_io(&error).0;
                let code = policy(error);
                if let Some(reporter) = reporter {
                    reporter.report_shell_probe("fence_socket", code, io);
                }
                return Err(code);
            }
        };
        let mut metadata = client.metadata();
        metadata.socket_epoch = Some(source.incarnation.clone());
        metadata.workspace_operation = WorkspaceOperation::ShellProbe;
        metadata.shell_probe = Some(probe.clone());
        metadata.focused_workspace_id = Some(focus.clone());
        let result_metadata = metadata.clone();
        let retained = resources.clone();
        let root = pane.pane_id.clone();
        let query_reporter = reporter.cloned();
        let query_codex_reporter = if params.provider == Provider::Codex {
            self.codex_invalidation_reporter
                .lock()
                .ok()
                .and_then(|current| current.clone())
        } else {
            None
        };
        security
            .creation_boundary_guarded(
                auth,
                action.clone(),
                (CreationStage::Workspace, metadata.clone()),
                || {
                    if tokio::time::Instant::now() >= deadline {
                        return Err(ErrorCode::Timeout.into());
                    }
                    selection.validate(self,params).map_err(|code| CreationAdmissionFailure::guarded(code, CreationAdmissionGuard::Directory))?;
                    self.fresh_creation_root(&source, &resources, &focus)
                        .map_err(|code| {
                            CreationAdmissionFailure::guarded(code, CreationAdmissionGuard::Root)
                        })?;
                    if process_evidence(shell.pid).map_err(|error| {
                        CreationAdmissionFailure::guarded(
                            policy(error),
                            CreationAdmissionGuard::ShellEpoch,
                        )
                    })? != shell
                        || shell_executable(&shell)
                            .map_err(|error| {
                                CreationAdmissionFailure::guarded(
                                    policy(error),
                                    CreationAdmissionGuard::ShellEpoch,
                                )
                            })?
                            .0
                            != executable
                    {
                        return Err(CreationAdmissionFailure::guarded(
                            ErrorCode::NotReady,
                            CreationAdmissionGuard::ShellEpoch,
                        ));
                    }
                    artifact.recheck(&shell).map_err(|error| {
                        CreationAdmissionFailure::guarded(
                            policy(error),
                            CreationAdmissionGuard::ResultView,
                        )
                    })
                },
                fd,
                async move {
                    let mut drop_witness = ShellProbeEffectDropWitness::new(
                        query_reporter.clone(),
                        query_codex_reporter.clone(),
                    );
                    let reply = client
                        .request(
                            "pane.send_input",
                            json!({"pane_id":root,"text":command,"keys":["Enter"]}),
                            "ok",
                        )
                        .await;
                    drop_witness.disarm();
                    let (state, definitive) = match reply {
                        Ok(_) => (
                            StageState::Dispatching,
                            Some(Id(
                                "Correlated fixed shell query enqueued; result pending".into()
                            )),
                        ),
                        Err(ApiError::Rejected(code))
                            if matches!(
                                code.as_str(),
                                "pane_not_found" | "invalid_key" | "pane_send_failed"
                            ) =>
                        {
                            if let Some(reporter) = &query_reporter {
                                reporter.report_shell_probe(
                                    "query_rejected_known",
                                    ErrorCode::NotReady,
                                    "none",
                                );
                            }
                            if let Some(reporter) = &query_codex_reporter {
                                reporter.shell_probe_prebinding(
                                    "query_rejected_known",
                                    ErrorCode::NotReady,
                                    "none",
                                );
                            }
                            (
                                StageState::Failed,
                                Some(Id("Herdr rejected fixed query before enqueue".into())),
                            )
                        }
                        Err(ApiError::Rejected(_)) => {
                            if let Some(reporter) = &query_reporter {
                                reporter.report_shell_probe(
                                    "query_rejected_other",
                                    ErrorCode::NotReady,
                                    "none",
                                );
                            }
                            if let Some(reporter) = &query_codex_reporter {
                                reporter.shell_probe_prebinding(
                                    "query_rejected_other",
                                    ErrorCode::NotReady,
                                    "none",
                                );
                            }
                            (StageState::Uncertain, None)
                        }
                        Err(ApiError::Changed(_)) => {
                            if let Some(reporter) = &query_reporter {
                                reporter.report_shell_probe(
                                    "query_source_changed",
                                    ErrorCode::StaleSession,
                                    "none",
                                );
                            }
                            if let Some(reporter) = &query_codex_reporter {
                                reporter.shell_probe_prebinding(
                                    "query_source_changed",
                                    ErrorCode::StaleSession,
                                    "none",
                                );
                            }
                            (StageState::Uncertain, None)
                        }
                        Err(ApiError::Unavailable(error)) => {
                            let io = codex_invalidation_io(&error).0;
                            let code = if error.kind() == io::ErrorKind::OutOfMemory {
                                ErrorCode::ResourceLimit
                            } else {
                                ErrorCode::NotReady
                            };
                            if let Some(reporter) = &query_reporter {
                                reporter.report_shell_probe("query_unavailable", code, io);
                            }
                            if let Some(reporter) = &query_codex_reporter {
                                reporter.shell_probe_prebinding_error(
                                    "query_unavailable",
                                    code,
                                    &error,
                                );
                            }
                            (StageState::Uncertain, None)
                        }
                    };
                    CreationOutcome {
                        resources: retained,
                        metadata: result_metadata,
                        state,
                        definitive,
                        at: pairing::now_ms(),
                    }
                },
            )
            .await?;
        let submitted = self.receipt(security, auth, action).await?;
        if submitted.state != ReceiptState::Dispatching
            || submitted
                .creation
                .as_ref()
                .is_none_or(|p| p.stage_state != StageState::Dispatching)
        {
            return Ok(submitted);
        }
        let capture_deadline=reporter.is_some_and(OmpCreationReporter::backend_capture);
        let mut deadline_detail=None;
        let matched = loop {
            let loop_now=tokio::time::Instant::now();
            if capture_deadline&&deadline_detail.is_none() {
                deadline_detail=Some((ProbeObservation::NotRead,probe_entry_budget(loop_now,deadline)));
            }
            if loop_now >= deadline {
                if let Some(reporter) = reporter {
                    reporter.report_probe_deadline("loop_check",deadline_detail);
                }
                self.report_codex_shell_probe(
                    params.provider,
                    "result_deadline",
                    ErrorCode::Timeout,
                    "none",
                );
                break None;
            }
            {
                let _guard = security.native_guard(auth).await.inspect_err(|code| {
                    if let Some(reporter) = reporter {
                        reporter.report_shell_probe("authorization", *code, "none");
                    }
                    self.report_codex_shell_probe(
                        params.provider,
                        "result_authorization",
                        *code,
                        "none",
                    );
                })?;
            }
            if tokio::time::Instant::now() >= deadline {
                if let Some(reporter) = reporter {
                    reporter.report_probe_deadline("after_authorization",deadline_detail);
                }
                self.report_codex_shell_probe(
                    params.provider,
                    "result_deadline",
                    ErrorCode::Timeout,
                    "none",
                );
                break None;
            }
            let current_shell = process_evidence(shell.pid).map_err(|error| {
                let io = codex_invalidation_io(&error).0;
                let code = if error.kind() == io::ErrorKind::OutOfMemory {
                    ErrorCode::ResourceLimit
                } else {
                    ErrorCode::NotReady
                };
                if let Some(reporter) = reporter {
                    reporter.report_shell_probe("result_shell_changed", code, io);
                }
                self.report_codex_shell_probe_error(
                    params.provider,
                    "result_shell_changed",
                    code,
                    &error,
                );
                code
            })?;
            if current_shell != shell {
                if let Some(reporter) = reporter {
                    reporter.report_shell_probe("result_shell_changed", ErrorCode::NotReady, "none");
                }
                self.report_codex_shell_probe(
                    params.provider,
                    "result_shell_changed",
                    ErrorCode::NotReady,
                    "none",
                );
                break None;
            }
            let bytes = match artifact.read(&shell) {
                Ok(bytes) => bytes,
                Err(error) => {
                    let io = codex_invalidation_io(&error).0;
                    let code = if error.kind() == io::ErrorKind::OutOfMemory {
                        ErrorCode::ResourceLimit
                    } else {
                        ErrorCode::NotReady
                    };
                    if let Some(reporter) = reporter {
                        reporter.report_shell_probe("result_read_failed", code, io);
                    }
                    self.report_codex_shell_probe_error(
                        params.provider,
                        "result_read_failed",
                        code,
                        &error,
                    );
                    break None;
                }
            };
            if let Some((observation,_))=&mut deadline_detail {observation.observe(&bytes);}
            if bytes.ends_with(b"\n") {
                if tokio::time::Instant::now() >= deadline {
                    if let Some(reporter) = reporter {
                        reporter.report_probe_deadline("complete_before_parse",deadline_detail);
                    }
                    self.report_codex_shell_probe(
                        params.provider,
                        "result_deadline",
                        ErrorCode::Timeout,
                        "none",
                    );
                    break None;
                }
                match shell_probe_result_category(&bytes, &nonce, &shell) {
                    Ok(category) => break Some(category),
                    Err(error) => {
                        let io = codex_invalidation_io(&error).0;
                        let code = if error.kind() == io::ErrorKind::OutOfMemory {
                            ErrorCode::ResourceLimit
                        } else {
                            ErrorCode::NotReady
                        };
                        if let Some(reporter) = reporter {
                            reporter.report_shell_probe("result_invalid", code, io);
                        }
                        self.report_codex_shell_probe_error(
                            params.provider,
                            "result_invalid",
                            code,
                            &error,
                        );
                        break None;
                    }
                }
            }
            tokio::time::sleep_until(
                deadline.min(tokio::time::Instant::now() + Duration::from_millis(25)),
            )
            .await;
        };
        let matched = match matched {
            Some(ShellProbeResult::File(accepted)) => Some(accepted),
            Some(ShellProbeResult::Rejected) => {
                if let Some(reporter) = reporter {
                    reporter.report_shell_probe("result_rejected", ErrorCode::NotReady, "none");
                }
                self.report_codex_shell_probe(
                    params.provider,
                    "result_rejected",
                    ErrorCode::NotReady,
                    "none",
                );
                Some(false)
            }
            Some(ShellProbeResult::RejectedKind) => {
                if let Some(reporter) = reporter {
                    reporter.report_shell_probe("result_rejected_kind", ErrorCode::NotReady, "none");
                }
                self.report_codex_shell_probe(
                    params.provider,
                    "result_rejected_kind",
                    ErrorCode::NotReady,
                    "none",
                );
                Some(false)
            }
            Some(ShellProbeResult::RejectedPath) => {
                if let Some(reporter) = reporter {
                    reporter.report_shell_probe("result_rejected_path", ErrorCode::NotReady, "none");
                }
                self.report_codex_shell_probe(
                    params.provider,
                    "result_rejected_path",
                    ErrorCode::NotReady,
                    "none",
                );
                Some(false)
            }
            Some(ShellProbeResult::Unsupported) => {
                if let Some(reporter) = reporter {
                    reporter.report_shell_probe("result_unsupported", ErrorCode::NotReady, "none");
                }
                self.report_codex_shell_probe(
                    params.provider,
                    "result_unsupported",
                    ErrorCode::NotReady,
                    "none",
                );
                Some(false)
            }
            None => None,
        };
        let matched = if matched == Some(true) && tokio::time::Instant::now() >= deadline {
            if let Some(reporter) = reporter {
                reporter.report_probe_deadline("parsed_success",deadline_detail);
            }
            self.report_codex_shell_probe(
                params.provider,
                "result_deadline",
                ErrorCode::Timeout,
                "none",
            );
            None
        } else {
            matched
        };
        let final_deadline = || {
            self.report_codex_shell_probe(
                params.provider,
                "final_deadline",
                ErrorCode::Timeout,
                "none",
            );
            ErrorCode::Timeout
        };
        let final_io = |guard, error: io::Error| {
            let code = if error.kind() == io::ErrorKind::OutOfMemory {
                ErrorCode::ResourceLimit
            } else {
                ErrorCode::NotReady
            };
            self.report_codex_shell_probe_error(params.provider, guard, code, &error);
            code
        };
        let final_changed = || {
            self.report_codex_shell_probe(
                params.provider,
                "final_shell_changed",
                ErrorCode::StaleSession,
                "none",
            );
            ErrorCode::StaleSession
        };
        let final_scope = || -> SecurityResult<Source> {
            selection.validate(self,params)?;
            let refreshed = self
                .fresh_creation_root_classified(&source, &resources, &focus)
                .inspect_err(|(code, guard)| {
                    self.report_codex_shell_probe(
                        params.provider,
                        guard.map_or("final_root", CreationRootGuard::shell_probe_guard),
                        *code,
                        "none",
                    );
                })
                .map_err(|(code, _)| code)?;
            if process_evidence(original_reporter.pid)
                .map_err(|error| final_io("final_shell_reporter", error))? != original_reporter
            {
                return Err(final_changed());
            }
            let pid = info.shell_pid.ok_or_else(final_changed)?;
            if process_evidence(shell.pid)
                .map_err(|error| final_io("final_shell_process", error))? != shell
                || !reported_pid_matches(&original_reporter, &shell, pid)
            {
                return Err(final_changed());
            }
            let (path, current_dialect) = shell_executable(&shell)
                .map_err(|error| final_io("final_shell_executable", error))?;
            if path != executable || current_dialect != dialect
                || process_evidence(shell.pid)
                    .map_err(|error| final_io("final_shell_process", error))? != shell
                || process_evidence(original_reporter.pid)
                    .map_err(|error| final_io("final_shell_reporter_recheck", error))? != original_reporter
            {
                return Err(final_changed());
            }
            Ok(refreshed)
        };
        let valid = if matched == Some(true) {
            // A published query result can precede the original shell becoming idle.
            // Only that busy observation may settle, within this same absolute deadline.
            loop {
                if tokio::time::Instant::now() >= deadline {
                    return Err(final_deadline());
                }
                final_scope()?;
                if tokio::time::Instant::now() >= deadline {
                    return Err(final_deadline());
                }
                {
                    let _guard = tokio::time::timeout_at(deadline, security.native_guard(auth))
                        .await.map_err(|_| final_deadline())?
                        .inspect_err(|code| {
                            self.report_codex_shell_probe(params.provider, "final_authorization", *code, "none");
                        })?;
                }
                if tokio::time::Instant::now() >= deadline {
                    return Err(final_deadline());
                }
                let refreshed = final_scope()?;
                if tokio::time::Instant::now() >= deadline {
                    return Err(final_deadline());
                }
                let client = tokio::time::timeout_at(deadline, Client::connect(&refreshed.socket_path))
                    .await.map_err(|_| final_deadline())?
                    .map_err(|error| final_io("final_connect", error))?;
                if tokio::time::Instant::now() >= deadline {
                    return Err(final_deadline());
                }
                final_scope()?;
                if client.evidence != source.evidence {
                    self.report_codex_shell_probe(params.provider, "final_identity", ErrorCode::StaleSession, "none");
                    return Err(ErrorCode::StaleSession);
                }
                if tokio::time::Instant::now() >= deadline {
                    return Err(final_deadline());
                }
                let observed = tokio::time::timeout_at(deadline, client.process_info(&probe.pane_id))
                    .await.map_err(|_| final_deadline())?
                    .map_err(|error| final_io("final_process_info", error))?;
                if tokio::time::Instant::now() >= deadline {
                    return Err(final_deadline());
                }
                let refreshed = final_scope()?;
                if observed.shell_pid != info.shell_pid {
                    return Err(final_changed());
                }
                match self.idle_probe_shell_classified(&refreshed, &observed) {
                    Ok((actual, shell_reporter, path, current_dialect)) => {
                        if actual != shell || shell_reporter != original_reporter
                            || path != executable || current_dialect != dialect
                        {
                            return Err(final_changed());
                        }
                        if tokio::time::Instant::now() >= deadline {
                            return Err(final_deadline());
                        }
                        {
                            let _guard = tokio::time::timeout_at(deadline, security.native_guard(auth))
                                .await.map_err(|_| final_deadline())?
                                .inspect_err(|code| {
                                    self.report_codex_shell_probe(params.provider, "final_authorization", *code, "none");
                                })?;
                        }
                        if tokio::time::Instant::now() >= deadline {
                            return Err(final_deadline());
                        }
                        let refreshed = final_scope()?;
                        artifact.recheck(&shell)
                            .map_err(|error| final_io("final_result_view", error))?;
                        break Some((refreshed, observed, shell_reporter));
                    }
                    Err((_, _, ProbeShellGuard::ShellNotIdle)) => {
                        final_scope()?;
                        if tokio::time::Instant::now() >= deadline {
                            return Err(final_deadline());
                        }
                        tokio::time::sleep_until(
                            deadline.min(tokio::time::Instant::now() + Duration::from_millis(200)),
                        ).await;
                    }
                    Err((code, io, guard)) => {
                        self.report_codex_shell_probe(params.provider, guard.final_shell_probe_guard(), code, io);
                        return Err(code);
                    }
                }
            }
        } else {
            None
        };
        if valid.is_some() && tokio::time::Instant::now() >= deadline {
            self.report_codex_shell_probe(
                params.provider,
                "final_deadline",
                ErrorCode::Timeout,
                "none",
            );
            return Err(ErrorCode::Timeout);
        }
        let final_authorization = if valid.is_some() {
            tokio::time::timeout_at(deadline, security.native_guard(auth))
                .await
                .map_err(|_| {
                    self.report_codex_shell_probe(params.provider, "final_deadline", ErrorCode::Timeout, "none");
                    ErrorCode::Timeout
                })?
        } else {
            security.native_guard(auth).await
        };
        let _guard = final_authorization.inspect_err(|code| {
            self.report_codex_shell_probe(
                params.provider,
                "final_authorization",
                *code,
                "none",
            );
        })?;
        if valid.is_some() && tokio::time::Instant::now() >= deadline {
            self.report_codex_shell_probe(
                params.provider,
                "final_deadline",
                ErrorCode::Timeout,
                "none",
            );
            return Err(ErrorCode::Timeout);
        }
        if valid.is_some() {
            final_scope()?;
            if tokio::time::Instant::now() >= deadline {
                return Err(final_deadline());
            }
        }
        let proof = auth.proof.clone();
        let id = action.clone();
        let known = resources.clone();
        let result_metadata = metadata.clone();
        self.storage.call(move|s|s.creation_result(&proof,&id,CreationOutcome{resources:known,metadata:result_metadata,state:match matched{Some(true)=>StageState::Confirmed,Some(false)=>StageState::Failed,None=>StageState::Uncertain},definitive:matched.map(|success|Id(if success{"Matched original-shell nonce/PID and exact external wrapper resolution"}else{"Matched shell query rejected resolution before provider effects"}.into())),at:pairing::now_ms()})).await.map_err(pairing::store_error)?;
        if let Some((refreshed, observed, reporter)) = valid {
            self.record_shell_measurement(
                &params.server_id,
                params.provider,
                ShellMeasurement {
                    execution_directory: self.execution_directory(selection, params)?,
                    server_incarnation: refreshed.incarnation,
                    pane_id: probe.pane_id,
                    shell,
                    resolved_outer_wrapper: wrapper.to_owned(),
                    project_id: params.project_id.clone(),
                    action_id: action.clone(),
                },
                &observed,
                &reporter,
            )?;
        } else if matched == Some(false) {
            let proof = auth.proof.clone();
            let id = action.clone();
            self.storage
                .call(move |s| {
                    s.transition(
                        &proof,
                        &id,
                        ReceiptTransition {
                            state: ReceiptState::Failed,
                            proof: TransitionProof::DefinitiveNoEffect(Id(
                                "Provider launch was not admitted after rejected shell resolution"
                                    .into(),
                            )),
                            result: None,
                            error: Some(WireError {
                                code: ErrorCode::NotReady,
                                message: SafeMessage(
                                    "Native shell resolution rejected; workspace retained".into(),
                                ),
                            }),
                            at: pairing::now_ms(),
                        },
                    )
                })
                .await
                .map_err(pairing::store_error)?;
        }
        drop(_guard);
        self.receipt(security, auth, action).await
    }
    /// Called by the native5 job after its real creation preflight/reservation. The actual
    /// socket is opened before admission; its effect future stays unpolled until the fence.
    pub(crate) async fn workspace_once(
        &self,
        security: &Security,
        auth: &Authorized,
        action: &Uuid,
        params: &SessionCreateParams,
        selection:&CreationSelection,
    ) -> SecurityResult<Receipt> {
        let current = self.receipt(security, auth, action).await?;
        let progress = current.creation.as_ref().ok_or(ErrorCode::InvalidRequest)?;
        if current.state != ReceiptState::Reserved
            || progress.stage != CreationStage::Workspace
            || progress.stage_state != StageState::NotStarted
        {
            return Ok(current);
        }
        if progress.server_id != params.server_id
            || progress.project_id != params.project_id
            || progress.provider != params.provider
        {
            return Err(ErrorCode::InvalidRequest);
        }
        let source = selection.validate(self,params)?;
        self.validate_topology_stage(&source, params, None)?;
        let project = selection.directory(params)?;
        let placement=selection.workspace().cloned();
        let client = Client::connect(&source.socket_path).await.map_err(policy)?;
        if client.evidence != source.evidence {
            return Err(ErrorCode::StaleSession);
        }
        let fd = client.fence_socket().map_err(policy)?;
        let mut metadata = client.metadata();
        metadata.socket_epoch = Some(source.incarnation.clone());
        metadata.focused_workspace_id = source.snapshot.focused_workspace_id.clone();
        metadata.placement_workspace_id=placement.clone();
        let mut effect_metadata = metadata.clone();
        let label = params.name.clone();
        self.begin_creation_shell_fence(auth,action,&source).await?;
        let entered=security
            .creation_boundary(
                auth,
                action.clone(),
                (CreationStage::Workspace, metadata),
                || {
                    // Re-resolve the stable authorized pathname at the final write fence;
                    // this does not promise atomic native check-and-chdir.
                    selection.validate(self,params)?;
                    self.validate_topology_stage(&source, params, None)
                },
                fd,
                async move {
                    let mut resources = empty_resources();
                    let (method,expected,arguments)=match &placement {
                        Some(workspace)=>("tab.create","tab_created",
                            json!({"workspace_id":workspace,"cwd":project,"label":label,"focus":false})),
                        None=>("workspace.create","workspace_created",
                            json!({"cwd":project,"label":label,"focus":false})),
                    };
                    let result=client.request(method,arguments,expected).await;
                    let mut state = StageState::Uncertain;
                    let mut definitive = None;
                    match result {
                        Ok(value) => {
                            let (known,tab,complete) = workspace_resources(&value,placement.as_ref());
                            effect_metadata.created_tab_id=tab.clone();
                            resources = known;
                            if complete&&tab.is_some_and(|tab|selection.retain_created(&resources,tab).is_ok()) {
                                state = StageState::Confirmed;
                                definitive = Some(
                                    Id::new("correlated complete workspace/root response")
                                        .expect("constant ID"),
                                );
                            }
                        }
                        Err(ApiError::Changed(value)) => {
                            let (known,tab,_)=workspace_resources(&value,placement.as_ref());
                            resources=known;
                            effect_metadata.created_tab_id=tab;
                        }
                        Err(ApiError::Rejected(code))
                            if matches!(
                                code.as_str(),
                                "invalid_request" | "invalid_params" | "invalid_env"
                            ) =>
                        {
                            state = StageState::Failed;
                            definitive = Some(
                                Id::new("Herdr rejected creation before effect")
                                    .expect("constant ID"),
                            );
                        }
                        Err(ApiError::Rejected(_) | ApiError::Unavailable(_)) => {}
                    }
                    CreationOutcome {
                        resources,
                        metadata: effect_metadata,
                        state,
                        definitive,
                        at: pairing::now_ms(),
                    }
                },
            )
            .await;
        let receipt = self.receipt(security, auth, action).await?;
        if let Err(code)=entered {
            if matches!(receipt.state,ReceiptState::Reserved|ReceiptState::Failed)
                &&receipt.creation.as_ref().is_some_and(|progress|progress.resources.workspace_id.is_none()
                    &&progress.resources.pane_id.is_none()&&progress.resources.terminal_id.is_none())
            {let _=self.release_creation_shell_fence(auth,action).await;}
            return Err(code);
        }
        if let Some(progress)=&receipt.creation {
            if progress.stage_state==StageState::Confirmed {
                if let (Some(pane),Some(terminal))=(&progress.resources.pane_id,&progress.resources.terminal_id) {
                    self.narrow_creation_shell_fence(auth,action,pane,terminal).await?;
                }
            }else if progress.stage_state==StageState::Failed&&progress.resources.workspace_id.is_none()
                &&progress.resources.pane_id.is_none()&&progress.resources.terminal_id.is_none()
            {self.release_creation_shell_fence(auth,action).await?;}
        }
        if receipt
            .creation
            .as_ref()
            .is_some_and(|p| p.stage_state == StageState::Confirmed)
        {
            let resources = &receipt
                .creation
                .as_ref()
                .expect("creation receipt")
                .resources;
            // Read-only confirmation is deliberately outside the10s write/revoke boundary.
            if self
                .confirm_workspace(&source, resources, params.provider, selection)
                .await
                .is_err()
            {
                let proof = auth.proof.clone();
                let id = action.clone();
                self.storage
                    .call(move |s| {
                        s.transition(
                            &proof,
                            &id,
                            ReceiptTransition {
                                state: ReceiptState::Uncertain,
                                proof: TransitionProof::LostOutcome,
                                result: None,
                                error: None,
                                at: pairing::now_ms(),
                            },
                        )
                    })
                    .await
                    .map_err(pairing::store_error)?;
                return self.receipt(security, auth, action).await;
            }
        }
        Ok(receipt)
    }
    async fn confirm_workspace(
        &self,
        source: &Source,
        resources: &CreationResources,
        provider: Provider,
        selection:&CreationSelection,
    ) -> io::Result<()> {
        let client = match Client::connect(&source.socket_path).await {
            Ok(client) => client,
            Err(error) => {
                self.report_codex_shell_probe_error(
                    provider,
                    "workspace_confirm_connect",
                    if error.kind() == io::ErrorKind::OutOfMemory {
                        ErrorCode::ResourceLimit
                    } else {
                        ErrorCode::NotReady
                    },
                    &error,
                );
                return Err(error);
            }
        };
        if client.evidence != source.evidence {
            let error = invalid("Herdr epoch changed after workspace response");
            self.report_codex_shell_probe_error(
                provider,
                "workspace_confirm_evidence_epoch",
                ErrorCode::StaleSession,
                &error,
            );
            return Err(error);
        }
        let (_, snapshot) = match client.snapshot().await {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.report_codex_shell_probe_error(
                    provider,
                    "workspace_confirm_snapshot",
                    if error.kind() == io::ErrorKind::OutOfMemory {
                        ErrorCode::ResourceLimit
                    } else {
                        ErrorCode::NotReady
                    },
                    &error,
                );
                return Err(error);
            }
        };
        let pane_id = match resources.pane_id.as_ref() {
            Some(pane_id) => pane_id,
            None => {
                let error = invalid("root pane absent");
                self.report_codex_shell_probe_error(
                    provider,
                    "workspace_confirm_pane",
                    ErrorCode::NotReady,
                    &error,
                );
                return Err(error);
            }
        };
        let pane = match snapshot.pane(pane_id) {
            Ok(pane) => pane,
            Err(error) => {
                self.report_codex_shell_probe_error(
                    provider,
                    "workspace_confirm_pane",
                    if error.kind() == io::ErrorKind::OutOfMemory {
                        ErrorCode::ResourceLimit
                    } else {
                        ErrorCode::NotReady
                    },
                    &error,
                );
                return Err(error);
            }
        };
        if resources.workspace_id.as_ref() != Some(&pane.workspace_id) {
            let error = invalid("authoritative created terminal mismatch");
            self.report_codex_shell_probe_error(
                provider,
                "workspace_confirm_created_workspace",
                ErrorCode::StaleSession,
                &error,
            );
            return Err(error);
        }
        if resources.terminal_id.as_ref() != Some(&pane.terminal_id) {
            let error = invalid("authoritative created terminal mismatch");
            self.report_codex_shell_probe_error(
                provider,
                "workspace_confirm_terminal",
                ErrorCode::StaleSession,
                &error,
            );
            return Err(error);
        }
        if snapshot.focused_workspace_id != source.snapshot.focused_workspace_id {
            let error = invalid("desktop focus changed during no-focus workspace creation");
            self.report_codex_shell_probe_error(
                provider,
                "workspace_confirm_focus",
                ErrorCode::StaleSession,
                &error,
            );
            return Err(error);
        }
        if !selection.matches_created(pane) {
            let error=invalid("authoritative created tab membership changed");
            self.report_codex_shell_probe_error(provider,"workspace_confirm_tab",ErrorCode::StaleSession,&error);
            return Err(error);
        }
        Ok(())
    }
    /// Kind/name/pane/args/timeout only. Herdr acknowledges submitted argv, not readiness.
    /// Provider5 supplies already confirmed native resources and performs later read-only
    /// native readiness polling outside this bounded launch/write fence.
    pub(crate) async fn launch_once(
        &self,
        security: &Security,
        auth: &Authorized,
        action: &Uuid,
        params: &SessionCreateParams,
        selection:&CreationSelection,
        args: Vec<String>,
    ) -> SecurityResult<Receipt> {
        self.launch_once_reported(security, auth, action, (params,selection), args, None)
            .await
    }
    fn codex_startup_terminal_reporter(&self) -> Result<Option<CodexInvalidationReporter>, ErrorCode> {
        if std::env::var_os("GOATR_OWNED_CODEX_STARTUP_TERMINAL_CAPTURE").as_deref()
            != Some(std::ffi::OsStr::new("1"))
        {
            return Ok(None);
        }
        self.codex_invalidation_reporter
            .lock()
            .ok()
            .and_then(|reporter| reporter.clone())
            .map(Some)
            .ok_or(ErrorCode::NotReady)
    }
    fn retain_codex_startup_terminal_pin(
        &self,
        action: &Uuid,
        source: &Source,
        resources: &CreationResources,
    ) -> io::Result<CodexStartupTerminalPin> {
        let workspace = resources
            .workspace_id
            .clone()
            .ok_or_else(|| invalid("startup terminal workspace absent"))?;
        let pane = resources
            .pane_id
            .clone()
            .ok_or_else(|| invalid("startup terminal pane absent"))?;
        let terminal = resources
            .terminal_id
            .clone()
            .ok_or_else(|| invalid("startup terminal terminal absent"))?;
        let pane_snapshot = source
            .snapshot
            .panes
            .iter()
            .find(|candidate| candidate.pane_id == pane)
            .ok_or_else(|| invalid("startup terminal pane unavailable"))?;
        let read_identity = pane_snapshot
            .read_identity
            .clone()
            .ok_or_else(|| invalid("startup terminal read identity absent"))?;
        read_identity.validate()?;
        if pane_snapshot.workspace_id != workspace || pane_snapshot.terminal_id != terminal {
            return Err(invalid("startup terminal resource routing changed"));
        }
        let pin = CodexStartupTerminalPin {
            action: action.clone(),
            server_id: source.server_id.clone(),
            source_incarnation: source.incarnation.clone(),
            source_evidence: source.evidence.clone(),
            socket_path: source.socket_path.clone(),
            workspace_id: workspace,
            tab_id: pane_snapshot.tab_id.clone(),
            pane_id: pane,
            terminal_id: terminal,
            read_identity,
        };
        pairing::private_dir(&self.config.runtime_dir, false)?;
        let path = self
            .config
            .runtime_dir
            .join(format!("codex-startup-terminal-{}.json", action.0));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&path)?;
        serde_json::to_writer(&file, &pin)?;
        file.sync_all()?;
        pairing::private_file(&path)?;
        Ok(pin)
    }
    pub(crate) async fn launch_once_reported(
        &self,
        security: &Security,
        auth: &Authorized,
        action: &Uuid,
        (params,selection):(&SessionCreateParams,&CreationSelection),
        args: Vec<String>,
        reporter: Option<OmpCreationReporter>,
    ) -> SecurityResult<Receipt> {
        let receipt = self.receipt(security, auth, action).await?;
        let progress = receipt.creation.as_ref().ok_or(ErrorCode::InvalidRequest)?;
        if receipt.state != ReceiptState::Dispatching
            || matches!(progress.stage, CreationStage::Launch | CreationStage::Ready)
        {
            return Ok(receipt);
        }
        if progress.server_id != params.server_id
            || progress.project_id != params.project_id
            || progress.provider != params.provider
        {
            return Err(ErrorCode::InvalidRequest);
        }
        if args.len() > 512
            || args
                .iter()
                .any(|a| a.len() > FRAME_MAX || a.chars().any(char::is_control))
        {
            return Err(ErrorCode::InvalidRequest);
        }
        let resources = progress.resources.clone();
        let proof = auth.proof.clone();
        let id = action.clone();
        let local = self
            .storage
            .call(move |s| s.creation_local(&proof, &id))
            .await
            .map_err(pairing::store_error)?
            .ok_or(ErrorCode::NotFound)?;
        let source = match self.source(&params.server_id) {
            Ok(source) => source,
            Err(code) => {
                if let Some(reporter) = &reporter { reporter.report_launch_preflight("source_lookup", code, "none"); }
                return Err(code);
            }
        };
        if let Err((code, guard, io)) = self.validate_topology_stage_classified(&source, params, Some((&resources, action))) {
            if let Some(reporter) = &reporter { reporter.report_launch_preflight(guard, code, io); }
            return Err(code);
        }
        let proof = auth.proof.clone();
        let id = action.clone();
        let probe = self
            .storage
            .call(move |s| s.creation_probe(&proof, &id))
            .await
            .map_err(pairing::store_error)?;
        if let Err((code, io, guard)) = self.fresh_probe_root_classified(&source, &resources, &probe.focused_workspace_id) {
            if let Some(reporter) = &reporter { reporter.report_launch_preflight(guard.immediate_guard(), code, io); }
            return Err(code);
        }
        if source
            .snapshot
            .agents
            .iter()
            .any(|a| a.name.as_deref() == Some(&local.alias))
        {
            return Err(ErrorCode::ActionIdConflict);
        }
        let pane = match resources.pane_id.clone() {
            Some(pane) => pane,
            None => {
                if let Some(reporter) = &reporter { reporter.report_launch_preflight("resource_pane_absent", ErrorCode::NotReady, "none"); }
                return Err(ErrorCode::NotReady);
            }
        };
        let terminal = match resources.terminal_id.clone() {
            Some(terminal) => terminal,
            None => {
                if let Some(reporter) = &reporter { reporter.report_launch_preflight("resource_terminal_absent", ErrorCode::NotReady, "none"); }
                return Err(ErrorCode::NotReady);
            }
        };
        let process_client = match Client::connect(&source.socket_path).await {
            Ok(client) => client,
            Err(error) => {
                let io = codex_invalidation_io(&error).0;
                let code = policy(error);
                if let Some(reporter) = &reporter { reporter.report_launch_preflight("process_connect", code, io); }
                return Err(code);
            }
        };
        if process_client.evidence != source.evidence {
            if let Some(reporter) = &reporter { reporter.report_launch_preflight("process_evidence", ErrorCode::StaleSession, "none"); }
            return Err(ErrorCode::StaleSession);
        }
        let observed = match process_client.process_info(&pane).await {
            Ok(observed) => observed,
            Err(error) => {
                let io = codex_invalidation_io(&error).0;
                let code = policy(error);
                if let Some(reporter) = &reporter { reporter.report_launch_preflight("process_info", code, io); }
                return Err(code);
            }
        };
        let (shell, _, executable, _) = match self.idle_probe_shell_classified(&source, &observed) {
            Ok(shell) => shell,
            Err((code, io, guard)) => {
                if let Some(reporter) = &reporter { reporter.report_launch_preflight(guard.shell_probe_guard(), code, io); }
                return Err(code);
            }
        };
        if shell != probe.shell {
            if let Some(reporter) = &reporter { reporter.report_launch_preflight("original_shell_mismatch", ErrorCode::StaleSession, "none"); }
            return Err(ErrorCode::StaleSession);
        }
        if executable.to_str() != Some(probe.executable.as_str()) {
            if let Some(reporter) = &reporter { reporter.report_launch_preflight("original_executable_mismatch", ErrorCode::StaleSession, "none"); }
            return Err(ErrorCode::StaleSession);
        }
        if let Err((code, guard, io)) = self.validate_topology_stage_classified(&source, params, Some((&resources, action))) {
            if let Some(reporter) = &reporter { reporter.report_launch_preflight(guard, code, io); }
            return Err(code);
        }
        let client = match Client::connect(&source.socket_path).await {
            Ok(client) => client,
            Err(error) => {
                let io = codex_invalidation_io(&error).0;
                let code = policy(error);
                if let Some(reporter) = &reporter { reporter.report_launch_preflight("launch_connect", code, io); }
                return Err(code);
            }
        };
        if client.evidence != source.evidence {
            if let Some(reporter) = &reporter { reporter.report_launch_preflight("launch_evidence", ErrorCode::StaleSession, "none"); }
            return Err(ErrorCode::StaleSession);
        }
        let fd = match client.fence_socket() {
            Ok(fd) => fd,
            Err(error) => {
                let io = codex_invalidation_io(&error).0;
                let code = policy(error);
                if let Some(reporter) = &reporter { reporter.report_launch_preflight("launch_socket_fence", code, io); }
                return Err(code);
            }
        };
        let mut metadata = client.metadata();
        metadata.socket_epoch = Some(source.incarnation.clone());
        let effect_metadata = metadata.clone();
        let prior = resources.clone();
        if params.provider == Provider::Codex
            && let Some(capture) = self.codex_startup_terminal_reporter()?
            && let Err(error) = self.retain_codex_startup_terminal_pin(action, &source, &resources)
        {
            let (io, errno) = codex_invalidation_io(&error);
            let code = policy(error);
            capture.startup_terminal("startup_terminal_pin", code, io, errno);
            return Err(code);
        }
        let kind = match params.provider {
            Provider::Omp => "omp",
            Provider::Codex => "codex",
        };
        let preflight_reporter = reporter.clone();
        security.creation_boundary_guarded(auth, action.clone(), (CreationStage::Launch, metadata), || { selection.validate(self, params).map_err(|code| CreationAdmissionFailure::guarded(code, CreationAdmissionGuard::Directory))?; let checked: SecurityResult<()> = (|| {
            let current = match self.fresh_creation_root(&source, &resources, &probe.focused_workspace_id) {
                Ok(current) => current,
                Err(code) => {
                    if let Some(reporter) = &preflight_reporter { reporter.report_launch_preflight("original_root", code, "none"); }
                    return Err(code);
                }
            };
            if let Err((code, guard, io)) = self.validate_topology_stage_classified(&current, params, Some((&resources, action))) {
                if let Some(reporter) = &preflight_reporter { reporter.report_launch_preflight(guard, code, io); }
                return Err(code);
            }
            if let Err((code, io, guard)) = self.fresh_probe_root_classified(&current, &resources, &probe.focused_workspace_id) {
                if let Some(reporter) = &preflight_reporter { reporter.report_launch_preflight(guard.immediate_guard(), code, io); }
                return Err(code);
            }
            match process_evidence(shell.pid) {
                Ok(current_shell) if current_shell == shell => {}
                Ok(_) => {
                    if let Some(reporter) = &preflight_reporter { reporter.report_launch_preflight("original_shell_mismatch", ErrorCode::StaleSession, "none"); }
                    return Err(ErrorCode::StaleSession);
                }
                Err(error) => {
                    let io = codex_invalidation_io(&error).0;
                    let code = policy(error);
                    if let Some(reporter) = &preflight_reporter { reporter.report_launch_preflight("original_shell_process", code, io); }
                    return Err(code);
                }
            }
            match shell_executable(&shell) {
                Ok((current_executable, _)) if current_executable == executable => Ok(()),
                Ok(_) => {
                    if let Some(reporter) = &preflight_reporter { reporter.report_launch_preflight("original_executable_mismatch", ErrorCode::StaleSession, "none"); }
                    Err(ErrorCode::StaleSession)
                }
                Err(error) => {
                    let io = codex_invalidation_io(&error).0;
                    let code = policy(error);
                    if let Some(reporter) = &preflight_reporter { reporter.report_launch_preflight("original_executable_recheck", code, io); }
                    Err(code)
                }
            }
        })(); checked.map_err(Into::into) }, fd, async move {
            let result = client.request_reported("agent.start", json!({"name":local.alias,"kind":kind,"pane_id":pane,"args":args,"timeout_ms":30000}), "agent_started", reporter.clone()).await;
            let (state, definitive) = match result {
                Ok(value) => {
                    let agent: Result<Agent, _> = serde_json::from_value(value.get("agent").cloned().unwrap_or(Value::Null));
                    let argv_matches = value.get("argv").and_then(Value::as_array).is_some_and(|argv| argv.len() == args.len() + 1 && argv.first().and_then(Value::as_str) == Some(kind) && argv.iter().skip(1).zip(&args).all(|(actual, expected)| actual.as_str() == Some(expected)));
                    let fields = agent.as_ref().map_or((false, false, false, false, false, argv_matches), |agent| (
                        agent.name.as_deref() == Some(local.alias.as_str()),
                        agent.launch_pending || agent.agent.as_deref() == Some(kind),
                        agent.pane_id == pane,
                        agent.terminal_id == terminal,
                        prior.workspace_id.as_ref() == Some(&agent.workspace_id),
                        argv_matches,
                    ));
                    if agent.as_ref().is_ok_and(|a| launch_ack_matches(a, argv_matches, kind, &local.alias, &pane, &terminal, prior.workspace_id.as_ref())) {
                        if let Some(reporter) = &reporter { reporter.report_agent_ack("ack_matched", fields); }
                        (StageState::Dispatching, Some(Id::new("correlated Herdr kind launch submitted").expect("constant ID")))
                    } else {
                        if let Some(reporter) = &reporter { reporter.report_agent_ack("ack_mismatch", fields); }
                        (StageState::Uncertain, None)
                    }
                }
                Err(ApiError::Rejected(code)) if matches!(code.as_str(), "invalid_agent_name" | "unsupported_agent_kind" | "invalid_agent_argument" | "invalid_agent_timeout" | "agent_pane_not_found" | "agent_pane_busy" | "agent_name_taken") => {
                    if let Some(reporter) = &reporter { reporter.report_agent("rejected_known", None); }
                    (StageState::Failed, Some(Id::new("Herdr rejected launch before terminal submission").expect("constant ID")))
                }
                Err(ApiError::Rejected(_)) => {
                    if let Some(reporter) = &reporter { reporter.report_agent("rejected_other", None); }
                    (StageState::Uncertain, None)
                }
                Err(ApiError::Changed(_)) => {
                    if let Some(reporter) = &reporter { reporter.report_agent("source_changed", None); }
                    (StageState::Uncertain, None)
                }
                Err(ApiError::Unavailable(error)) => {
                    if let Some(reporter) = &reporter { reporter.report_agent_io("unavailable", &error); }
                    (StageState::Uncertain, None)
                }
            };
            CreationOutcome { resources: prior, metadata: effect_metadata, state, definitive, at: pairing::now_ms() }
        }).await?;
        self.receipt(security, auth, action).await
    }
    async fn receipt(
        &self,
        security: &Security,
        auth: &Authorized,
        action: &Uuid,
    ) -> SecurityResult<Receipt> {
        let proof = auth.proof.clone();
        let id = action.clone();
        let receipt = security
            .storage
            .call(move |s| s.status(&proof, &id, pairing::now_ms()))
            .await
            .map_err(pairing::store_error)?
            .ok_or(ErrorCode::NotFound)?;
        security.state.peer_event(
            &auth.proof,
            EventBody::ActionStatus(ActionStatusEvent {
                receipt: receipt.clone(),
            }),
        )?;
        Ok(receipt)
    }
    /// A proven backend snapshot installs through the existing single reducer/source epoch.
    /// This seam does not join native providers or infer a binding from titles/cwd/native ID.
    pub async fn begin_native(
        &self,
        key: StableSessionKey,
        owner: ProcessEvidence,
        state: SessionState,
        reconnect: bool,
    ) -> SecurityResult<(Target, Uuid)> {
        self.begin_native_reported(key, owner, state, reconnect, |_, _| {})
            .await
    }
    pub(crate) async fn begin_native_reported(
        &self,
        key: StableSessionKey,
        owner: ProcessEvidence,
        mut state: SessionState,
        reconnect: bool,
        on_codex_arm: impl FnOnce(&Target, &Uuid),
    ) -> SecurityResult<(Target, Uuid)> {
        state.summary.connectivity = SessionConnectivity::Synchronizing;
        let _update = self.updates.lock().await;
        let source = self.source(&key.server_id)?;
        if key.host_id != self.host_id
            || key.server_incarnation != source.incarnation
            || process_evidence(owner.pid).map_err(policy)? != owner
        {
            return Err(ErrorCode::StaleSession);
        }
        let pane = source
            .snapshot
            .panes
            .iter()
            .find(|p| p.terminal_id == key.terminal_id)
            .ok_or(ErrorCode::StaleSession)?;
        if state.summary.server_id != key.server_id
            || state.summary.terminal_id != key.terminal_id
            || state.summary.native_id != key.native_conversation_id
            || state.summary.workspace_id != pane.workspace_id
            || state.summary.pane_id != pane.pane_id
        {
            return Err(ErrorCode::StaleSession);
        }
        // A pin is captured only at installation. Later topology refreshes never replace it.
        let terminal_read_pin = pane.read_identity.clone();
        state.summary.capabilities.terminal_read = false;
        state.summary.capabilities.terminal_create = self.terminal_creation_available(&source,&state.summary.workspace_id);
        state.summary.capabilities.workspace_rename = true;
        state.summary.capabilities.agent_rename = positive_agent_alias(&source.snapshot,pane,state.summary.provider);
        let proven = key.clone();
        let target = self
            .storage
            .call(move |s| s.session_identity(&proven, reconnect))
            .await
            .map_err(pairing::store_error)?;
        state.summary.session_id = target.session_id.clone();
        state.summary.generation = target.generation.clone();
        let display = state.clone();
        let proven = key.clone();
        self.storage
            .call(move |s| s.remember_session_display(&proven, &display))
            .await
            .map_err(pairing::store_error)?;
        source.recheck().map_err(policy)?;
        let codex_reporter = self
            .codex_invalidation_reporter
            .lock()
            .ok()
            .and_then(|current| current.clone());
        let superseded: Vec<_> = self
            .inner
            .lock()
            .expect("topology mutex")
            .bindings
            .values()
            .filter(|binding| {
                binding.key.server_id == key.server_id
                    && binding.key.terminal_id == key.terminal_id
                    && binding.key != key
            })
            .map(|binding| (binding.target.clone(), binding.source_epoch.clone()))
            .collect();
        for (old, epoch) in superseded {
            if self.state.validate_source(&old, &epoch).is_ok() {
                if let Some(reporter) = &codex_reporter {
                    reporter.target(
                        &old,
                        &epoch,
                        "herdr",
                        "binding_superseded",
                        ErrorCode::StaleSession,
                        "none",
                    );
                }
                self.state.binding_offline(
                    &old,
                    &epoch,
                    "Native owner/conversation binding was replaced; old identity remains offline",
                )?;
            }
        }
        let codex = state.summary.provider == Provider::Codex;
        let epoch = self.state.begin_install(state)?;
        let report = source
            .snapshot
            .agents
            .iter()
            .find(|a| a.terminal_id == key.terminal_id)
            .map(|a| (a.agent.clone(), a.agent_session.clone()));
        // This freezes the first admitted Codex identity before its binding is visible to a
        // topology refresh. Later generations never replace the diagnostic target.
        if codex {
            self.arm_codex_invalidation_reporter(&target, &epoch, &key.server_id, &source.evidence);
            on_codex_arm(&target, &epoch);
        }
        self.inner.lock().expect("topology mutex").bindings.insert(
            target.session_id.clone(),
            Binding {
                key,
                target: target.clone(),
                source_epoch: epoch.clone(),
                herdr_report: report,
                terminal_read_pin,
            },
        );
        self.publish()?;
        Ok((target, epoch))
    }
    pub async fn finish_native(
        &self,
        key: &StableSessionKey,
        owner: &ProcessEvidence,
        epoch: &Uuid,
        snapshot: ProviderSnapshot,
    ) -> SecurityResult<()> {
        let _update = self.updates.lock().await;
        let source = self.source(&key.server_id)?;
        if source.incarnation != key.server_incarnation
            || process_evidence(owner.pid).map_err(policy)? != *owner
        {
            return Err(ErrorCode::StaleSession);
        }
        let target = Target {
            session_id: snapshot.state.summary.session_id.clone(),
            generation: snapshot.state.summary.generation.clone(),
        };
        {
            let inner = self.inner.lock().expect("topology mutex");
            let binding = inner
                .bindings
                .get(&target.session_id)
                .ok_or(ErrorCode::StaleSession)?;
            if &binding.key != key || binding.target != target || &binding.source_epoch != epoch {
                return Err(ErrorCode::StaleSession);
            }
        }
        self.state.validate_source(&target, epoch)?;
        let capture_omp = snapshot.state.summary.provider == Provider::Omp;
        // Freeze identity before installation publishes Ready. A matching loss
        // in that interval is emitted only after installation succeeds.
        if capture_omp {
            self.prepare_omp_offline_reporter(&target, epoch, &source, &key.terminal_id);
        }
        self.state.finish_install(epoch, snapshot)?;
        if capture_omp {
            self.confirm_omp_offline_reporter(&target, epoch);
        }
        self.publish()
    }

    pub async fn retire_native(
        &self,
        session: &Uuid,
        history_required: bool,
    ) -> SecurityResult<bool> {
        let _update = self.updates.lock().await;
        let history_required = history_required || self.state.session_required(session);
        let id = session.clone();
        let removed = self
            .storage
            .call(move |s| s.retire_session_mapping(&id, history_required, pairing::now_ms()))
            .await
            .map_err(pairing::store_error)?;
        if removed {
            self.inner
                .lock()
                .expect("topology mutex")
                .bindings
                .remove(session);
            self.state.retire_session(session);
            self.publish()?;
        }
        Ok(removed)
    }
    fn publish(&self) -> SecurityResult<()> {
        let mut inner = self.inner.lock().expect("topology mutex");
        let mut servers = Vec::with_capacity(inner.servers.len());
        let mut unbound = Vec::new();
        let mut hierarchy = Vec::new();
        let mut nodes = 0;
        let mut seen = HashSet::new();
        let mut records: Vec<_> = inner.servers.values().collect();
        records.sort_by(|a, b| a.slot.cmp(&b.slot));
        let mut rows = Vec::new();
        for record in records {
            let ready = record.source.as_ref().is_some_and(|s| s.recheck().is_ok());
            let omp_reason = self
                .omp_creation_reason
                .lock()
                .expect("OMP capability mutex")
                .clone();
            let omp = omp_reason.is_none()
                && record.source.as_ref().is_some_and(|source| {
                    source.snapshot.no_focus().is_ok()
                        && record.coarse_shell.as_ref().is_some_and(|coarse| {
                            self.config.projects.iter().any(|p| {
                                self.validate_coarse(source, coarse, Provider::Omp, &p.id)
                                    .is_ok()
                            })
                        })
                });
            let codex_reason = self
                .codex_creation_reason
                .lock()
                .expect("Codex capability mutex")
                .clone();
            let codex = codex_reason.is_none()
                && record.source.as_ref().is_some_and(|source| {
                    source.snapshot.no_focus().is_ok()
                        && record.coarse_shell.as_ref().is_some_and(|coarse| {
                            self.config.projects.iter().any(|p| {
                                self.validate_coarse(source, coarse, Provider::Codex, &p.id)
                                    .is_ok()
                            })
                        })
                });
            let reason = record.reason.clone().unwrap_or_else(|| if !self.config.allow_scoped_shell_probe { "Creation disabled: explicit local allow_scoped_shell_probe consent is absent".into() } else if record.coarse_shell.is_none() { "Creation disabled: current shell executable/version, reporting namespace and integration views are unproved".into() } else if !omp && !codex { format!("{}; {}", omp_reason.unwrap_or_else(|| "OMP coarse creation proof unavailable".into()), codex_reason.unwrap_or_else(|| "Codex coarse creation proof unavailable".into())) } else { "Creation requires this action's once-only fresh-root shell proof; capabilities are attempt eligibility, not Ready".into() });
            servers.push(Server {
                server_id: record.server_id.clone(),
                name: record.name.clone(),
                connectivity: if ready {
                    ServerConnectivity::Online
                } else if record.source.is_some() {
                    ServerConnectivity::Offline
                } else {
                    ServerConnectivity::Unavailable
                },
                can_create: CanCreate { omp, codex },
                reason: Some(reason),
            });
            if let Some(source) = &record.source {
                let count = source.snapshot.workspaces.len()
                    + source.snapshot.tabs.len()
                    + source.snapshot.panes.len();
                let graph = if count > 512 - nodes {
                    NativeHierarchy {
                        server_id: source.server_id.clone(),
                        server_incarnation: source.incarnation.clone(),
                        freshness: NativeFreshness::Unavailable,
                        reason: Some(if ready {
                            NativeUnavailableReason::TooLarge
                        } else {
                            NativeUnavailableReason::SourceUnavailable
                        }),
                        workspaces: Vec::new(),
                        tabs: Vec::new(),
                        panes: Vec::new(),
                    }
                } else {
                    nodes += count;
                    source
                        .snapshot
                        .hierarchy(&source.server_id, &source.incarnation, ready)
                };
                hierarchy.push(graph);
                for agent in &source.snapshot.agents {
                    let provider = match agent.agent.as_deref() {
                        Some("omp") => Provider::Omp,
                        Some("codex") => Provider::Codex,
                        _ => continue,
                    };
                    if inner.bindings.values().any(|b| {
                        b.key.server_id == source.server_id
                            && b.key.server_incarnation == source.incarnation
                            && b.key.terminal_id == agent.terminal_id
                            && b.herdr_report.as_ref().is_some_and(|(kind, native)| {
                                *kind == agent.agent && *native == agent.agent_session
                            })
                            && self
                                .state
                                .validate_source(&b.target, &b.source_epoch)
                                .is_ok()
                    }) {
                        continue;
                    }
                    if rows.len() + inner.bindings.len() >= 512 {
                        return Err(ErrorCode::ResourceLimit);
                    }
                    let key = (
                        source.server_id.clone(),
                        source.incarnation.clone(),
                        agent.terminal_id.clone(),
                        provider,
                    );
                    seen.insert(key.clone());
                    rows.push((key, agent.clone(), ready));
                }
            }
        }
        for (key, agent, ready) in rows {
            let row_id = inner
                .unbound_ids
                .entry(key.clone())
                .or_insert_with(Uuid::random)
                .clone();
            let native_reason = inner.unbound_reasons.get(&key).cloned();
            unbound.push(UnboundRow {
                row_id,
                server_id: key.0,
                workspace_id: agent.workspace_id,
                terminal_id: agent.terminal_id,
                pane_id: agent.pane_id,
                provider: key.3,
                title: native_label(agent.title.as_deref(), &mut None)
                    .map(|v| v.0)
                    .unwrap_or_else(|| match key.3 {
                        Provider::Omp => "OMP".into(),
                        Provider::Codex => "Codex".into(),
                    }),
                state: native_reason.as_ref().map(|v| v.0).unwrap_or(if ready {
                    UnboundState::SetupRequired
                } else {
                    UnboundState::Unavailable
                }),
                reason: native_reason.map(|v| v.1).unwrap_or_else(|| {
                    if ready {
                        "Native owner/conversation/terminal binding has not been proved".into()
                    } else {
                        "Herdr source evidence is unavailable".into()
                    }
                }),
            });
        }
        inner.unbound_ids.retain(|key, _| seen.contains(key));
        inner.unbound_reasons.retain(|key, _| seen.contains(key));
        let mut association_epochs = Vec::new();
        for graph in hierarchy
            .iter_mut()
            .filter(|h| h.freshness == NativeFreshness::Current)
        {
            let source = inner
                .servers
                .get(&graph.server_id)
                .and_then(|r| r.source.as_ref())
                .ok_or(ErrorCode::NotReady)?;
            let agents: HashMap<_, _> = source
                .snapshot
                .agents
                .iter()
                .map(|a| (&a.terminal_id, a))
                .collect();
            for pane in &mut graph.panes {
                let Some(agent) = agents.get(&pane.terminal_id) else {
                    continue;
                };
                let provider = match agent.agent.as_deref() {
                    Some("omp") => Provider::Omp,
                    Some("codex") => Provider::Codex,
                    _ => continue,
                };
                let proven = inner.bindings.values().find(|b| {
                    b.key.server_id == graph.server_id
                        && b.key.server_incarnation == graph.server_incarnation
                        && b.key.terminal_id == pane.terminal_id
                        && b.herdr_report.as_ref().is_some_and(|(kind, native)| {
                            *kind == agent.agent && *native == agent.agent_session
                        })
                        && self
                            .state
                            .validate_source(&b.target, &b.source_epoch)
                            .is_ok()
                });
                if let Some(binding) = proven {
                    pane.target = Some(binding.target.clone());
                    association_epochs.push((&binding.target, &binding.source_epoch));
                } else {
                    pane.unbound_row_id = inner
                        .unbound_ids
                        .get(&(
                            graph.server_id.clone(),
                            graph.server_incarnation.clone(),
                            pane.terminal_id.clone(),
                            provider,
                        ))
                        .cloned();
                }
            }
        }
        let captured: HashMap<_, _> = inner
            .servers
            .values()
            .filter_map(|r| r.source.as_ref())
            .map(|s| (&s.server_id, s))
            .collect();
        let shells=self.shell_inventory(&inner);
        for graph in &mut hierarchy {for pane in &mut graph.panes {
            if pane.target.is_none() && pane.unbound_row_id.is_none() {
                pane.shell_target=shells.iter().find(|(summary,_)|summary.server_id==graph.server_id && summary.pane_id==pane.pane_id && summary.terminal_id==pane.terminal_id && summary.workspace_id==pane.workspace_id && summary.tab_id==pane.tab_id && summary.connectivity==SessionConnectivity::Ready).map(|(summary,_)|summary.target.clone());
            }
        }}
        self.state.replace_inventory(
            servers,
            self.config.projects(),
            shells,
            unbound,
            hierarchy,
            &association_epochs,
            |graphs| {
                for h in graphs {
                    let source = captured.get(&h.server_id).ok_or(ErrorCode::NotReady)?;
                    source.fence_inventory(h)?;
                }
                Ok(())
            },
        )
    }
    async fn unavailable(&self, server: &Uuid, reason: &str) -> SecurityResult<()> {
        let codex_reporter = self
            .codex_invalidation_reporter
            .lock()
            .ok()
            .and_then(|current| current.clone());
        let omp_reporter = self
            .omp_offline_reporter
            .lock()
            .ok()
            .and_then(|current| current.clone());
        {
            let mut inner = self.inner.lock().expect("topology mutex");
            let bindings: Vec<_> = inner
                .bindings
                .values()
                .filter(|binding| binding.key.server_id == *server)
                .map(|binding| {
                    (
                        binding.target.clone(),
                        binding.source_epoch.clone(),
                        binding.key.clone(),
                    )
                })
                .collect();
            if let Some(record) = inner.servers.get_mut(server) {
                if record.reason.as_deref() == Some(reason) {
                    return Ok(());
                }
                if let Some(reporter) = &codex_reporter {
                    for (target, epoch, _) in &bindings {
                        reporter.target(
                            target,
                            epoch,
                            "herdr",
                            "effective_topology_unavailable",
                            ErrorCode::NotReady,
                            "none",
                        );
                    }
                }
                if let Some(reporter) = &omp_reporter {
                    for (target, epoch, key) in &bindings {
                        reporter.offline_binding(
                            target,
                            epoch,
                            key,
                            OmpOfflineFailure::new(
                                "herdr",
                                "effective_topology_loss",
                                ErrorCode::NotReady,
                                "none",
                            ),
                        );
                    }
                }
                if let Some(source) = &record.source {
                    source.invalidation.fetch_add(1, Ordering::AcqRel);
                }
                record.reason = Some(reason.into());
            }
            let incarnation = inner
                .servers
                .get(server)
                .and_then(|s| s.source.as_ref())
                .map(|s| s.incarnation.clone());
            inner
                .proofs
                .retain(|p| incarnation.as_ref() != Some(&p.server_incarnation));
        }
        self.state.topology_offline(server, reason)?;
        self.publish()
    }
    async fn apply_snapshot(
        &self,
        server: &Uuid,
        evidence: ServerEvidence,
        snapshot: Snapshot,
        revision: u64,
        invalidation: Arc<AtomicU64>,
        event_dirty: bool,
    ) -> SecurityResult<(u64, bool)> {
        let _update = self.updates.lock().await;
        if invalidation.load(Ordering::Acquire) != revision {
            return Err(ErrorCode::NotReady);
        }
        let mut revision = revision;
        let slot = {
            let inner = self.inner.lock().expect("topology mutex");
            if inner.servers.values().any(|s| {
                &s.server_id != server
                    && s.source.as_ref().is_some_and(|source| {
                        source.recheck().is_ok() && source.evidence == evidence
                    })
            }) {
                return Err(ErrorCode::NotReady);
            }
            inner
                .servers
                .get(server)
                .ok_or(ErrorCode::NotFound)?
                .slot
                .clone()
        };
        let proof = evidence.clone();
        let identity = self
            .storage
            .call(move |s| s.server_identity(&slot, &proof))
            .await
            .map_err(pairing::store_error)?;
        if &identity.server_id != server {
            return Err(ErrorCode::NotReady);
        }
        let silently_changed = {
            let inner = self.inner.lock().expect("topology mutex");
            let current = inner
                .servers
                .get(server)
                .and_then(|record| record.source.as_ref());
            if let Some(current) = current
                && Arc::ptr_eq(&current.invalidation, &invalidation)
                && current.evidence == evidence
                && current.incarnation == identity.incarnation
                && current.revision == revision
            {
                if !event_dirty
                    && current.snapshot.as_ref() == &snapshot
                    && current.recheck().is_ok()
                {
                    return Ok((revision, false));
                }
                current.snapshot.as_ref() != &snapshot
            } else {
                false
            }
        };
        if silently_changed {
            invalidation
                .compare_exchange(revision, revision + 1, Ordering::AcqRel, Ordering::Acquire)
                .map_err(|_| ErrorCode::NotReady)?;
            revision += 1;
            self.state.topology_synchronizing(server)?;
        }
        {
            let mut inner = self.inner.lock().expect("topology mutex");
            if invalidation.load(Ordering::Acquire) != revision {
                return Err(ErrorCode::NotReady);
            }
            let record = inner.servers.get_mut(server).ok_or(ErrorCode::NotFound)?;
            record.source = Some(Source {
                server_id: server.clone(),
                incarnation: identity.incarnation,
                socket_path: record.path.clone(),
                evidence,
                snapshot: Arc::new(snapshot),
                revision,
                invalidation,
            });
            record.reason = None;
            record.coarse_shell = None;
        }
        let current = self.source(server)?;
        self.state.reconcile_grid_source(&current);
        let bindings: Vec<_> = self
            .inner
            .lock()
            .expect("topology mutex")
            .bindings
            .values()
            .filter(|b| b.key.server_id == *server)
            .map(|b| {
                (
                    b.target.clone(),
                    b.source_epoch.clone(),
                    b.key.server_incarnation.clone(),
                    b.herdr_report.clone(),
                    b.terminal_read_pin.clone(),
                    b.key.clone(),
                )
            })
            .collect();
        let mut display_updates = Vec::new();
        let codex_reporter = self
            .codex_invalidation_reporter
            .lock()
            .ok()
            .and_then(|current| current.clone());
        let omp_reporter = self
            .omp_offline_reporter
            .lock()
            .ok()
            .and_then(|current| current.clone());
        for (target, epoch, incarnation, report, terminal_read_pin, key) in bindings {
            if let Ok(display) = self.state.session_state(&target, &epoch) {
                if incarnation != current.incarnation {
                    if let Some(reporter) = &codex_reporter {
                        reporter.target(
                            &target,
                            &epoch,
                            "herdr",
                            "incarnation_changed",
                            ErrorCode::StaleSession,
                            "none",
                        );
                    }
                    if let Some(reporter) = &omp_reporter {
                        reporter.offline_binding(
                            &target,
                            &epoch,
                            &key,
                            OmpOfflineFailure::new(
                                "herdr",
                                "incarnation_changed",
                                ErrorCode::StaleSession,
                                "none",
                            ),
                        );
                    }
                    self.state.binding_offline(
                        &target,
                        &epoch,
                        "Herdr incarnation changed; old native identity remains offline",
                    )?;
                    continue;
                }
                let reported = current
                    .snapshot
                    .agents
                    .iter()
                    .find(|a| a.terminal_id == display.summary.terminal_id)
                    .map(|a| (a.agent.clone(), a.agent_session.clone()));
                if report.is_some() && report != reported {
                    if let Some(reporter) = &codex_reporter {
                        reporter.native_report(&target, &epoch, "herdr", &report, &reported);
                    }
                    if let Some(reporter) = &omp_reporter {
                        reporter.offline_binding(
                            &target,
                            &epoch,
                            &key,
                            OmpOfflineFailure::new(
                                "herdr",
                                "native_report_changed",
                                ErrorCode::StaleSession,
                                "none",
                            ),
                        );
                    }
                    self.state.binding_offline(
                        &target,
                        &epoch,
                        "Herdr native-session report changed; old proven identity remains offline",
                    )?;
                    continue;
                }
                if let Some(pane) = current
                    .snapshot
                    .panes
                    .iter()
                    .find(|p| p.terminal_id == display.summary.terminal_id)
                {
                    if pane.read_identity != terminal_read_pin {
                        self.state.binding_offline(
                            &target,
                            &epoch,
                            "Herdr terminal-read identity changed; old native identity remains offline",
                        )?;
                        continue;
                    }
                    if let Err(code) = self.state.topology_revalidated(
                        &target,
                        &epoch,
                        &pane.workspace_id,
                        &pane.pane_id,
                        || current.recheck().map_err(policy),
                    ) {
                        if let Some(reporter) = &omp_reporter {
                            reporter.offline_binding(
                                &target,
                                &epoch,
                                &key,
                                OmpOfflineFailure::new(
                                    "herdr",
                                    "effective_topology_loss",
                                    code,
                                    "none",
                                ),
                            );
                        }
                        return Err(code);
                    }
                    if display.summary.workspace_id != pane.workspace_id
                        || display.summary.pane_id != pane.pane_id
                    {
                        let retained = self.state.session_state(&target, &epoch)?;
                        display_updates.push((key, retained));
                    }
                } else {
                    if let Some(reporter) = &codex_reporter {
                        reporter.target(
                            &target,
                            &epoch,
                            "herdr",
                            "terminal_absent",
                            ErrorCode::StaleSession,
                            "none",
                        );
                    }
                    if let Some(reporter) = &omp_reporter {
                        reporter.offline_binding(
                            &target,
                            &epoch,
                            &key,
                            OmpOfflineFailure::new(
                                "herdr",
                                "terminal_absent",
                                ErrorCode::StaleSession,
                                "none",
                            ),
                        );
                    }
                    self.state.binding_offline(
                        &target,
                        &epoch,
                        "Previously proven terminal is absent from authoritative topology",
                    )?;
                }
            }
        }
        for (key, retained) in display_updates {
            self.storage
                .call(move |s| s.remember_session_display(&key, &retained))
                .await
                .map_err(pairing::store_error)?;
        }
        self.update_shells(&current).await?;
        self.publish()?;
        Ok((revision, true))
    }
    /// Discovery is periodic; subscriptions continuously drain independently of refreshes.
    /// Stop drops owned listeners/readers only, never Herdr/native desktop resources.
    pub async fn run(self: Arc<Self>, mut stop: watch::Receiver<bool>) -> io::Result<()> {
        let mut jobs: HashMap<Uuid, (tokio::task::JoinHandle<()>, watch::Sender<()>)> =
            HashMap::new();
        let result = loop {
            if *stop.borrow() {
                break Ok(());
            }
            match discover(&self.config).await {
                Ok(slots) => {
                    let wanted: HashSet<_> = slots
                        .iter()
                        .map(|s| serde_json::to_string(&(&s.socket_path, &s.name)))
                        .collect::<Result<_, _>>()?;
                    let obsolete: Vec<_> = self
                        .inner
                        .lock()
                        .expect("topology mutex")
                        .servers
                        .values()
                        .filter(|s| !wanted.contains(&s.slot))
                        .map(|s| (s.server_id.clone(), s.slot.clone()))
                        .collect();
                    for (server, slot) in obsolete {
                        if let Some((job, _)) = jobs.remove(&server) {
                            job.abort();
                            let _ = job.await;
                        }
                        self.unavailable(&server, "Herdr logical slot is no longer discovered")
                            .await
                            .map_err(|_| limit())?;
                        let removed = self
                            .storage
                            .call(move |s| s.retire_server_mapping(&slot, false, pairing::now_ms()))
                            .await
                            .map_err(store_io)?;
                        if removed {
                            self.inner
                                .lock()
                                .expect("topology mutex")
                                .servers
                                .remove(&server);
                        }
                    }
                    let mut present = HashSet::new();
                    for slot in slots {
                        let logical =
                            serde_json::to_string(&(slot.socket_path.clone(), slot.name.clone()))?;
                        let name =
                            Label::new(&slot.name).map_err(|_| invalid("invalid slot name"))?;
                        let label = name.clone();
                        let key = logical.clone();
                        let id = self
                            .storage
                            .call(move |s| s.enroll_server(&key, &label))
                            .await
                            .map_err(store_io)?;
                        present.insert(id.clone());
                        self.inner
                            .lock()
                            .expect("topology mutex")
                            .servers
                            .entry(id.clone())
                            .or_insert(ServerRecord {
                                slot: logical,
                                name,
                                path: slot.socket_path.clone(),
                                server_id: id.clone(),
                                source: None,
                                reason: Some(
                                    "Herdr process/socket/namespace evidence is not established"
                                        .into(),
                                ),
                                coarse_shell: None,
                            });
                        if let Some((job, refresh)) = jobs.get(&id)
                            && !job.is_finished()
                        {
                            refresh.send_replace(());
                            continue;
                        }
                        let owner = self.clone();
                        let server = id.clone();
                        let rx = stop.clone();
                        let (refresh, refresh_rx) = watch::channel(());
                        jobs.insert(
                            id,
                            (
                                tokio::spawn(async move {
                                    owner.follow(server, slot.socket_path, rx, refresh_rx).await;
                                }),
                                refresh,
                            ),
                        );
                    }
                    for (id, (job, _)) in &jobs {
                        if !present.contains(id) {
                            job.abort();
                            let _ = self
                                .unavailable(id, "Herdr source is absent from discovery")
                                .await;
                        }
                    }
                    self.publish().map_err(|_| limit())?;
                }
                Err(error) => {
                    let ids: Vec<_> = self
                        .inner
                        .lock()
                        .expect("topology mutex")
                        .servers
                        .keys()
                        .cloned()
                        .collect();
                    if error.kind() == io::ErrorKind::OutOfMemory {
                        break Err(error);
                    }
                    for (_, (job, _)) in jobs.drain() {
                        job.abort();
                        let _ = job.await;
                    }
                    for id in ids {
                        let _ = self
                            .unavailable(&id, "Configured Herdr discovery is unavailable")
                            .await;
                    }
                }
            }
            tokio::select! { _ = stop.changed() => {}, _ = tokio::time::sleep(Duration::from_secs(5)) => {} }
        };
        self.terminal_stopping.store(true,Ordering::Release);
        // A permit covers precommit registration AND the actual job. Late SQLite callbacks
        // cannot escape shutdown merely because their original transport waiter disappeared.
        let _all_terminal_jobs=self.terminal_slots.clone().acquire_many_owned(shells::CREATION_JOBS).await.map_err(|_|limit())?;
        {let mut terminal_jobs=self.terminal_jobs.lock().expect("terminal creation jobs");while terminal_jobs.try_join_next().is_some(){}}
        for (_, (job, _)) in jobs {
            job.abort();
            let _ = job.await;
        }
        result
    }
    async fn follow(
        &self,
        server: Uuid,
        path: PathBuf,
        mut stop: watch::Receiver<bool>,
        mut refresh: watch::Receiver<()>,
    ) {
        while !*stop.borrow() {
            if let Err(error) = self.recover(&server, &path, &mut stop, &mut refresh).await {
                let reason = if error.kind() == io::ErrorKind::OutOfMemory {
                    "Herdr topology exceeds the finite inventory/frame limits"
                } else {
                    "Herdr socket/process/namespace or subscription evidence is unavailable"
                };
                let _ = self.unavailable(&server, reason).await;
            }
            tokio::select! { _ = stop.changed() => {}, _ = tokio::time::sleep(Duration::from_secs(1)) => {} }
        }
    }
    async fn recover(
        &self,
        server: &Uuid,
        path: &Path,
        stop: &mut watch::Receiver<bool>,
        refresh: &mut watch::Receiver<()>,
    ) -> io::Result<()> {
        let invalidation = Arc::new(AtomicU64::new(0));
        let codex_reporter = self.codex_invalidation_reporter.clone();
        let mut pane_ids = Vec::new();
        let mut event_dirty = true;
        loop {
            let mut subscription =
                Subscription::connect(path, &pane_ids)
                    .await
                    .inspect_err(|error| {
                        if let Ok(current) = codex_reporter.lock()
                            && let Some(reporter) = current.as_ref()
                        {
                            reporter.subscription_error(
                                server,
                                None,
                                "subscription_connect",
                                error,
                            );
                        }
                        self.report_omp_recovery_io(server, "subscription_connect", error)
                    })?;
            let evidence = subscription.evidence.clone();
            let (dirty, mut dirty_rx) = watch::channel(0u64);
            let counter = invalidation.clone();
            let state = self.state.clone();
            let dirty_server = server.clone();
            let drain_failure = Arc::new(Mutex::new(None));
            let recorded_failure = drain_failure.clone();
            let offline_reporter = self.omp_offline_reporter.clone();
            let event_codex_reporter = codex_reporter.clone();
            let event_evidence = evidence.clone();
            let drain = tokio::spawn(async move {
                let mut phase = "topology_event";
                let result: io::Result<()> = async {
                    loop {
                        phase = "topology_event";
                        let result = subscription.event().await;
                        if let Err(error) = &result {
                            if let Ok(current) = event_codex_reporter.lock()
                                && let Some(reporter) = current.as_ref()
                            {
                                reporter.subscription_error(
                                    &dirty_server,
                                    Some(&event_evidence),
                                    "subscription_event",
                                    error,
                                );
                            }
                            let observed = OmpOfflineFailure::new(
                                "herdr",
                                "subscription_event",
                                if error.kind() == io::ErrorKind::OutOfMemory {
                                    ErrorCode::ResourceLimit
                                } else {
                                    ErrorCode::NotReady
                                },
                                codex_invalidation_io(error).0,
                            );
                            // Latch before dirty notification permits secondary failures.
                            if let Ok(current) = offline_reporter.lock()
                                && let Some(reporter) = current.as_ref()
                            {
                                reporter.offline_server(&dirty_server, &event_evidence, observed);
                            }
                            if let Ok(mut failure) = recorded_failure.lock() {
                                *failure = Some(observed);
                            }
                        }
                        let version = counter.fetch_add(1, Ordering::AcqRel) + 1;
                        phase = "topology_sync";
                        if let Err(code) = state.topology_synchronizing(&dirty_server) {
                            if let Ok(current) = offline_reporter.lock()
                                && let Some(reporter) = current.as_ref()
                            {
                                reporter.offline_server(
                                    &dirty_server,
                                    &event_evidence,
                                    OmpOfflineFailure::new("herdr", "topology_sync", code, "none"),
                                );
                            }
                            return Err(limit());
                        }
                        let _ = dirty.send(version);
                        phase = "topology_event";
                        result?;
                    }
                }
                .await;
                (result, phase)
            });
            let drain = DrainTask {
                task: drain,
                failure: drain_failure,
            };
            let mut consumed_dirty = 0;
            let recovery: io::Result<()> = async {
            loop {
                let revision = invalidation.load(Ordering::Acquire);
                event_dirty |= take_dirty_notification(&mut dirty_rx, &mut consumed_dirty);
                refresh.borrow_and_update();
                let client = Client::connect(path)
                    .await
                    .inspect_err(|error| {
                        if let Ok(current) = codex_reporter.lock()
                            && let Some(reporter) = current.as_ref()
                        {
                            reporter.subscription_error(
                                server,
                                Some(&evidence),
                                "snapshot_connect",
                                error,
                            );
                        }
                        self.report_omp_recovery_io(server, "snapshot_connect", error)
                    })?;
                if client.evidence != evidence {
                    return Err(invalid("subscription/snapshot epochs disagree"));
                }
                let (_, snapshot) = tokio::select! {
                    result = client.snapshot() => result.inspect_err(|error| {
                        if let Ok(current) = codex_reporter.lock()
                            && let Some(reporter) = current.as_ref()
                        {
                            reporter.subscription_error(
                                server,
                                Some(&evidence),
                                "snapshot_read",
                                error,
                            );
                        }
                        self.report_omp_recovery_io(server, "snapshot_read", error)
                    })?,
                    _ = stop.changed() => return Ok(())
                };
                if invalidation.load(Ordering::Acquire) != revision {
                    continue;
                }
                let mut required: Vec<_> =
                    snapshot.panes.iter().map(|p| p.pane_id.clone()).collect();
                required.sort_by(|a, b| a.0.cmp(&b.0));
                if pane_ids != required {
                    let _update = self.updates.lock().await;
                    if invalidation
                        .compare_exchange(
                            revision,
                            revision + 1,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        )
                        .is_err()
                    {
                        continue;
                    }
                    self.state
                        .topology_synchronizing(server)
                        .map_err(|_| limit())?;
                    pane_ids = required;
                    break Ok(());
                }
                let (applied_revision, _) = match self
                    .apply_snapshot(
                        server,
                        evidence.clone(),
                        snapshot,
                        revision,
                        invalidation.clone(),
                        event_dirty,
                    )
                    .await
                {
                    Ok(applied) => applied,
                    Err(code) => {
                        if invalidation.load(Ordering::Acquire) != revision {
                            continue;
                        }
                        if let Ok(current) = codex_reporter.lock()
                            && let Some(reporter) = current.as_ref()
                        {
                            reporter.revalidation(server, &evidence, code);
                        }
                        return Err(if code == ErrorCode::ResourceLimit {
                            limit()
                        } else {
                            invalid("topology replacement failed")
                        });
                    }
                };
                // Missing proof can recover on the existing healthy discovery refresh,
                // even when apply_snapshot preserves the unchanged Source and bindings.
                self.refresh_coarse(server, applied_revision, &invalidation).await;
                if invalidation.load(Ordering::Acquire) != applied_revision {
                    continue;
                }
                event_dirty = false;
                tokio::select! {
                    changed = dirty_rx.changed() => {
                        if changed.is_err() {
                            if let Some(failure) = drain.failure() {
                                self.report_omp_recovery_failure(server, failure);
                            } else {
                                self.report_omp_recovery_failure(
                                    server,
                                    OmpOfflineFailure::new(
                                        "herdr",
                                        "subscription_ended",
                                        ErrorCode::NotReady,
                                        "none",
                                    ),
                                );
                            }
                            return Err(invalid("subscription ended"));
                        }
                        event_dirty = true;
                    },
                    changed = refresh.changed() => if changed.is_err() { return Err(invalid("discovery refresh owner ended")); },
                    _ = stop.changed() => return Ok(())
                }
                if drain.task.is_finished() {
                    if let Some(failure) = drain.failure() {
                        self.report_omp_recovery_failure(server, failure);
                    } else {
                        self.report_omp_recovery_failure(
                            server,
                            OmpOfflineFailure::new(
                                "herdr",
                                "subscription_ended",
                                ErrorCode::NotReady,
                                "none",
                            ),
                        );
                    }
                    return Err(invalid("Herdr events lost/reconnect required"));
                }
            }
            }.await;
            drain.stop().await;
            recovery?;
            if *stop.borrow() {
                return Ok(());
            }
        }
    }
}
fn take_dirty_notification(dirty: &mut watch::Receiver<u64>, consumed: &mut u64) -> bool {
    let version = *dirty.borrow_and_update();
    let changed = version != *consumed;
    *consumed = version;
    changed
}

struct DrainTask {
    task: tokio::task::JoinHandle<(io::Result<()>, &'static str)>,
    failure: Arc<Mutex<Option<OmpOfflineFailure>>>,
}
impl DrainTask {
    fn failure(&self) -> Option<OmpOfflineFailure> {
        self.failure.lock().ok().and_then(|failure| *failure)
    }
    async fn stop(mut self) {
        self.task.abort();
        let _ = (&mut self.task).await;
    }
}
impl Drop for DrainTask {
    fn drop(&mut self) {
        self.task.abort();
    }
}
pub fn empty_resources() -> CreationResources {
    CreationResources { checkout: None,
        workspace_id: None,
        pane_id: None,
        terminal_id: None,
        native_id: None,
        binding_id: None,
        session_id: None,
        generation: None,
        owner_started: None,
        relay_started: None,
    }
}
/// Validate known returned IDs independently, retaining partial resources on malformed/change
/// outcomes. Complete routing is required for a confirmed workspace stage.
fn workspace_resources(value: &Value,placement:Option<&Id>) -> (CreationResources,Option<Id>,bool) {
    let extract = |pointer: &str| {
        value
            .pointer(pointer)
            .and_then(Value::as_str)
            .and_then(|s| Id::new(s).ok())
    };
    let mut resources = empty_resources();
    let workspace=extract("/workspace/workspace_id");
    let root_workspace=extract("/root_pane/workspace_id");
    let tab_workspace=extract("/tab/workspace_id");
    let tab=extract("/tab/tab_id");
    let root_tab=extract("/root_pane/tab_id");
    let routed_workspace=if placement.is_some(){root_workspace.as_ref().or(tab_workspace.as_ref())}else{workspace.as_ref()};
    resources.pane_id = extract("/root_pane/pane_id");
    resources.terminal_id = extract("/root_pane/terminal_id");
    let complete = routed_workspace.is_some()
        && resources.pane_id.is_some()
        && resources.terminal_id.is_some()
        && root_workspace.as_ref()==routed_workspace
        && tab_workspace.as_ref()==routed_workspace
        && placement.is_none_or(|expected|routed_workspace==Some(expected))
        && tab.is_some()&&tab==root_tab;
    resources.workspace_id=if placement.is_some(){root_workspace.or(tab_workspace)}else{workspace};
    // A partial returned ID is a retained fact only. Admission still requires all
    // independent routing fields above, including the exact captured placement.
    (resources,tab.or(root_tab),complete)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn probe_deadline_observation_is_monotone_and_entry_budget_uses_exact_boundaries() {
        let mut observation=ProbeObservation::NotRead;
        for (bytes,expected) in [
            (b"" as &[u8],ProbeObservation::EmptyOnly),
            (b"partial" as &[u8],ProbeObservation::PartialSeen),
            (b"" as &[u8],ProbeObservation::PartialSeen),
            (b"complete\n" as &[u8],ProbeObservation::CompleteSeen),
            (b"partial" as &[u8],ProbeObservation::CompleteSeen),
            (b"" as &[u8],ProbeObservation::CompleteSeen),
        ] {observation.observe(bytes);assert_eq!(observation,expected);}
        let now=tokio::time::Instant::now();
        for (remaining,expected) in [
            (Duration::ZERO,"expired"),(Duration::from_secs(1),"le_1s"),
            (Duration::from_secs(1)+Duration::from_nanos(1),"le_5s"),
            (Duration::from_secs(5),"le_5s"),(Duration::from_secs(5)+Duration::from_nanos(1),"gt_5s"),
        ] {assert_eq!(probe_entry_budget(now,now+remaining),expected);}
        let reporter=OmpCreationReporter::new_backend();
        reporter.report_probe_deadline("loop_check",Some((ProbeObservation::NotRead,"expired")));
        reporter.report_probe_deadline("parsed_success",Some((ProbeObservation::CompleteSeen,"gt_5s")));
        assert_eq!(reporter.0.lock().unwrap().lines,1,"detail must not create another terminal line or dedupe key");
    }

    async fn native_once_authorized()->(iroh::Endpoint,iroh::Endpoint,Arc<Security>,Arc<Authorized>) {
        let host=crate::endpoint_builder(iroh::SecretKey::from_bytes(&[31;32]))
            .relay_mode(iroh::RelayMode::Disabled).clear_address_lookup().bind().await.unwrap();
        let phone=crate::endpoint_builder(iroh::SecretKey::from_bytes(&[32;32]))
            .relay_mode(iroh::RelayMode::Disabled).clear_address_lookup().bind().await.unwrap();
        let mut storage=Storage::memory([9;32],pairing::now_ms()).unwrap();
        let device=PairedDevice{device_id:Uuid::random(),endpoint_id:EndpointId(phone.id().to_string()),
            device_name:Label::new("phone").unwrap(),paired_at:Uint(pairing::now_ms()),last_seen_at:None};
        storage.pair_device(&device).unwrap();
        let boot=storage.boot_id.clone();
        let security=Security::new(StorageOwner::new(storage).unwrap(),Arc::new(State::default()),
            EndpointId(host.id().to_string()),boot,vec![]);
        let endpoint=host.clone();
        let incoming=tokio::spawn(async move{endpoint.accept().await.unwrap().await.unwrap()});
        let connection=phone.connect(host.addr(),crate::ALPN).await.unwrap();
        let server=incoming.await.unwrap();assert_eq!(server.remote_id(),phone.id());
        let auth=Arc::new(security.hello(EndpointId(server.remote_id().to_string()),
            HelloParams{app_version:"test".into(),device_name:Label::new("phone").unwrap()},server).await.unwrap());
        drop(connection);
        (host,phone,security,auth)
    }

#[tokio::test]
async fn stock_herdr_workspace_cwd_probe_and_launch_are_receipt_once_only_without_native_ready_claims()
{
    use serde_json::{Value, json};
    use std::{
        os::unix::fs::PermissionsExt,
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
    };
    use tokio::{
        io::BufReader,
        net::UnixListener,
        sync::{Mutex, watch},
        task::JoinSet,
    };
    let (host, phone, security, auth) = native_once_authorized().await;
    let root = PathBuf::from(std::env::var_os("HOME").unwrap())
        .join(format!(".gc-{}", &Uuid::random().0[..8]));
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let socket = root.join("h.sock");
    let wrapper = root.join("omp");
    let herdr = root.join("herdr");
    let interpreter = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|p| p.join("bash"))
        .find(|p| p.is_file())
        .expect("owned GNU Bash fixture is required");
    fs::write(&wrapper, format!("#!{}\nexit 99\n", interpreter.display())).unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
    let listing = json!({"sessions":[{"name":"owned","default":false,"running":true,"socket_path":socket,"session_dir":root}]});
    fs::write(
        &herdr,
        format!(
            "#!{}\nprintf '%s\\n' '{}'\n",
            interpreter.display(),
            listing
        ),
    )
    .unwrap();
    fs::set_permissions(&herdr, fs::Permissions::from_mode(0o700)).unwrap();
    let registry = root.join("r");
    fs::create_dir(&registry).unwrap();
    fs::set_permissions(&registry, fs::Permissions::from_mode(0o700)).unwrap();
    let project = Uuid::random();
    let config=Config::parse(&format!("version=1\nruntime_dir={root:?}\nruntime_dir_agent={root:?}\nallow_scoped_shell_probe=true\nomp_registry_dir={registry:?}\nomp_session_roots=[{root:?}]\n[executables]\nherdr={herdr:?}\nomp={wrapper:?}\n[[projects]]\nid={:?}\nlabel=\"owned\"\npath={root:?}\n",project.0)).unwrap();
    fn shell(executable: &std::path::Path, root: &std::path::Path) -> tokio::process::Child {
        tokio::process::Command::new(executable)
            .args(["--noprofile", "--norc", "-i"])
            .current_dir(root)
            .env("PATH", root)
            .env("HOME", root)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap()
    }
    let mut old_shell = shell(&interpreter, &root);
    let mut new_shell = shell(&interpreter, &root);
    let old_pid = old_shell.id().unwrap();
    let new_pid = new_shell.id().unwrap();
    let stdin = Arc::new(Mutex::new(new_shell.stdin.take().unwrap()));
    let snapshot = Arc::new(Mutex::new(
        json!({"version":"0.9.1","protocol":22,"focused_workspace_id":"desktop","workspaces":[{"workspace_id":"desktop","number":1,"label":"Desktop","focused":true,"active_tab_id":"desktop-tab","agent_status":"unknown"}],"tabs":[{"tab_id":"desktop-tab","workspace_id":"desktop","number":1,"label":"Desktop tab","focused":true,"agent_status":"unknown"}],"panes":[{"pane_id":"desktop-pane","terminal_id":"desktop-terminal","workspace_id":"desktop","tab_id":"desktop-tab","focused":false,"agent_status":"unknown"}],"agents":[]}),
    ));
    let listener = UnixListener::bind(&socket).unwrap();
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
    let workspaces = Arc::new(AtomicUsize::new(0));
    let launches = Arc::new(AtomicUsize::new(0));
    let probes = Arc::new(AtomicUsize::new(0));
    let (event, event_rx) = watch::channel(None::<Value>);
    let (view, writes, starts, queries, input, project_path) = (
        snapshot.clone(),
        workspaces.clone(),
        launches.clone(),
        probes.clone(),
        stdin.clone(),
        root.clone(),
    );
    let fixture = tokio::spawn(async move {
        let mut clients = JoinSet::new();
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let (view, writes, starts, queries, input, project_path) = (
                view.clone(),
                writes.clone(),
                starts.clone(),
                queries.clone(),
                input.clone(),
                project_path.clone(),
            );
            let mut events = event_rx.clone();
            clients.spawn(async move {
                let mut reader=BufReader::new(stream);let request:Value=serde_json::from_slice(&read_line(&mut reader).await.unwrap()).unwrap();let id=request["id"].clone();let p=&request["params"];
                let result=match request["method"].as_str().unwrap() {
                    "events.subscribe"=>{reader.get_mut().write_all(format!("{}\n",json!({"id":id,"result":{"type":"subscription_started"}})).as_bytes()).await.unwrap();loop{if events.changed().await.is_err(){return;}let value=events.borrow().clone();if let Some(value)=value&&reader.get_mut().write_all(format!("{value}\n").as_bytes()).await.is_err(){return;}}}
                    "session.snapshot"=>json!({"type":"session_snapshot","snapshot":view.lock().await.clone()}),
                    "pane.process_info"=>{let pid=if p["pane_id"]=="created-pane"{new_pid}else{old_pid};json!({"type":"pane_process_info","process_info":{"pane_id":p["pane_id"],"shell_pid":pid,"foreground_process_group_id":pid,"foreground_processes":[{"pid":pid,"name":"bash"}]}})}
                    "workspace.create"=>{writes.fetch_add(1,Ordering::SeqCst);assert_eq!(p["focus"],false);assert_eq!(p["cwd"],json!(project_path));let root_pane=json!({"pane_id":"created-pane","terminal_id":"created-terminal","workspace_id":"created-workspace","tab_id":"created-tab","focused":false,"agent_status":"unknown"});let workspace=json!({"workspace_id":"created-workspace","number":2,"label":"Created workspace","focused":false,"active_tab_id":"created-tab","agent_status":"unknown"});let tab=json!({"tab_id":"created-tab","workspace_id":"created-workspace","number":1,"label":"Created tab","focused":false,"agent_status":"unknown"});let mut view=view.lock().await;view["workspaces"].as_array_mut().unwrap().push(workspace.clone());view["tabs"].as_array_mut().unwrap().push(tab.clone());view["panes"].as_array_mut().unwrap().push(root_pane.clone());assert_eq!(view["focused_workspace_id"],"desktop");json!({"type":"workspace_created","workspace":workspace,"tab":tab,"root_pane":root_pane})}
                    "pane.send_input"=>{queries.fetch_add(1,Ordering::SeqCst);assert_eq!(p["pane_id"],"created-pane");assert_eq!(p["keys"],json!(["Enter"]));let text=p["text"].as_str().unwrap();assert!(text.len()<=8192&&!text.chars().any(char::is_control));let mut input=input.lock().await;input.write_all(text.as_bytes()).await.unwrap();input.write_all(b"\n").await.unwrap();input.flush().await.unwrap();json!({"type":"ok"})}
                    "agent.start"=>{starts.fetch_add(1,Ordering::SeqCst);assert_eq!(p["kind"],"omp");assert_eq!(p["pane_id"],"created-pane");assert_eq!(p["timeout_ms"],30000);assert!(p.get("executable").is_none());let alias=p["name"].as_str().unwrap();assert!(alias.starts_with("goatr-omp-")&&alias.len()<=32);let mut argv=vec![json!("omp")];argv.extend(p["args"].as_array().unwrap().iter().cloned());json!({"type":"agent_started","agent":{"pane_id":"created-pane","terminal_id":"created-terminal","workspace_id":"created-workspace","tab_id":"created-tab","agent":"omp","name":alias,"launch_pending":true,"interactive_ready":false},"argv":argv})}
                    method=>panic!("unexpected native fixture method {method}")
                };
                reader.get_mut().write_all(format!("{}\n",json!({"id":id,"result":result})).as_bytes()).await.unwrap();
            });
        }
    });
    let relay = crate::backends::omp::Relay::bind(0).await.unwrap();
    let overlay = crate::backends::omp::overlay(&config, relay.port()).unwrap();
    let owner = Topology::new(
        config,
        security.storage.clone(),
        security.state.clone(),
        security.host.clone(),
    )
    .await
    .unwrap();
    let (stop, rx) = watch::channel(false);
    let relay_job = tokio::spawn(relay.run(stop.subscribe()));
    owner.omp_availability(None).unwrap();
    let job = tokio::spawn(owner.clone().run(rx));
    let server = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let request = Uuid::random();
            security
                .state
                .host_snapshot(&auth.connection_id, request.clone())
                .unwrap();
            let snapshot = loop {
                let (Some(bytes), closed) = security.state.pop_output(&auth.connection_id).unwrap()
                else {
                    panic!("host.snapshot response required")
                };
                assert!(!closed);
                if let Envelope::Response(response) =
                    decode_payload(&bytes, Some(Method::HostSnapshot)).unwrap()
                    && response.id == request
                {
                    let Ok(ResultBody::HostSnapshot(snapshot)) = response.outcome else {
                        panic!("host.snapshot result required")
                    };
                    break snapshot;
                }
            };
            assert!(
                snapshot.sessions.is_empty(),
                "creation eligibility is not native readiness"
            );
            if let Some(server) = snapshot.servers.iter().find(|server| {
                server.connectivity == ServerConnectivity::Online && server.can_create.omp
            }) {
                assert!(
                    snapshot
                        .projects
                        .iter()
                        .any(|row| row.project_id == project && row.available)
                );
                break server.server_id.clone();
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("actual coarse shell/object-view preflight must settle");
    let params = SessionCreateParams { directory_ref: None, placement: None, boot_id: security.boot.clone(),
    server_id: server.clone(),
    project_id: project,
    provider: Provider::Omp,
    name: Label::new("new desktop").unwrap(), };
    let request = Request {
        id: Uuid::random(),
        body: RequestBody::SessionCreate(params.clone()),
    };
    // The original registered owner captures real selected-root authority before reserve.
    let selection=owner.capture_creation(&auth,&params).await.unwrap();
    let validator=owner.clone();
    let validated_selection=selection.clone();
    security
        .reserve(&auth, request.clone(), move |request| {
            validator.validate_create(request)?;
            let RequestBody::SessionCreate(params)=&request.body else{return Err(ErrorCode::InvalidRequest)};
            validated_selection.validate(&validator,params).map(|_|())
        }, |_| {})
        .await
        .unwrap();
    let workspace = owner
        .workspace_once(&security, &auth, &request.id, &params, &selection)
        .await
        .unwrap();
    assert_eq!(
        workspace
            .creation
            .as_ref()
            .unwrap()
            .resources
            .terminal_id
            .as_ref()
            .unwrap()
            .0,
        "created-terminal"
    );
    let args = vec![
        "--cwd".into(),
        root.to_str().unwrap().into(),
        "--config".into(),
        overlay.to_str().unwrap().into(),
    ];
    assert_eq!(
        owner
            .launch_once(&security, &auth, &request.id, &params, &selection, args.clone())
            .await
            .err(),
        Some(ErrorCode::NotReady)
    );
    assert_eq!(launches.load(Ordering::SeqCst), 0);
    event
        .send(Some(json!({"event":"pane_created","data":{}})))
        .unwrap();
    let prepared=owner.prepare_created_shell(
        &security,&auth,(&request.id,&params,&selection),
        tokio::time::Instant::now()+Duration::from_secs(10),None,
    ).await.unwrap().expect("new original shell and actual selected cwd must prepare before input");
    assert_eq!(prepared.shell.pid,new_pid);
    assert_eq!(probes.load(Ordering::SeqCst),0,"actual-cwd preparation must not send the fixed query");
    let probe=owner.shell_probe_once(&security,&auth,prepared).await.unwrap();
    assert_eq!(
        probe.creation.as_ref().unwrap().stage_state,
        StageState::Confirmed
    );
    assert_eq!(probes.load(Ordering::SeqCst), 1);
    assert!(owner.prepare_created_shell(
        &security,&auth,(&request.id,&params,&selection),
        tokio::time::Instant::now()+Duration::from_secs(10),None,
    ).await.unwrap().is_none(),"the existing probe intent is status-only, not another prepared input owner");
    assert_eq!(probes.load(Ordering::SeqCst), 1);
    let submitted = owner
        .launch_once(&security, &auth, &request.id, &params, &selection, args.clone())
        .await
        .unwrap();
    let progress = submitted.creation.as_ref().unwrap();
    assert_eq!(progress.stage, CreationStage::Launch);
    assert_eq!(progress.stage_state, StageState::Dispatching);
    assert!(progress.resources.native_id.is_none() && progress.resources.session_id.is_none());
    fs::remove_file(&socket).unwrap();
    owner
        .workspace_once(&security, &auth, &request.id, &params, &selection)
        .await
        .unwrap();
    owner
        .launch_once(&security, &auth, &request.id, &params, &selection, args)
        .await
        .unwrap();
    assert_eq!(workspaces.load(Ordering::SeqCst), 1);
    assert_eq!(launches.load(Ordering::SeqCst), 1);
    security
        .revoke(auth.proof.peer_id().clone(), None)
        .await
        .unwrap();
    assert_eq!(
        owner
            .workspace_once(&security, &auth, &request.id, &params, &selection)
            .await
            .err(),
        Some(ErrorCode::Unauthorized)
    );
    stop.send(true).unwrap();
    job.await.unwrap().unwrap();
    relay_job.await.unwrap().unwrap();
    fixture.abort();
    let _ = fixture.await;
    old_shell.kill().await.unwrap();
    new_shell.kill().await.unwrap();
    old_shell.wait().await.unwrap();
    new_shell.wait().await.unwrap();
    drop(owner);
    security.shutdown().await.unwrap();
    crate::close_endpoint(&phone).await.unwrap();
    crate::close_endpoint(&host).await.unwrap();
    fs::remove_dir_all(root).unwrap();
}

    #[test]
    fn stock_alias_confirmation_requires_effective_name_original_route_kind_and_session() {
        let original:Pane=serde_json::from_value(json!({"pane_id":"w1:p1","terminal_id":"terminal","workspace_id":"w1","tab_id":"w1:t1",
            "focused":false,"agent_status":"idle","agent":"omp","agent_session":{"source":"native","agent":"omp","kind":"session","value":"original"}})).unwrap();
        let reply:Agent=serde_json::from_value(json!({"pane_id":"w1:p1","terminal_id":"terminal","workspace_id":"w1","tab_id":"w1:t1",
            "name":"new-name","agent":"omp","agent_session":{"source":"native","agent":"omp","kind":"session","value":"original"}})).unwrap();
        let context=RenameContext{source_epoch:Uuid::random(),pane:original.pane_id.clone(),terminal:original.terminal_id.clone(),
            workspace:original.workspace_id.clone(),tab:original.tab_id.clone()};
        let name=AgentName("new-name".into());
        assert!(alias_ack_matches(&reply,&original,&context,&name));
        for changed in 0..8 {
            let mut wrong=reply.clone();
            match changed {
                0=>wrong.name=Some("other".into()),
                1=>wrong.pane_id=Id("w1:p2".into()),
                2=>wrong.terminal_id=Id("replacement-terminal".into()),
                3=>wrong.workspace_id=Id("w2".into()),
                4=>wrong.tab_id=Id("w1:t2".into()),
                5=>wrong.agent=Some("codex".into()),
                6=>wrong.agent_session.as_mut().unwrap().value="replacement-session".into(),
                _=>wrong.launch_pending=true,
            }
            assert!(!alias_ack_matches(&wrong,&original,&context,&name));
        }
    }
    #[test]
    fn stock_creation_replies_preserve_partial_ids_and_require_the_captured_workspace_and_tab() {
        let workspace=Id("w1".into());
        let tab=json!({
            "tab":{"workspace_id":"w1","tab_id":"w1:t2"},
            "root_pane":{"workspace_id":"w1","tab_id":"w1:t2","pane_id":"w1:p2","terminal_id":"terminal-two"}
        });
        let (known,tab_id,complete)=workspace_resources(&tab,Some(&workspace));
        assert!(complete);
        assert_eq!(known.workspace_id,Some(workspace.clone()));
        assert_eq!(known.pane_id,Some(Id("w1:p2".into())));
        assert_eq!(tab_id,Some(Id("w1:t2".into())));
        assert!(!workspace_resources(&tab,None).2,"a tab response is not a separate-workspace response");
        let mut different=tab.clone();
        different["root_pane"]["workspace_id"]=json!("w3");
        let (known,_,complete)=workspace_resources(&different,Some(&workspace));
        assert!(!complete);
        assert_eq!(known.workspace_id,Some(Id("w3".into())),"reported partial facts must not be silently replaced with the requested workspace");
        let mut partial=tab.clone();partial["root_pane"].as_object_mut().unwrap().remove("terminal_id");
        let (known,tab_id,complete)=workspace_resources(&partial,Some(&workspace));
        assert!(!complete);assert!(known.terminal_id.is_none());
        assert_eq!(known.pane_id,Some(Id("w1:p2".into())));assert_eq!(tab_id,Some(Id("w1:t2".into())));
        let mut wrong_tab=tab.clone();wrong_tab["root_pane"]["tab_id"]=json!("w1:t3");
        assert!(!workspace_resources(&wrong_tab,Some(&workspace)).2);
        let mut separate=tab;separate["workspace"]=json!({"workspace_id":"w1"});
        assert!(workspace_resources(&separate,None).2);
    }

    #[test]
    fn coarse_shell_capture_is_backend_only_once_and_reserves_native_slots() {
        for reporter in [OmpCreationReporter::new(), OmpCreationReporter::new_offline()] {
            reporter.report_coarse_shell(CoarseShellGuard::Installed, None, "none");
            assert_eq!(reporter.0.lock().unwrap().lines, 0);
            assert!(!reporter.0.lock().unwrap().coarse_shell_settled);
        }
        let reporter = OmpCreationReporter::new_backend();
        for _ in 0..20 {
            reporter.report("follower", "follower_other", Some(ErrorCode::NotReady));
        }
        reporter.report_coarse_shell(CoarseShellGuard::Installed, None, "none");
        reporter.report_coarse_shell(CoarseShellGuard::ShellAbsent, Some(ErrorCode::NotReady), "none");
        reporter.report_agent("ack_matched", None);
        reporter.report("omp_ready", "topology_not_ready", Some(ErrorCode::NotReady));
        let state = reporter.0.lock().unwrap();
        assert_eq!(state.lines, 3);
        assert!(state.coarse_shell_settled && state.backend_launch_emitted && state.backend_readiness_emitted);
        assert!(state.seen.contains(&("coarse_shell", "installed")));
        assert!(!state.seen.contains(&("coarse_shell", "shell_absent")));
    }

    #[test]
    fn omp_launch_capture_backend_only_reserves_native_slots_from_transport_progress() {
        let reporter = OmpCreationReporter::new_backend();
        for guard in [
            "dispatch_returned",
            "response_queued",
            "output_written_after_queue",
        ] {
            reporter.report("transport", guard, None);
        }
        let generic = [
            "follower_native_report_absent",
            "follower_native_report_wrong",
            "follower_registry_absent",
            "follower_registry_read",
            "follower_registry_snapshot",
            "follower_owner_join",
            "follower_ambiguous",
            "follower_history",
        ];
        for guard in generic {
            reporter.report("follower", guard, Some(ErrorCode::NotReady));
        }
        reporter.report_agent("ack_matched", None);
        reporter.report("omp_ready", "topology_not_ready", Some(ErrorCode::NotReady));
        reporter.report(
            "omp_ready",
            "native_report_absent",
            Some(ErrorCode::NotReady),
        );
        reporter.report(
            "omp_ready",
            "native_report_absent",
            Some(ErrorCode::NotReady),
        );
        reporter.report(
            "omp_ready",
            "native_report_wrong",
            Some(ErrorCode::NotReady),
        );
        reporter.report("omp_ready", "no_connection", Some(ErrorCode::NotReady));
        reporter.report_agent("unavailable", None);
        for guard in generic {
            reporter.report("follower", guard, Some(ErrorCode::NotReady));
        }

        let state = reporter.0.lock().unwrap();
        assert_eq!(state.lines, 8);
        assert_eq!(state.progress_lines, 1);
        assert!(state.backend_launch_emitted && state.backend_readiness_emitted);
        assert!(state.seen.contains(&("herdr_launch", "ack_matched")));
        assert!(state.seen.contains(&("omp_ready", "topology_not_ready")));
        assert!(state.seen.contains(&("omp_ready", "native_report_absent")));
        assert!(state.seen.contains(&("omp_ready", "native_report_wrong")));
        assert!(state.seen.contains(&("omp_ready", "no_connection")));
        assert!(state.seen.contains(&("herdr_launch", "unavailable")));
        assert!(!state.seen.contains(&("transport", "dispatch_returned")));
        assert!(!state.seen.contains(&("follower", "follower_ambiguous")));
    }

    #[test]
    fn omp_creation_capture_reserves_terminal_readiness_summary() {
        let reporter = OmpCreationReporter::new();
        for guard in [
            "dispatch_returned",
            "response_queued",
            "output_written_after_queue",
        ] {
            reporter.report("transport", guard, None);
        }
        reporter.report_agent("ack_matched", None);
        reporter.defer_creation_topology_not_ready();
        reporter.report(
            "omp_ready",
            "native_report_absent",
            Some(ErrorCode::NotReady),
        );
        reporter.report(
            "follower",
            "follower_native_report_absent",
            Some(ErrorCode::NotReady),
        );
        let agent: Agent = serde_json::from_value(json!({
            "terminal_id":"terminal", "workspace_id":"workspace", "tab_id":"tab",
            "pane_id":"pane", "agent":"omp", "interactive_ready":false
        }))
        .unwrap();
        reporter.creation_agent_observation(Some(&agent));
        reporter.creation_readiness_failed();
        reporter.creation_readiness_failed();

        let state = reporter.0.lock().unwrap();
        assert_eq!(state.lines, 3);
        assert_eq!(state.creation_nonterminal_lines, 2);
        assert_eq!(state.progress_lines, 1);
        assert!(state.creation_terminal_emitted);
        assert!(state.seen.contains(&("herdr_launch", "ack_matched")));
        assert!(state.seen.contains(&("omp_ready", "topology_not_ready")));
        assert!(
            state
                .seen
                .contains(&("omp_ready", "report_absent_without_interactive_ready"))
        );
        assert!(!state.seen.contains(&("omp_ready", "native_report_absent")));
        assert!(
            !state
                .seen
                .contains(&("follower", "follower_native_report_absent"))
        );
        assert!(!state.seen.contains(&("transport", "dispatch_returned")));
        assert!(!state.seen.contains(&("transport", "response_queued")));
        assert!(
            !state
                .seen
                .contains(&("transport", "output_written_after_queue"))
        );
    }

    #[test]
    fn omp_creation_capture_preserves_first_follower_failure_before_terminal_summary() {
        let reporter = OmpCreationReporter::new();
        reporter.report_agent("ack_matched", None);
        reporter.defer_creation_topology_not_ready();
        reporter.report(
            "follower",
            "follower_registry_read",
            Some(ErrorCode::NotReady),
        );
        reporter.report(
            "follower",
            "follower_registry_snapshot",
            Some(ErrorCode::NotReady),
        );
        let agent: Agent = serde_json::from_value(json!({
            "terminal_id":"terminal", "workspace_id":"workspace", "tab_id":"tab",
            "pane_id":"pane", "agent":"omp", "interactive_ready":false
        }))
        .unwrap();
        reporter.creation_agent_observation(Some(&agent));
        reporter.creation_readiness_failed();

        let state = reporter.0.lock().unwrap();
        assert_eq!(state.lines, 3);
        assert_eq!(state.creation_nonterminal_lines, 2);
        assert!(state.seen.contains(&("herdr_launch", "ack_matched")));
        assert!(state.seen.contains(&("follower", "follower_registry_read")));
        assert!(
            !state
                .seen
                .contains(&("follower", "follower_registry_snapshot"))
        );
        assert!(!state.seen.contains(&("omp_ready", "topology_not_ready")));
        assert!(
            state
                .seen
                .contains(&("omp_ready", "report_absent_without_interactive_ready"))
        );
    }

    #[test]
    fn omp_creation_capture_reserves_terminal_shell_probe_failure() {
        let reporter = OmpCreationReporter::new();
        reporter.report_agent("ack_matched", None);
        reporter.report(
            "follower",
            "follower_registry_read",
            Some(ErrorCode::NotReady),
        );
        reporter.report(
            "follower",
            "follower_registry_snapshot",
            Some(ErrorCode::NotReady),
        );
        reporter.report_shell_probe("root_focus_at_deadline", ErrorCode::NotReady, "none");
        reporter.report_shell_probe("shell_not_idle", ErrorCode::NotReady, "none");

        let state = reporter.0.lock().unwrap();
        assert_eq!(state.lines, 3);
        assert_eq!(state.creation_nonterminal_lines, 2);
        assert!(state.creation_terminal_emitted);
        assert!(state.seen.contains(&("herdr_launch", "ack_matched")));
        assert!(state.seen.contains(&("follower", "follower_registry_read")));
        assert!(
            !state
                .seen
                .contains(&("follower", "follower_registry_snapshot"))
        );
        assert!(
            state
                .seen
                .contains(&("shell_probe", "root_focus_at_deadline"))
        );
        assert!(!state.seen.contains(&("shell_probe", "shell_not_idle")));
    }

    #[test]
    fn omp_creation_capture_reports_fatal_topology_immediately() {
        let reporter = OmpCreationReporter::new();
        reporter.report("omp_ready", "topology_not_ready", Some(ErrorCode::NotReady));

        let state = reporter.0.lock().unwrap();
        assert_eq!(state.lines, 1);
        assert_eq!(state.creation_nonterminal_lines, 1);
        assert!(!state.creation_topology_not_ready_pending);
        assert!(state.seen.contains(&("omp_ready", "topology_not_ready")));
    }

    #[test]
    fn codex_subscription_capture_keeps_the_original_server_evidence_latch_available() {
        let reporter = CodexInvalidationReporter::new();
        let target = Target {
            session_id: Uuid::random(),
            generation: Uuid::random(),
        };
        let epoch = Uuid::random();
        let server = Uuid::random();
        let evidence = ServerEvidence {
            linux_boot_id: Uuid::random(),
            pid: 1,
            pid_start_ticks: 1,
            socket_device: 1,
            socket_inode: 1,
        };
        reporter.arm(&target, &epoch, &server, &evidence);

        let foreign_server = Uuid::random();
        let mut foreign_evidence = evidence.clone();
        foreign_evidence.pid_start_ticks += 1;
        reporter.subscription_error(
            &foreign_server,
            Some(&evidence),
            "subscription_event",
            &io::Error::from(io::ErrorKind::BrokenPipe),
        );
        reporter.subscription_error(
            &server,
            Some(&foreign_evidence),
            "snapshot_read",
            &io::Error::from(io::ErrorKind::BrokenPipe),
        );
        assert!(!reporter.0.lock().unwrap().invalidated);

        // A retry/replacement mismatch is silent, leaving the first original I/O
        // failure able to win the same finite reporter latch.
        reporter.revalidation(&server, &foreign_evidence, ErrorCode::NotReady);
        assert!(!reporter.0.lock().unwrap().invalidated);
        reporter.subscription_error(
            &server,
            Some(&evidence),
            "subscription_event",
            &io::Error::from(io::ErrorKind::BrokenPipe),
        );
        let state = reporter.0.lock().unwrap();
        assert!(state.invalidated);
        assert_eq!(state.lines, 1);
        drop(state);
        reporter.target(
            &target,
            &epoch,
            "herdr",
            "effective_topology_unavailable",
            ErrorCode::NotReady,
            "none",
        );
        assert_eq!(reporter.0.lock().unwrap().lines, 1);
    }

    #[test]
    fn codex_revalidation_capture_uses_only_the_selected_propagation() {
        let reporter = CodexInvalidationReporter::new();
        let target = Target {
            session_id: Uuid::random(),
            generation: Uuid::random(),
        };
        let epoch = Uuid::random();
        let server = Uuid::random();
        let evidence = ServerEvidence {
            linux_boot_id: Uuid::random(),
            pid: 1,
            pid_start_ticks: 1,
            socket_device: 1,
            socket_inode: 1,
        };
        reporter.arm(&target, &epoch, &server, &evidence);
        reporter.revalidation(&server, &evidence, ErrorCode::NotReady);
        let state = reporter.0.lock().unwrap();
        assert!(state.invalidated);
        assert_eq!(state.lines, 1);
    }

    #[test]
    fn omp_creation_capture_latches_selected_agent_state_until_terminal_settlement() {
        let reporter = OmpCreationReporter::new();
        let pending: Agent = serde_json::from_value(json!({
            "terminal_id":"terminal", "workspace_id":"workspace", "tab_id":"tab",
            "pane_id":"pane", "agent":"omp", "interactive_ready":false
        }))
        .unwrap();
        let mut interactive = pending.clone();
        interactive.interactive_ready = true;

        reporter.creation_agent_observation(Some(&pending));
        reporter.creation_agent_observation(Some(&interactive));
        reporter.creation_agent_observation(None);
        reporter.creation_readiness_failed();

        let state = reporter.0.lock().unwrap();
        assert!(state.creation_interactive_ready_seen);
        assert_eq!(state.creation_original_agent_present, Some(false));
        assert!(state.creation_terminal_emitted);
        assert!(state.seen.contains(&("omp_ready", "original_agent_absent")));
    }

    #[test]
    fn omp_creation_capture_reports_interactive_report_absence_only_at_settlement() {
        let reporter = OmpCreationReporter::new();
        let agent: Agent = serde_json::from_value(json!({
            "terminal_id":"terminal", "workspace_id":"workspace", "tab_id":"tab",
            "pane_id":"pane", "agent":"omp", "interactive_ready":true
        }))
        .unwrap();
        reporter.creation_agent_observation(Some(&agent));
        reporter.creation_readiness_failed();

        let state = reporter.0.lock().unwrap();
        assert!(state.creation_terminal_emitted);
        assert!(
            state
                .seen
                .contains(&("omp_ready", "report_absent_after_interactive_ready"))
        );
    }

    #[test]
    fn connection_output_reporter_pins_first_connection_and_deduplicates_failure() {
        let reporter = OmpCreationReporter::new();
        let original = Uuid::random();
        let replacement = Uuid::random();
        reporter.transport_failure(
            &original,
            "reader_frame",
            Some(ErrorCode::NotReady),
            "unexpected_eof",
            "frame_payload",
        );
        assert_eq!(reporter.0.lock().unwrap().lines, 0);

        reporter.arm_transport(&original);
        reporter.arm_transport(&replacement);
        reporter.transport_failure(
            &replacement,
            "output_write",
            None,
            "connection_reset",
            "write_payload",
        );
        assert_eq!(reporter.0.lock().unwrap().lines, 0);

        reporter.transport_failure(
            &original,
            "reader_frame",
            Some(ErrorCode::NotReady),
            "unexpected_eof",
            "frame_payload",
        );
        reporter.transport_failure(
            &original,
            "output_write",
            None,
            "connection_reset",
            "write_payload",
        );
        let state = reporter.0.lock().unwrap();
        assert_eq!(state.transport_connection.as_ref(), Some(&original));
        assert!(state.transport_emitted);
        assert_eq!(state.lines, 1);
        assert!(state.seen.contains(&("transport", "reader_frame")));
    }

    #[test]
    fn offline_transport_failure_requires_the_ready_original_connection() {
        let reporter = OmpCreationReporter::new_offline();
        let original_connection = Uuid::random();
        let replacement_connection = Uuid::random();
        {
            let mut state = reporter.0.lock().unwrap();
            state.ready = false;
            state.original = Some(OmpOfflineOriginal {
                target: Target {
                    session_id: Uuid::random(),
                    generation: Uuid::random(),
                },
                epoch: Uuid::random(),
                server_id: Uuid::random(),
                incarnation: Uuid::random(),
                evidence: ServerEvidence {
                    linux_boot_id: Uuid::random(),
                    pid: 1,
                    pid_start_ticks: 1,
                    socket_device: 1,
                    socket_inode: 1,
                },
                terminal_id: Id("terminal".into()),
            });
        }
        reporter.arm_transport(&original_connection);
        reporter.arm_transport(&replacement_connection);
        reporter.transport_failure(
            &replacement_connection,
            "reader_frame",
            Some(ErrorCode::NotReady),
            "unexpected_eof",
            "frame_payload",
        );
        assert!(!reporter.0.lock().unwrap().invalidated);

        reporter.transport_failure(
            &original_connection,
            "reader_frame",
            None,
            "not_connected",
            "frame_header",
        );
        reporter.transport_failure(
            &original_connection,
            "output_write",
            None,
            "connection_reset",
            "write_payload",
        );
        let (target, epoch) = {
            let state = reporter.0.lock().unwrap();
            assert_eq!(state.lines, 0);
            let pending = state
                .pending
                .expect("first failure retained before Ready confirmation");
            assert_eq!(pending.code, None);
            assert_eq!(pending.frame, "frame_header");
            assert_eq!(pending.io, "not_connected");
            let original = state.original.as_ref().unwrap();
            (original.target.clone(), original.epoch.clone())
        };
        reporter.confirm_offline(&target, &epoch);
        let state = reporter.0.lock().unwrap();
        assert!(state.invalidated && state.transport_emitted);
        assert_eq!(state.lines, 1);
        assert!(state.seen.contains(&("transport", "reader_frame")));
        assert!(!state.seen.contains(&("transport", "output_write")));
    }

    #[test]
    fn omp_offline_reporter_pins_original_binding_and_reports_only_first_failure() {
        let target = Target {
            session_id: Uuid::random(),
            generation: Uuid::random(),
        };
        let epoch = Uuid::random();
        let server = Uuid::random();
        let incarnation = Uuid::random();
        let terminal = Id("terminal".into());
        let key = StableSessionKey {
            host_id: EndpointId("host".into()),
            server_id: server.clone(),
            server_incarnation: incarnation.clone(),
            terminal_id: terminal.clone(),
            provider_owner_identity: Id("omp-owner".into()),
            native_conversation_id: Id("native".into()),
        };
        let reporter = OmpCreationReporter::new_offline();
        {
            let mut state = reporter.0.lock().unwrap();
            state.original = Some(OmpOfflineOriginal {
                target: target.clone(),
                epoch: epoch.clone(),
                server_id: server,
                incarnation,
                evidence: ServerEvidence {
                    linux_boot_id: Uuid::random(),
                    pid: 1,
                    pid_start_ticks: 1,
                    socket_device: 1,
                    socket_inode: 1,
                },
                terminal_id: terminal,
            });
        }
        let foreign = Target {
            session_id: Uuid::random(),
            generation: Uuid::random(),
        };
        reporter.offline_binding(
            &foreign,
            &epoch,
            &key,
            OmpOfflineFailure::new("herdr", "terminal_absent", ErrorCode::StaleSession, "none"),
        );
        assert!(!reporter.0.lock().unwrap().invalidated);
        let foreign_source = StableSessionKey {
            server_incarnation: Uuid::random(),
            ..key.clone()
        };
        reporter.offline_binding(
            &target,
            &epoch,
            &foreign_source,
            OmpOfflineFailure::new("herdr", "terminal_absent", ErrorCode::StaleSession, "none"),
        );
        assert!(!reporter.0.lock().unwrap().invalidated);
        let evidence = {
            let state = reporter.0.lock().unwrap();
            state.original.as_ref().unwrap().evidence.clone()
        };
        let mut foreign_evidence = evidence.clone();
        foreign_evidence.pid_start_ticks += 1;
        reporter.offline_server(
            &key.server_id,
            &foreign_evidence,
            OmpOfflineFailure::new(
                "herdr",
                "subscription_event",
                ErrorCode::NotReady,
                "unexpected_eof",
            ),
        );
        assert!(!reporter.0.lock().unwrap().invalidated);
        // The event error must win before dirty notification can provoke a
        // secondary binding error, even while Ready publication is in progress.
        reporter.offline_server(
            &key.server_id,
            &evidence,
            OmpOfflineFailure::new(
                "herdr",
                "subscription_event",
                ErrorCode::NotReady,
                "unexpected_eof",
            ),
        );
        reporter.offline_binding(
            &target,
            &epoch,
            &key,
            OmpOfflineFailure::new("herdr", "terminal_absent", ErrorCode::StaleSession, "none"),
        );
        reporter.offline_binding(
            &target,
            &epoch,
            &key,
            OmpOfflineFailure::new("follower", "follower_source", ErrorCode::NotReady, "none"),
        );
        {
            let state = reporter.0.lock().unwrap();
            assert!(state.invalidated);
            assert_eq!(state.lines, 0);
            assert_eq!(state.pending.unwrap().guard, "subscription_event");
        }
        reporter.confirm_offline(&foreign, &epoch);
        assert_eq!(reporter.0.lock().unwrap().lines, 0);
        reporter.confirm_offline(&target, &epoch);
        let state = reporter.0.lock().unwrap();
        assert!(state.invalidated);
        assert_eq!(state.lines, 1);
        assert!(state.pending.is_none());
        assert!(state.seen.contains(&("herdr", "subscription_event")));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn late_event_gate_revalidates_equal_current_revision_without_native_loss() {
        let root = PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(format!(".gh-gate-{}", &Uuid::random().0[..8]));
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let path = root.join("h.sock");
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let evidence = Client::connect(&path).await.unwrap().evidence;
        let executable = fs::canonicalize(std::env::current_exe().unwrap()).unwrap();
        let config = Config::parse(&format!(
            "version=1\nruntime_dir={root:?}\nruntime_dir_agent={root:?}\n[executables]\nherdr={executable:?}\n"
        )).unwrap();
        let mut store = Storage::memory([16; 32], 0).unwrap();
        let device = PairedDevice {
            device_id: Uuid::random(),
            endpoint_id: EndpointId(iroh::SecretKey::from_bytes(&[16; 32]).public().to_string()),
            device_name: Label::new("private event fixture").unwrap(),
            paired_at: Uint(0),
            last_seen_at: None,
        };
        let paired_epoch = store.pair_device(&device).unwrap();
        let peer = store
            .authenticated_hello(&device.device_id, paired_epoch)
            .unwrap();
        let boot = store.boot_id.clone();
        let slot = serde_json::to_string(&(&path, "fixture")).unwrap();
        let server = store
            .enroll_server(&slot, &Label::new("fixture").unwrap())
            .unwrap();
        let storage = Arc::new(StorageOwner::new(store).unwrap());
        let state = Arc::new(State::default());
        let connection = state.connect(&peer);
        let host = EndpointId(iroh::SecretKey::from_bytes(&[17; 32]).public().to_string());
        let owner = Topology::new(config, storage.clone(), state.clone(), host.clone())
            .await
            .unwrap();
        assert!(owner.source_readiness(&server).unwrap().is_none());
        assert!(matches!(owner.source(&server), Err(ErrorCode::NotReady)));
        let snapshot: Snapshot = serde_json::from_value(json!({
            "version":"0.9.1","protocol":22,"focused_workspace_id":"workspace",
            "workspaces":[{"workspace_id":"workspace","number":1,"label":"fixture","focused":true,"active_tab_id":"tab","agent_status":"idle"}],
            "tabs":[{"tab_id":"tab","workspace_id":"workspace","number":1,"label":"fixture","focused":true,"agent_status":"idle"}],
            "panes":[{"pane_id":"pane","terminal_id":"terminal","workspace_id":"workspace","tab_id":"tab","focused":true,"agent_status":"idle","agent":"omp"}],
            "agents":[{"pane_id":"pane","terminal_id":"terminal","workspace_id":"workspace","tab_id":"tab","agent":"omp","agent_session":{"source":"test","agent":"omp","kind":"path","value":"/native.jsonl"},"interactive_ready":true}]
        })).unwrap();
        let invalidation = Arc::new(AtomicU64::new(0));
        owner
            .apply_snapshot(
                &server,
                evidence.clone(),
                snapshot.clone(),
                0,
                invalidation.clone(),
                true,
            )
            .await
            .unwrap();
        let source = owner.source(&server).unwrap();
        let ready_source = owner.source_readiness(&server).unwrap().unwrap();
        assert_eq!(ready_source.evidence, source.evidence);
        assert_eq!(ready_source.incarnation, source.incarnation);
        assert!(ready_source.recheck_readiness().unwrap());
        let native_owner = process_evidence(std::process::id()).unwrap();
        let key = StableSessionKey {
            host_id: host,
            server_id: server.clone(),
            server_incarnation: source.incarnation.clone(),
            terminal_id: Id::new("terminal").unwrap(),
            provider_owner_identity: Id::new(format!(
                "{}:{}",
                native_owner.pid, native_owner.start_ticks
            ))
            .unwrap(),
            native_conversation_id: Id::new("/native.jsonl").unwrap(),
        };
        let display: SessionState = serde_json::from_value(json!({
            "summary":{"sessionId":Uuid::random(),"generation":Uuid::random(),"serverId":server,
                "workspaceId":"workspace","terminalId":"terminal","paneId":"pane","provider":"omp",
                "nativeId":"/native.jsonl","title":"retained","status":"idle","connectivity":"ready",
                "capabilities":{"history":true,"prompt":true,"cancel":true,"respond":true,"promptMode":"submit_or_steer","cancelMode":"current_work"},
                "historyCoverage":{"scope":"all_branches","source":"persisted_and_live","olderAvailable":false,"sourceTruncated":false,"desktopBranchKnown":false}},
            "desktopSelection":"unknown","activity":{"state":"idle"}
        })).unwrap();
        let omp_reporter = OmpCreationReporter::new_offline();
        owner.set_omp_offline_reporter(omp_reporter.clone());
        let (target, epoch) = owner
            .begin_native(key.clone(), native_owner.clone(), display.clone(), false)
            .await
            .unwrap();
        omp_reporter.offline_binding(
            &target,
            &epoch,
            &key,
            OmpOfflineFailure::new("follower", "follower_source", ErrorCode::NotReady, "none"),
        );
        assert!(!omp_reporter.0.lock().unwrap().invalidated);
        let history = crate::state::SafeHistory::from_redacted_display(
            serde_json::from_value(json!({"id":"retained","order":0,"role":"assistant","kind":"message","text":{"kind":"text","text":"retained history"},"source":"live","complete":true,"sourceTruncated":false})).unwrap()
        ).unwrap();
        let dialog: PendingRequest = serde_json::from_value(json!({
            "requestKey":Uuid::random(),"provider":"omp","kind":"omp_editor",
            "presentation":{"kind":"omp_editor","title":{"kind":"text","text":"retained dialog"},"prefill":{"kind":"text","text":"retained draft"}}
        })).unwrap();
        let mut installed = display;
        installed.summary.session_id = target.session_id.clone();
        installed.summary.generation = target.generation.clone();
        owner
            .finish_native(
                &key,
                &native_owner,
                &epoch,
                ProviderSnapshot {
                    state: installed,
                    history: vec![history],
                    pending: vec![dialog.clone()],
                    older_available: false,
                    native_older: None,
                },
            )
            .await
            .unwrap();
        omp_reporter.offline_binding(
            &target,
            &epoch,
            &key,
            OmpOfflineFailure::new("follower", "follower_source", ErrorCode::NotReady, "none"),
        );
        assert!(omp_reporter.0.lock().unwrap().invalidated);
        let prompt = Request {
            id: Uuid::random(),
            body: RequestBody::PromptSubmit(PromptSubmitParams { delivery: None, native_turn_id: None, images: None, skill: None, boot_id: boot,
            target: target.clone(),
            text: PromptText::new("fixture admission").unwrap(), }),
        };
        state.validate_session_mutation(&prompt).unwrap();
        let retained_history = state
            .history(&peer, &target, None, 100, 0)
            .unwrap()
            .result
            .records;
        let revision_exposed = Arc::new(tokio::sync::Barrier::new(2));
        let release_gate = Arc::new(tokio::sync::Barrier::new(2));
        let (dirty, mut dirty_rx) = watch::channel(0u64);
        let late_gate = {
            let dirty = dirty.clone();
            let invalidation = invalidation.clone();
            let state = state.clone();
            let server = server.clone();
            let revision_exposed = revision_exposed.clone();
            let release_gate = release_gate.clone();
            tokio::spawn(async move {
                let revision = invalidation.fetch_add(1, Ordering::AcqRel) + 1;
                revision_exposed.wait().await;
                release_gate.wait().await;
                state.topology_synchronizing(&server).unwrap();
                dirty.send(revision).unwrap();
            })
        };
        tokio::time::timeout(Duration::from_secs(3), revision_exposed.wait())
            .await
            .unwrap();
        assert!(owner.source_readiness(&server).unwrap().is_none());
        assert!(!ready_source.recheck_readiness().unwrap());
        let pending = ready_source.recheck().unwrap_err();
        assert_eq!(pending.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            pending.to_string(),
            "Herdr topology requires authoritative refresh"
        );
        let (revision, replaced) = owner
            .apply_snapshot(
                &server,
                evidence.clone(),
                snapshot.clone(),
                1,
                invalidation.clone(),
                false,
            )
            .await
            .unwrap();
        assert_eq!(revision, 1);
        assert!(replaced);
        state.validate_session_mutation(&prompt).unwrap();
        let refreshed_source = owner.source_readiness(&server).unwrap().unwrap();
        assert_eq!(refreshed_source.evidence, ready_source.evidence);
        assert_eq!(refreshed_source.incarnation, ready_source.incarnation);
        state.validate_source(&target, &epoch).unwrap();
        release_gate.wait().await;
        late_gate.await.unwrap();
        dirty_rx.changed().await.unwrap();
        assert!(!dirty_rx.has_changed().unwrap());
        assert_eq!(
            state
                .session_state(&target, &epoch)
                .unwrap()
                .summary
                .connectivity,
            SessionConnectivity::Synchronizing
        );
        assert_eq!(
            state.validate_session_mutation(&prompt),
            Err(ErrorCode::NotReady)
        );
        let clean_source = owner.source(&server).unwrap();
        assert_eq!(clean_source.revision, 1);
        assert!(clean_source.recheck().is_ok());
        assert_eq!(
            owner
                .apply_snapshot(
                    &server,
                    evidence.clone(),
                    snapshot.clone(),
                    revision,
                    invalidation.clone(),
                    true,
                )
                .await
                .unwrap(),
            (1, true)
        );

        // A send between the old clean probe and consumption must not vanish.
        let send_window_revision = invalidation.fetch_add(1, Ordering::AcqRel) + 1;
        owner
            .apply_snapshot(
                &server,
                evidence.clone(),
                snapshot.clone(),
                send_window_revision,
                invalidation.clone(),
                false,
            )
            .await
            .unwrap();
        let mut consumed_dirty = 1;
        assert!(!dirty_rx.has_changed().unwrap());
        state.topology_synchronizing(&server).unwrap();
        dirty.send(send_window_revision).unwrap();
        let event_dirty = take_dirty_notification(&mut dirty_rx, &mut consumed_dirty);
        assert!(event_dirty);
        assert_eq!(consumed_dirty, send_window_revision);
        assert_eq!(
            state.validate_session_mutation(&prompt),
            Err(ErrorCode::NotReady)
        );
        owner
            .apply_snapshot(
                &server,
                evidence.clone(),
                snapshot.clone(),
                send_window_revision,
                invalidation.clone(),
                event_dirty,
            )
            .await
            .unwrap();
        state.validate_session_mutation(&prompt).unwrap();

        // A running synchronous old drain must finish before replacement commits.
        let old_gate_entered = Arc::new(std::sync::Barrier::new(2));
        let old_gate_release = Arc::new(std::sync::Barrier::new(2));
        let (old_dirty, old_dirty_rx) = watch::channel(0u64);
        drop(old_dirty_rx);
        let old_drain = {
            let counter = invalidation.clone();
            let state = state.clone();
            let server = server.clone();
            let entered = old_gate_entered.clone();
            let release = old_gate_release.clone();
            tokio::spawn(async move {
                let version = counter.fetch_add(1, Ordering::AcqRel) + 1;
                entered.wait();
                release.wait();
                let result = state.topology_synchronizing(&server).map_err(|_| limit());
                if result.is_ok() {
                    let _ = old_dirty.send(version);
                }
                (result, "topology_event")
            })
        };
        old_gate_entered.wait();
        let pane_revision = invalidation.load(Ordering::Acquire);
        invalidation
            .compare_exchange(
                pane_revision,
                pane_revision + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .unwrap();
        state.topology_synchronizing(&server).unwrap();
        let mut changed_panes = snapshot.clone();
        let mut added = changed_panes.panes[0].clone();
        added.pane_id = Id::new("additional-pane").unwrap();
        added.terminal_id = Id::new("additional-terminal").unwrap();
        changed_panes.panes.push(added);
        let replacement = {
            let owner = owner.clone();
            let server = server.clone();
            let evidence = evidence.clone();
            let invalidation = invalidation.clone();
            tokio::spawn(async move {
                DrainTask {
                    task: old_drain,
                    failure: Arc::new(Mutex::new(None)),
                }
                .stop()
                .await;
                owner
                    .apply_snapshot(
                        &server,
                        evidence,
                        changed_panes,
                        pane_revision + 1,
                        invalidation,
                        true,
                    )
                    .await
            })
        };
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(
            !replacement.is_finished(),
            "replacement passed a still-running old drain"
        );
        assert_eq!(
            state.validate_session_mutation(&prompt),
            Err(ErrorCode::NotReady)
        );
        old_gate_release.wait();
        let (_, replaced) = tokio::time::timeout(Duration::from_secs(3), replacement)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(replaced);
        state.validate_source(&target, &epoch).unwrap();
        state.validate_session_mutation(&prompt).unwrap();
        let restored = state.session_state(&target, &epoch).unwrap();
        assert_eq!(restored.summary.session_id, target.session_id);
        assert_eq!(restored.summary.generation, target.generation);
        assert_eq!(restored.summary.connectivity, SessionConnectivity::Ready);
        assert_eq!(
            state
                .history(&peer, &target, None, 100, 0)
                .unwrap()
                .result
                .records,
            retained_history
        );
        let request = Uuid::random();
        state
            .session_open(&connection, request.clone(), &target, 0)
            .unwrap();
        let mut offered = None;
        while let (Some(bytes), _) = state.pop_output(&connection).unwrap() {
            if let Envelope::Response(response) =
                decode_payload(&bytes, Some(Method::SessionOpen)).unwrap()
                && response.id == request
                && let Ok(ResultBody::SessionOpen(opened)) = response.outcome
            {
                offered = Some(opened.pending_requests);
            }
        }
        assert_eq!(offered.unwrap(), vec![dialog]);
        let current = owner.source_readiness(&server).unwrap().unwrap();
        fs::remove_file(&path).unwrap();
        assert_eq!(
            current.recheck_readiness().unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        assert!(matches!(
            owner.source_readiness(&server),
            Err(ErrorCode::NotReady)
        ));
        let replacement_socket = tokio::net::UnixListener::bind(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(current.recheck_readiness().is_err());
        assert!(matches!(
            owner.source_readiness(&server),
            Err(ErrorCode::NotReady)
        ));
        drop(replacement_socket);
        drop(owner);
        Arc::try_unwrap(storage)
            .ok()
            .unwrap()
            .close()
            .await
            .unwrap();
        drop(listener);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pending_launch_ack_preserves_exact_correlation_without_native_kind() {
        let mut agent: Agent = serde_json::from_value(json!({
            "terminal_id":"terminal", "workspace_id":"workspace", "tab_id":"tab",
            "pane_id":"pane", "name":"owned-launch",
            "launch_pending":true, "interactive_ready":false
        }))
        .unwrap();
        let pane = Id::new("pane").unwrap();
        let terminal = Id::new("terminal").unwrap();
        let workspace = Id::new("workspace").unwrap();
        let wrong = Id::new("wrong").unwrap();
        let matches = |agent: &Agent, argv_matches, kind| {
            launch_ack_matches(
                agent,
                argv_matches,
                kind,
                "owned-launch",
                &pane,
                &terminal,
                Some(&workspace),
            )
        };
        for kind in ["omp", "codex"] {
            assert!(
                matches(&agent, true, kind),
                "pending submitted ACK must not require native detection"
            );
            assert!(
                !matches(&agent, false, kind),
                "exact submitted argv remains mandatory"
            );
            assert!(!launch_ack_matches(
                &agent,
                true,
                kind,
                "other-alias",
                &pane,
                &terminal,
                Some(&workspace)
            ));
            assert!(!launch_ack_matches(
                &agent,
                true,
                kind,
                "owned-launch",
                &wrong,
                &terminal,
                Some(&workspace)
            ));
            assert!(!launch_ack_matches(
                &agent,
                true,
                kind,
                "owned-launch",
                &pane,
                &wrong,
                Some(&workspace)
            ));
            assert!(!launch_ack_matches(
                &agent,
                true,
                kind,
                "owned-launch",
                &pane,
                &terminal,
                Some(&wrong)
            ));
            assert!(!launch_ack_matches(
                &agent,
                true,
                kind,
                "owned-launch",
                &pane,
                &terminal,
                None
            ));
        }
        assert!(
            !agent.interactive_ready,
            "submission ACK does not manufacture native Ready"
        );
        agent.launch_pending = false;
        assert!(!matches(&agent, true, "omp"));
        agent.agent = Some("codex".into());
        assert!(
            !matches(&agent, true, "omp"),
            "non-pending wrong native kind stays rejected"
        );
        assert!(matches(&agent, true, "codex"));
        agent.agent = Some("omp".into());
        assert!(matches(&agent, true, "omp"));
    }

    #[test]
    fn provider_root_absolute_symlink_resolves_inside_root() {
        let path = PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(format!(".goatr-root-{}", Uuid::random().0));
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        let provider = path.join("provider");
        fs::create_dir(&provider).unwrap();
        let host_target = path.join("installed-wrapper");
        fs::write(&host_target, "host wrapper").unwrap();
        let provider_target = provider.join(host_target.strip_prefix("/").unwrap());
        fs::create_dir_all(provider_target.parent().unwrap()).unwrap();
        fs::write(&provider_target, "provider wrapper").unwrap();
        symlink(&host_target, provider.join("omp")).unwrap();
        let root = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC)
            .open(&provider)
            .unwrap();
        let actual = root_object(&root, Path::new("/omp")).unwrap();
        let confined = object(&provider_target).unwrap();
        let host = object(&host_target).unwrap();
        fs::remove_file(&provider_target).unwrap();
        let missing = root_object(&root, Path::new("/omp"));
        fs::remove_dir_all(&path).unwrap();
        assert_ne!(host, confined);
        assert_eq!(
            actual, confined,
            "absolute symlink escaped the provider root"
        );
        assert!(
            missing.is_err(),
            "missing provider target accepted the host object"
        );
    }
}

#[cfg(test)]
mod probe_tests {
    use super::*;
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    struct Root(PathBuf);
    impl Root {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("gp-{}", &Uuid::random().0[..8]));
            fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Root {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    fn executable(name: &str) -> PathBuf {
        std::env::split_paths(&std::env::var_os("PATH").expect("fixture PATH"))
            .map(|p| p.join(name))
            .find(|p| p.is_file())
            .unwrap_or_else(|| panic!("owned {name} resolver fixture is required"))
    }
    /// Actual private Unix frames and kernel process/shell evidence, with no admission bypass.
    struct CoarseRecoveryFixture {
        owner: Arc<Topology>,
        state: Arc<State>,
        connection: Uuid,
        server: Uuid,
        snapshot: Snapshot,
        snapshots: Arc<AtomicU64>,
        probes: Arc<AtomicU64>,
        probe_started: Arc<tokio::sync::Notify>,
        native: tokio::task::JoinHandle<()>,
        shell: tokio::process::Child,
        root: Root,
    }
    impl Drop for CoarseRecoveryFixture {
        fn drop(&mut self) {
            self.native.abort();
        }
    }
    impl CoarseRecoveryFixture {
        async fn new(consent: bool, missing_first: bool, pause: Option<tokio::sync::oneshot::Receiver<()>>) -> Self {
            let root = Root::new();
            let path = root.0.join("h.sock");
            let listener = tokio::net::UnixListener::bind(&path).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            let shell = tokio::process::Command::new(executable("bash"))
                .args(["--noprofile", "--norc"])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .kill_on_drop(true).spawn().unwrap();
            let shell_pid = shell.id().unwrap();
            let (wrapper, _) = shell_executable(&process_evidence(shell_pid).unwrap()).unwrap();
            let binary = fs::canonicalize(std::env::current_exe().unwrap()).unwrap();
            let runtime = &root.0;
            let mut config = Config::parse(&format!(
                "version=1\nruntime_dir={runtime:?}\nruntime_dir_agent={runtime:?}\nallow_scoped_shell_probe={consent}\n[executables]\nherdr={binary:?}\nomp={wrapper:?}\n"
            )).unwrap();
            config.projects.push(config::ConfiguredProject {
                allow_read_tools: false,
                id: Uuid::random(), label: Label::new("fixture").unwrap(), path: root.0.clone(),
            });
            let mut storage = Storage::memory([28; 32], 0).unwrap();
            let device = PairedDevice {
                device_id: Uuid::random(),
                endpoint_id: EndpointId(iroh::SecretKey::from_bytes(&[28; 32]).public().to_string()),
                device_name: Label::new("coarse recovery fixture").unwrap(),
                paired_at: Uint(0), last_seen_at: None,
            };
            let epoch = storage.pair_device(&device).unwrap();
            let peer = storage.authenticated_hello(&device.device_id, epoch).unwrap();
            let slot = serde_json::to_string(&(&path, "fixture")).unwrap();
            let server = storage.enroll_server(&slot, &Label::new("fixture").unwrap()).unwrap();
            let state = Arc::new(State::default());
            let connection = state.connect(&peer);
            let owner = Topology::new(config, Arc::new(StorageOwner::new(storage).unwrap()), state.clone(),
                EndpointId(iroh::SecretKey::from_bytes(&[29; 32]).public().to_string())).await.unwrap();
            // The real backend callback supplies attempt availability; this fixture launches no provider.
            owner.omp_availability(None).unwrap();
            let wire = json!({
                "version":"0.9.1","protocol":22,"focused_workspace_id":"workspace",
                "workspaces":[{"workspace_id":"workspace","number":1,"label":"fixture","focused":true,"active_tab_id":"tab","agent_status":"idle"}],
                "tabs":[{"tab_id":"tab","workspace_id":"workspace","number":1,"label":"fixture","focused":true,"agent_status":"idle"}],
                "panes":[{"pane_id":"pane","terminal_id":"terminal","workspace_id":"workspace","tab_id":"tab","focused":true,"agent_status":"idle","agent":"omp"}],
                "agents":[{"pane_id":"pane","terminal_id":"terminal","workspace_id":"workspace","tab_id":"tab","agent":"omp","agent_session":{"source":"fixture","agent":"omp","kind":"path","value":"/native.jsonl"},"interactive_ready":true}]
            });
            let snapshot: Snapshot = serde_json::from_value(wire.clone()).unwrap();
            snapshot.validate().unwrap();
            let snapshots = Arc::new(AtomicU64::new(0));
            let probes = Arc::new(AtomicU64::new(0));
            let pause = Arc::new(Mutex::new(pause));
            let probe_started = Arc::new(tokio::sync::Notify::new());
            let native = {
                let snapshots = snapshots.clone();
                let probes = probes.clone();
                let probe_started = probe_started.clone();
                tokio::spawn(async move {
                    let mut handlers = tokio::task::JoinSet::new();
                    loop {
                        let (stream, _) = listener.accept().await.unwrap();
                        let wire = wire.clone();
                        let snapshots = snapshots.clone();
                        let probes = probes.clone();
                        let pause = pause.clone();
                        let probe_started = probe_started.clone();
                        handlers.spawn(async move {
                            let mut reader = BufReader::new(stream);
                            // A connect used only for peer evidence legitimately sends no request.
                            let Ok(bytes) = read_line(&mut reader).await else { return; };
                            let request: Value = serde_json::from_slice(&bytes).unwrap();
                            let method = request["method"].as_str().unwrap();
                            let result = match method {
                                "events.subscribe" => json!({"type":"subscription_started"}),
                                "session.snapshot" => {
                                    snapshots.fetch_add(1, Ordering::SeqCst);
                                    json!({"type":"session_snapshot","snapshot":wire})
                                }
                                "pane.process_info" => {
                                    let number = probes.fetch_add(1, Ordering::SeqCst);
                                    let waiting = { pause.lock().unwrap().take() };
                                    if let Some(waiting) = waiting {
                                        probe_started.notify_one();
                                        waiting.await.unwrap();
                                    }
                                    let pid = if missing_first && number == 0 { None } else { Some(shell_pid) };
                                    json!({"type":"pane_process_info","process_info":{"pane_id":"pane","shell_pid":pid,"foreground_process_group_id":null,"tty":null,"foreground_processes":[]}})
                                }
                                _ => panic!("unexpected fixture native method"),
                            };
                            let mut reply = serde_json::to_vec(&json!({"id":request["id"],"result":result})).unwrap();
                            reply.push(b'\n');
                            reader.get_mut().write_all(&reply).await.unwrap();
                            if method == "events.subscribe" {
                                // Keep this real subscription healthy and quiet until its owner closes it.
                                let _ = read_line(&mut reader).await;
                            }
                        });
                    }
                })
            };
            Self { owner, state, connection, server, snapshot, snapshots, probes, probe_started, native, shell, root }
        }
        fn inventory(&self) -> HostSnapshotResult {
            let request = Uuid::random();
            self.state.host_snapshot(&self.connection, request.clone()).unwrap();
            while let (Some(bytes), _) = self.state.pop_output(&self.connection).unwrap() {
                if let Envelope::Response(response) = decode_payload(&bytes, Some(Method::HostSnapshot)).unwrap()
                    && response.id == request
                    && let Ok(ResultBody::HostSnapshot(snapshot)) = response.outcome
                {
                    return snapshot;
                }
            }
            panic!("fixture host snapshot missing")
        }
        fn recover(&self) -> (watch::Sender<bool>, watch::Sender<()>, tokio::task::JoinHandle<io::Result<()>>) {
            let (stop, mut stopped) = watch::channel(false);
            let (refresh, mut refreshed) = watch::channel(());
            let owner = self.owner.clone();
            let server = self.server.clone();
            let path = self.root.0.join("h.sock");
            let job = tokio::spawn(async move { owner.recover(&server, &path, &mut stopped, &mut refreshed).await });
            (stop, refresh, job)
        }
        async fn finish(mut self) {
            self.native.abort();
            let _ = (&mut self.native).await;
            self.shell.kill().await.unwrap();
            self.shell.wait().await.unwrap();
        }
    }
    async fn coarse_wait(mut ready: impl FnMut() -> bool) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !ready() { tokio::time::sleep(Duration::from_millis(10)).await; }
        }).await.unwrap();
    }

    #[tokio::test]
    async fn missing_coarse_proof_heals_on_unchanged_recovery_without_rebinding_or_reprobing() {
        let fixture = CoarseRecoveryFixture::new(true, true, None).await;
        let (stop, refresh, job) = fixture.recover();
        coarse_wait(|| fixture.probes.load(Ordering::SeqCst) == 1).await;
        let source = fixture.owner.source(&fixture.server).unwrap();
        assert!(!fixture.inventory().servers[0].can_create.omp);
        // Retain an installed provider Target/source epoch across the metadata-only healing.
        let native_owner = process_evidence(std::process::id()).unwrap();
        let key = StableSessionKey {
            host_id: fixture.owner.host_id.clone(), server_id: fixture.server.clone(),
            server_incarnation: source.incarnation.clone(), terminal_id: Id("terminal".into()),
            provider_owner_identity: Id(format!("{}:{}",native_owner.pid,native_owner.start_ticks)),
            native_conversation_id: Id("/native.jsonl".into()),
        };
        let mut display: SessionState = serde_json::from_value(json!({
            "summary":{"sessionId":Uuid::random(),"generation":Uuid::random(),"serverId":fixture.server,
                "workspaceId":"workspace","terminalId":"terminal","paneId":"pane","provider":"omp",
                "nativeId":"/native.jsonl","title":"retained","status":"idle","connectivity":"ready",
                "capabilities":{"history":true,"prompt":true,"cancel":true,"respond":true,"promptMode":"submit_or_steer","cancelMode":"current_work"},
                "historyCoverage":{"scope":"all_branches","source":"persisted_and_live","olderAvailable":false,"sourceTruncated":false,"desktopBranchKnown":false}},
            "desktopSelection":"unknown","activity":{"state":"idle"}
        })).unwrap();
        let (target, epoch) = fixture.owner.begin_native(key.clone(), native_owner.clone(), display.clone(), false).await.unwrap();
        display.summary.session_id = target.session_id.clone();
        display.summary.generation = target.generation.clone();
        fixture.owner.finish_native(&key, &native_owner, &epoch, ProviderSnapshot {
            state: display, history: vec![], pending: vec![], older_available: false, native_older: None,
        }).await.unwrap();
        // Same watch notification as the existing healthy five-second discovery owner.
        refresh.send_replace(());
        coarse_wait(|| fixture.inventory().servers[0].can_create.omp).await;
        assert_eq!(fixture.probes.load(Ordering::SeqCst), 2);
        let healed = fixture.owner.source(&fixture.server).unwrap();
        assert!(source.same_physical_identity(&healed));
        assert_eq!(source.revision, healed.revision);
        assert!(Arc::ptr_eq(&source.snapshot, &healed.snapshot));
        fixture.state.validate_source(&target, &epoch).unwrap();
        let retained = fixture.state.session_state(&target, &epoch).unwrap();
        assert_eq!(retained.summary.session_id, target.session_id);
        assert_eq!(retained.summary.generation, target.generation);
        // Reaching the following snapshot proves the preceding serialized refresh completed.
        let before = fixture.snapshots.load(Ordering::SeqCst);
        refresh.send_replace(());
        coarse_wait(|| fixture.snapshots.load(Ordering::SeqCst) > before).await;
        let next = fixture.snapshots.load(Ordering::SeqCst);
        refresh.send_replace(());
        coarse_wait(|| fixture.snapshots.load(Ordering::SeqCst) > next).await;
        assert_eq!(fixture.probes.load(Ordering::SeqCst), 2, "installed proof was reprobed");
        stop.send_replace(true);
        tokio::time::timeout(Duration::from_secs(5), job).await.unwrap().unwrap().unwrap();
        fixture.finish().await;
    }

    #[tokio::test]
    async fn consent_absent_unchanged_recovery_never_requests_process_info() {
        let fixture = CoarseRecoveryFixture::new(false, false, None).await;
        let (stop, refresh, job) = fixture.recover();
        coarse_wait(|| fixture.snapshots.load(Ordering::SeqCst) >= 2).await;
        for _ in 0..2 {
            let before = fixture.snapshots.load(Ordering::SeqCst);
            refresh.send_replace(());
            coarse_wait(|| fixture.snapshots.load(Ordering::SeqCst) > before).await;
        }
        assert_eq!(fixture.probes.load(Ordering::SeqCst), 0);
        assert!(!fixture.inventory().servers[0].can_create.omp);
        stop.send_replace(true);
        tokio::time::timeout(Duration::from_secs(5), job).await.unwrap().unwrap().unwrap();
        fixture.finish().await;
    }

    #[tokio::test]
    async fn in_flight_coarse_probe_cannot_install_into_equal_replaced_source() {
        for new_recovery_owner in [false, true] {
            let (release, waiting) = tokio::sync::oneshot::channel();
            let fixture = CoarseRecoveryFixture::new(true, false, Some(waiting)).await;
            let path = fixture.root.0.join("h.sock");
            let evidence = Client::connect(&path).await.unwrap().evidence;
            let original_owner = Arc::new(AtomicU64::new(0));
            fixture.owner.apply_snapshot(&fixture.server,evidence.clone(),fixture.snapshot.clone(),0,original_owner.clone(),true).await.unwrap();
            let original = fixture.owner.source(&fixture.server).unwrap();
            let probe = {
                let owner = fixture.owner.clone(); let server = fixture.server.clone(); let revision = original_owner.clone();
                tokio::spawn(async move { owner.refresh_coarse(&server,0,&revision).await })
            };
            tokio::time::timeout(Duration::from_secs(5), fixture.probe_started.notified()).await.unwrap();
            let replacement_owner = if new_recovery_owner { Arc::new(AtomicU64::new(0)) } else { original_owner.clone() };
            fixture.owner.apply_snapshot(&fixture.server,evidence,fixture.snapshot.clone(),0,replacement_owner.clone(),true).await.unwrap();
            assert!(original.recheck().is_ok(), "physical/revision check alone must not admit the stale completion");
            release.send(()).unwrap();
            tokio::time::timeout(Duration::from_secs(5), probe).await.unwrap().unwrap();
            assert!(fixture.owner.inner.lock().unwrap().servers[&fixture.server].coarse_shell.is_none());
            assert!(!fixture.inventory().servers[0].can_create.omp);
            // A later admitted observation can still heal; discarding stale I/O must not latch failure.
            fixture.owner.refresh_coarse(&fixture.server,0,&replacement_owner).await;
            assert!(fixture.inventory().servers[0].can_create.omp);
            assert_eq!(fixture.probes.load(Ordering::SeqCst),2);
            fixture.finish().await;
        }
    }

    #[tokio::test]
    async fn creation_admission_uses_current_same_source_root_and_rejects_changes() {
        let root = Root::new();
        let path = root.0.join("h.sock");
        let _listener = tokio::net::UnixListener::bind(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let evidence = Client::connect(&path).await.unwrap().evidence;
        let executable = fs::canonicalize(std::env::current_exe().unwrap()).unwrap();
        let runtime = &root.0;
        let config = Config::parse(&format!(
            "version=1\nruntime_dir={runtime:?}\nruntime_dir_agent={runtime:?}\n[executables]\nherdr={executable:?}\n"
        )).unwrap();
        let mut store = Storage::memory([18; 32], 0).unwrap();
        let slot = serde_json::to_string(&(&path, "fixture")).unwrap();
        let server = store
            .enroll_server(&slot, &Label::new("fixture").unwrap())
            .unwrap();
        let owner = Topology::new(
            config,
            Arc::new(StorageOwner::new(store).unwrap()),
            Arc::new(State::default()),
            EndpointId(iroh::SecretKey::from_bytes(&[19; 32]).public().to_string()),
        )
        .await
        .unwrap();
        let snapshot: Snapshot = serde_json::from_value(json!({
            "version":"0.9.1","protocol":22,"focused_workspace_id":"anchor",
            "workspaces":[
                {"workspace_id":"anchor","number":1,"label":"anchor","focused":true,"active_tab_id":"anchor-tab","agent_status":"idle"},
                {"workspace_id":"owned","number":2,"label":"owned","focused":false,"active_tab_id":"owned-tab","agent_status":"idle"}],
            "tabs":[
                {"tab_id":"anchor-tab","workspace_id":"anchor","number":1,"label":"anchor","focused":true,"agent_status":"idle"},
                {"tab_id":"owned-tab","workspace_id":"owned","number":1,"label":"owned","focused":false,"agent_status":"idle"}],
            "panes":[
                {"pane_id":"anchor-pane","terminal_id":"anchor-terminal","workspace_id":"anchor","tab_id":"anchor-tab","focused":true,"agent_status":"idle"},
                {"pane_id":"owned-pane","terminal_id":"owned-terminal","workspace_id":"owned","tab_id":"owned-tab","focused":false,"agent_status":"idle"}],
            "agents":[]
        })).unwrap();
        let invalidation = Arc::new(AtomicU64::new(0));
        snapshot.validate().unwrap();
        owner
            .apply_snapshot(
                &server,
                evidence.clone(),
                snapshot.clone(),
                0,
                invalidation.clone(),
                true,
            )
            .await
            .unwrap();
        let original = owner.source(&server).unwrap();
        let resources = CreationResources { checkout: None,
            workspace_id: Some(Id("owned".into())),
            pane_id: Some(Id("owned-pane".into())),
            terminal_id: Some(Id("owned-terminal".into())),
            ..empty_resources()
        };
        let focus = Id("anchor".into());
        owner
            .fresh_creation_root(&original, &resources, &focus)
            .unwrap();

        let revision = invalidation.fetch_add(1, Ordering::AcqRel) + 1;
        assert!(
            matches!(
                owner.fresh_creation_root(&original, &resources, &focus),
                Err(ErrorCode::NotReady)
            ),
            "dirty topology is never admission"
        );
        let mut unrelated = snapshot.clone();
        unrelated.workspaces[0].label = "updated anchor".into();
        owner
            .apply_snapshot(
                &server,
                evidence.clone(),
                unrelated,
                revision,
                invalidation.clone(),
                true,
            )
            .await
            .unwrap();
        assert!(
            original.recheck().is_err(),
            "captured revision must stay stale"
        );
        owner
            .fresh_creation_root(&original, &resources, &focus)
            .unwrap();

        // Each rejection uses an authoritative fresh revision, not the stale gate.
        for mutation in 0..8 {
            let mut changed = snapshot.clone();
            match mutation {
                0 => {
                    changed.panes[1].workspace_id = Id("anchor".into());
                    changed.panes[1].tab_id = Id("anchor-tab".into());
                }
                1 => changed.panes[1].terminal_id = Id("replacement".into()),
                2 => changed.focused_workspace_id = Some(Id("owned".into())),
                3 => changed.panes[1].focused = true,
                4 => changed.panes[1].agent = Some("omp".into()),
                5 => changed.panes[1].agent_status = NativeAgentStatus::Working,
                6 => changed.panes[1].pane_id = Id("replacement".into()),
                _ => changed.agents.push(serde_json::from_value(json!({
                    "pane_id":"owned-pane","terminal_id":"owned-terminal","workspace_id":"owned","tab_id":"owned-tab","launch_pending":true
                })).unwrap()),
            }
            changed.validate().unwrap();
            let revision = invalidation.fetch_add(1, Ordering::AcqRel) + 1;
            owner
                .apply_snapshot(
                    &server,
                    evidence.clone(),
                    changed,
                    revision,
                    invalidation.clone(),
                    true,
                )
                .await
                .unwrap();
            assert!(
                owner
                    .fresh_creation_root(&original, &resources, &focus)
                    .is_err(),
                "changed owned root {mutation} admitted"
            );
        }
        let revision = invalidation.fetch_add(1, Ordering::AcqRel) + 1;
        owner
            .apply_snapshot(
                &server,
                evidence,
                snapshot,
                revision,
                invalidation.clone(),
                true,
            )
            .await
            .unwrap();
        let current = owner.source(&server).unwrap();
        for mutation in 0..4 {
            let mut replacement = current.clone();
            match mutation {
                0 => replacement.incarnation = Uuid::random(),
                1 => replacement.evidence.pid_start_ticks += 1,
                2 => replacement.socket_path = root.0.join("absent.sock"),
                _ => replacement.invalidation = Arc::new(AtomicU64::new(replacement.revision)),
            }
            owner
                .inner
                .lock()
                .unwrap()
                .servers
                .get_mut(&server)
                .unwrap()
                .source = Some(replacement);
            assert!(
                owner
                    .fresh_creation_root(&original, &resources, &focus)
                    .is_err(),
                "replacement source {mutation} admitted"
            );
        }
        owner
            .inner
            .lock()
            .unwrap()
            .servers
            .get_mut(&server)
            .unwrap()
            .source = None;
        assert!(matches!(
            owner.fresh_creation_root(&original, &resources, &focus),
            Err(ErrorCode::NotReady)
        ));
    }
    #[tokio::test]
    async fn creation_shell_admission_rejects_changed_original_epoch() {
        let mut child = tokio::process::Command::new(executable("bash"))
            .args(["--noprofile", "--norc"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let shell = process_evidence(child.id().unwrap()).unwrap();
        assert_eq!(shell_executable(&shell).unwrap().1, ShellDialect::Bash);
        let mut changed = shell.clone();
        changed.start_ticks += 1;
        assert!(shell_executable(&changed).is_err());
        changed = shell.clone();
        changed.namespaces.mount.1 += 1;
        assert!(shell_executable(&changed).is_err());
        child.kill().await.unwrap();
        child.wait().await.unwrap();
        assert!(shell_executable(&shell).is_err());
    }
    #[test]
    fn shell_source_admission_is_exact_for_approved_output_packages() {
        let hash = "0123456789abcdfghijklmnpqrsvwxyz";
        for (package, name, dialect) in [
            ("bash-5.3p9", "bash", ShellDialect::Bash),
            ("bash-interactive-5.3p9", "bash", ShellDialect::Bash),
            ("zsh-5.9", "zsh", ShellDialect::Zsh),
            ("zsh-5.9.1", "zsh", ShellDialect::Zsh),
        ] {
            let path = format!("/nix/store/{hash}-{package}/bin/{name}");
            assert_eq!(shell_source_dialect(Path::new(&path)).unwrap(), dialect);
        }
        for (package, name) in [
            ("bash-5.3", "bash"),
            ("bash-5.3p8", "bash"),
            ("bash-5.3p10", "bash"),
            ("bash-interactive-5.3p15", "bash"),
            ("bash-static-5.3p9", "bash"),
            ("renamed-bash-5.3p9", "bash"),
            ("bash-5.3p9-extra", "bash"),
            ("zsh-5.9.0", "zsh"),
            ("zsh-5.9.10", "zsh"),
            ("zsh-5.9.1-extra", "zsh"),
            ("zsh-5.10", "zsh"),
            ("zsh-5.9.1", "bash"),
            ("bash-5.3p9", "renamed"),
        ] {
            let path = format!("/nix/store/{hash}-{package}/bin/{name}");
            assert!(shell_source_dialect(Path::new(&path)).is_err(), "{path}");
        }
        for path in [
            format!("/tmp/{hash}-zsh-5.9.1/bin/zsh"),
            format!("/nix/store/{hash}-zsh-5.9.1/libexec/zsh"),
            format!("/nix/store/{hash}-zsh-5.9.1/nested/bin/zsh"),
            "/nix/store/not-a-store-hash-zsh-5.9.1/bin/zsh".into(),
        ] {
            assert!(shell_source_dialect(Path::new(&path)).is_err(), "{path}");
        }
    }
    #[tokio::test]
    async fn shell_executable_rejects_unapproved_copy_and_replaced_opened_object() {
        let root = Root::new();
        let path = root.0.join("bash");
        fs::copy(executable("bash"), &path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        let mut child = tokio::process::Command::new(&path)
            .args(["--noprofile", "--norc"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let shell = process_evidence(child.id().unwrap()).unwrap();
        let opened = File::open(format!("/proc/{}/exe", shell.pid)).unwrap();
        let metadata = opened.metadata().unwrap();
        let identity = (metadata.dev(), metadata.ino());
        assert_eq!(identity, object(&path).unwrap());
        assert!(
            shell_executable(&shell).is_err(),
            "renamed ELF is not source proof"
        );
        let replacement = root.0.join("replacement");
        fs::copy(executable("bash"), &replacement).unwrap();
        fs::rename(&replacement, &path).unwrap();
        assert_ne!(identity, object(&path).unwrap());
        assert!(
            shell_executable(&shell).is_err(),
            "replaced source object was admitted"
        );
        child.kill().await.unwrap();
        child.wait().await.unwrap();
    }
    #[tokio::test]
    async fn bash_and_zsh_templates_measure_exact_quoted_outer_path_and_reject_aliases_functions_and_trace()
     {
        for (dialect, name) in [(ShellDialect::Bash, "bash"), (ShellDialect::Zsh, "zsh")] {
            for (setup, expected) in [
                ("", true),
                ("alias omp='false'", false),
                ("omp() { return 0; }", false),
                ("set -x", false),
            ] {
                let root = Root::new();
                let wrapper = root.0.join("out'er wrapper");
                fs::write(
                    &wrapper,
                    format!(
                        "#!/bin/sh\nprintf ran > '{}'\n",
                        root.0.join("must-not-run").display()
                    ),
                )
                .unwrap();
                fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
                let bin = root.0.join("omp");
                std::os::unix::fs::symlink(&wrapper, &bin).unwrap();
                let result = root.0.join("result");
                File::create(&result).unwrap();
                fs::set_permissions(&result, fs::Permissions::from_mode(0o600)).unwrap();
                let mut command = tokio::process::Command::new(executable(name));
                match dialect {
                    ShellDialect::Bash => {
                        command.args(["--noprofile", "--norc", "-i"]);
                    }
                    ShellDialect::Zsh => {
                        command.args(["-f", "-i"]);
                    }
                }
                let mut child = command
                    .env_clear()
                    .current_dir(&root.0)
                    .env("PATH", &root.0)
                    .env("HOME", &root.0)
                    .env("ZDOTDIR", &root.0)
                    .env("TERM", "dumb")
                    .env("LC_ALL", "C")
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .kill_on_drop(true)
                    .spawn()
                    .unwrap();
                let shell = process_evidence(child.id().unwrap()).unwrap();
                let (actual, actual_dialect) = shell_executable(&shell).unwrap();
                assert_eq!(actual_dialect, dialect);
                assert_eq!(actual, fs::canonicalize(executable(name)).unwrap());
                let mut wrong_epoch = shell.clone();
                wrong_epoch.start_ticks += 1;
                assert!(shell_executable(&wrong_epoch).is_err());
                let nonce = "5".repeat(64);
                // type/whence retain the selected PATH symlink spelling, not its raw target.
                let query =
                    shell_probe_command(dialect, Provider::Omp, &nonce, &bin, &result).unwrap();
                assert!(query.len() <= 8192 && !query.chars().any(char::is_control));
                let stdin = child.stdin.as_mut().unwrap();
                // Global zshenv can reset PATH even under -f; own the fixture after startup.
                stdin
                    .write_all(
                        format!("PATH={}\n", shell_word(root.0.to_str().unwrap()).unwrap())
                            .as_bytes(),
                    )
                    .await
                    .unwrap();
                if !setup.is_empty() {
                    stdin
                        .write_all(format!("{setup}\n").as_bytes())
                        .await
                        .unwrap();
                }
                stdin
                    .write_all(format!("{query}\n").as_bytes())
                    .await
                    .unwrap();
                stdin.flush().await.unwrap();
                let bytes = tokio::time::timeout(Duration::from_secs(5), async {
                    loop {
                        let bytes = fs::read(&result).unwrap();
                        if bytes.ends_with(b"\n") {
                            break bytes;
                        }
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                })
                .await
                .expect("fixed current-shell result");
                assert_eq!(
                    shell_probe_result(&bytes, &nonce, &shell).unwrap(),
                    expected,
                    "{name} setup {setup}"
                );
                assert!(!root.0.join("must-not-run").exists());
                assert!(shell_probe_result(&bytes, &"6".repeat(64), &shell).is_err());
                let mut wrong = shell.clone();
                *wrong.namespaces.nspids.last_mut().unwrap() += 1;
                assert!(shell_probe_result(&bytes, &nonce, &wrong).is_err());
                child.kill().await.unwrap();
                child.wait().await.unwrap();
            }
        }
    }
    #[test]
    fn result_requires_exact_nonce_pid_single_record_and_completion() {
        let shell = process_evidence(std::process::id()).unwrap();
        let pid = shell.namespaces.nspids.last().unwrap();
        let nonce = "a".repeat(64);
        let good = format!("1|{nonce}|{pid}|file|1|done\n");
        assert!(shell_probe_result(good.as_bytes(), &nonce, &shell).unwrap());
        for bad in [
            good.trim_end().to_owned(),
            format!("{good}{good}"),
            good.replace("|file|", "|function|"),
            good.replace(&format!("|{pid}|"), &format!("|0{pid}|")),
            good.replace("|1|done", "|true|done"),
        ] {
            assert!(shell_probe_result(bad.as_bytes(), &nonce, &shell).is_err());
        }
    }
    #[test]
    fn private_result_artifact_uses_retained_object_and_rejects_replacement_before_consumption() {
        let root = Root::new();
        let shell = process_evidence(std::process::id()).unwrap();
        let config=Config::parse(&format!("version=1\nruntime_dir={:?}\nruntime_dir_agent={:?}\n[executables]\nherdr=\"/not-executed\"\n",root.0,root.0)).unwrap();
        let mut artifact = ProbeArtifact::create(&config, &Uuid::random(), &shell).unwrap();
        artifact.recheck(&shell).unwrap();
        assert_eq!(artifact.file.metadata().unwrap().nlink(), 1);
        let replacement = artifact.dir.join("other");
        File::create(&replacement).unwrap();
        fs::set_permissions(&replacement, fs::Permissions::from_mode(0o600)).unwrap();
        fs::rename(&replacement, &artifact.path).unwrap();
        assert!(artifact.read(&shell).is_err());
        let path = artifact.path.clone();
        drop(artifact);
        assert!(path.exists(), "foreign replacement must not be unlinked");
    }

    #[tokio::test]
    async fn synthetic_bash_zsh_resolver_edges_report_finite_diagnostics_without_execution() {
        for (dialect, name) in [(ShellDialect::Bash, "bash"), (ShellDialect::Zsh, "zsh")] {
            for (provider, kind) in [(Provider::Omp, "omp"), (Provider::Codex, "codex")] {
                let cases = if dialect == ShellDialect::Zsh {
                    vec![
                        "plain",
                        "same_object_outer",
                        "hashed",
                        "automatic_hash",
                        "relative_path_hash",
                        "higher_precedence",
                        "version_5_9",
                        "version_5_9_1",
                        "version_5_9_0",
                        "version_5_9_10",
                        "version_5_10",
                        "builtin_function",
                        "wrong_outer",
                        "alias",
                        "function",
                        "autoload",
                        "trace",
                        "resolver_function",
                        "printf_function",
                    ]
                } else {
                    vec![
                        "plain",
                        "same_object_outer",
                        "hashed",
                        "automatic_hash",
                        "higher_precedence",
                        "wrong_outer",
                        "alias",
                        "function",
                        "trace",
                        "resolver_function",
                        "printf_function",
                    ]
                };
                for case in cases {
                    let root = Root::new();
                    let wrapper = root.0.join("out'er wrapper");
                    fs::write(
                        &wrapper,
                        format!(
                            "#!/bin/sh\nprintf executed > '{}'\n",
                            root.0.join("must-not-run").display()
                        ),
                    )
                    .unwrap();
                    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
                    let bin_dir = root.0.join("bin dir's");
                    fs::DirBuilder::new().mode(0o700).create(&bin_dir).unwrap();
                    let bin = bin_dir.join(kind);
                    std::os::unix::fs::symlink(&wrapper, &bin).unwrap();
                    let other_dir = root.0.join("other");
                    let relative_dir = other_dir.join("relative");
                    fs::create_dir_all(&relative_dir).unwrap();
                    fs::create_dir(root.0.join("relative")).unwrap();
                    let other_bin = other_dir.join(kind);
                    let relative_bin = relative_dir.join(kind);
                    let other_wrapper = root.0.join("different wrapper");
                    fs::copy(&wrapper, &other_wrapper).unwrap();
                    std::os::unix::fs::symlink(&other_wrapper, &other_bin).unwrap();
                    std::os::unix::fs::symlink(&other_wrapper, &relative_bin).unwrap();
                    let result = root.0.join("result");
                    File::create(&result).unwrap();
                    fs::set_permissions(&result, fs::Permissions::from_mode(0o600)).unwrap();
                    let literal = if case == "relative_path_hash" {
                        shell_word(&format!("relative/{kind}")).unwrap()
                    } else {
                        shell_word(bin.to_str().unwrap()).unwrap()
                    };
                    let bin_path = shell_word(bin_dir.to_str().unwrap()).unwrap();
                    let other_path = shell_word(other_dir.to_str().unwrap()).unwrap();
                    let setup = match case {
                        "plain" | "same_object_outer" | "wrong_outer" => String::new(),
                        "hashed" if dialect == ShellDialect::Bash => {
                            format!("PATH={other_path}; hash -p {literal} {kind}")
                        }
                        "hashed" => format!("PATH={other_path}; hash {kind}={literal}"),
                        "automatic_hash" => format!("hash {kind}"),
                        "relative_path_hash" => {
                            format!("PATH=relative:{bin_path}; hash {kind}; cd {other_path}")
                        }
                        "higher_precedence" => format!("PATH={other_path}:{bin_path}"),
                        "version_5_9" => "ZSH_VERSION=5.9".into(),
                        "version_5_9_1" => "ZSH_VERSION=5.9.1".into(),
                        "version_5_9_0" => "ZSH_VERSION=5.9.0".into(),
                        "version_5_9_10" => "ZSH_VERSION=5.9.10".into(),
                        "version_5_10" => "ZSH_VERSION=5.10".into(),
                        // Quoting the first helper token cannot bypass this function.
                        "builtin_function" => "builtin() { return 0; }".into(),
                        "alias" => format!("alias {kind}='false'"),
                        "function" => format!("{kind}() {{ return 0; }}"),
                        "autoload" => format!("autoload -Uz {kind}"),
                        "trace" => "set -x".into(),
                        "resolver_function" if dialect == ShellDialect::Bash => {
                            "type() { return 0; }".into()
                        }
                        "resolver_function" => "whence() { return 0; }".into(),
                        "printf_function" => "printf() { return 0; }".into(),
                        _ => panic!("closed synthetic fixture case"),
                    };
                    let diagnostics = match dialect {
                        ShellDialect::Bash => format!(
                            "__v=unsupported; [[ \"${{BASH_VERSINFO[0]-}}.${{BASH_VERSINFO[1]-}}.${{BASH_VERSINFO[2]-}}\" == '5.3.9' ]] && __v=supported; __h=other; [[ \"$('builtin' 'type' '-t' 'type')\" == builtin && \"$('builtin' 'type' '-t' 'printf')\" == builtin ]] && __h=builtin; __k=\"$('builtin' 'type' '-t' '{kind}')\"; case \"$__k\" in file|alias|function|builtin) ;; *) __k=other ;; esac; __m=0; [[ \"$('builtin' 'type' '-p' '{kind}')\" == {literal} ]] && __m=1; __t=off; [[ \"$-\" == *[xv]* ]] && __t=on; 'builtin' 'printf' 'version=%s|helpers=%s|category=%s|pathMatch=%s|trace=%s\\n' \"$__v\" \"$__h\" \"$__k\" \"$__m\" \"$__t\""
                        ),
                        ShellDialect::Zsh => format!(
                            "__v=unsupported; [[ ( \"$ZSH_VERSION\" == '5.9' || \"$ZSH_VERSION\" == '5.9.1' ) ]] && __v=supported; __h=other; [[ \"$('builtin' 'whence' '-w' 'whence')\" == 'whence: builtin' && \"$('builtin' 'whence' '-w' 'printf')\" == 'printf: builtin' ]] && __h=builtin; case \"$('builtin' 'whence' '-w' '{kind}')\" in '{kind}: command') __k=command ;; '{kind}: hashed') __k=hashed ;; '{kind}: alias') __k=alias ;; '{kind}: function') __k=function ;; '{kind}: builtin') __k=builtin ;; *) __k=other ;; esac; __m=0; [[ \"$('builtin' 'whence' '{kind}')\" == {literal} ]] && __m=1; __t=off; [[ \"$-\" == *[xv]* ]] && __t=on; 'builtin' 'printf' 'version=%s|helpers=%s|category=%s|pathMatch=%s|trace=%s\\n' \"$__v\" \"$__h\" \"$__k\" \"$__m\" \"$__t\""
                        ),
                    };
                    let expected_wrapper = match case {
                        "wrong_outer" => root.0.join("different-outer"),
                        "same_object_outer" => wrapper.clone(),
                        _ => bin,
                    };
                    let nonce = "b".repeat(64);
                    let query =
                        shell_probe_command(dialect, provider, &nonce, &expected_wrapper, &result)
                            .unwrap();
                    let mut command = tokio::process::Command::new(executable(name));
                    match dialect {
                        ShellDialect::Bash => {
                            command.args(["--noprofile", "--norc", "-i"]);
                        }
                        ShellDialect::Zsh => {
                            command.args(["-f", "-i"]);
                        }
                    }
                    let mut child = command
                        .env_clear()
                        .env("PATH", &bin_dir)
                        .env("HOME", &root.0)
                        .env("ZDOTDIR", &root.0)
                        .env("TERM", "dumb")
                        .env("LC_ALL", "C")
                        .current_dir(&root.0)
                        .stdin(std::process::Stdio::piped())
                        .stdout(std::process::Stdio::piped())
                        .stderr(std::process::Stdio::piped())
                        .kill_on_drop(true)
                        .spawn()
                        .unwrap();
                    let shell = process_evidence(child.id().unwrap()).unwrap();
                    let mut input = child.stdin.take().unwrap();
                    // These bytes go only to the disposable owned synthetic interpreter,
                    // never to a live pane, provider, or real query/result/capability.
                    // Set the owned PATH after global startup, which -f does not suppress.
                    input
                        .write_all(
                            format!("PATH={bin_path}\n{setup}\n{diagnostics}\n{query}\nexit\n")
                                .as_bytes(),
                        )
                        .await
                        .unwrap();
                    drop(input);
                    let output =
                        tokio::time::timeout(Duration::from_secs(5), child.wait_with_output())
                            .await
                            .expect("synthetic shell diagnostic must terminate")
                            .unwrap();
                    let bytes = fs::read(&result).unwrap();
                    let measured = shell_probe_result(&bytes, &nonce, &shell);
                    assert!(
                        output.stdout.len() <= 8192 && output.stderr.len() <= 8192,
                        "synthetic diagnostic exceeded its bounded fixture output"
                    );
                    assert!(
                        !root.0.join("must-not-run").exists(),
                        "resolver executed the synthetic provider"
                    );
                    if case == "relative_path_hash" {
                        assert!(
                            String::from_utf8_lossy(&output.stdout).contains("|pathMatch=1|"),
                            "relative PATH entry did not precede the inherited automatic hash"
                        );
                    }
                    if case == "builtin_function" {
                        assert!(measured.is_err(), "shadowed helper installed launch proof");
                        continue;
                    }
                    assert_eq!(
                        measured.as_ref().ok().copied(),
                        Some(matches!(
                            case,
                            "plain" | "same_object_outer" | "hashed" | "automatic_hash" | "version_5_9" | "version_5_9_1"
                        )),
                        "{name} {kind} synthetic {case}: status={:?}; finite diagnostics={}; synthetic stderr={}; result category={:?}",
                        output.status.code(),
                        String::from_utf8_lossy(&output.stdout),
                        String::from_utf8_lossy(&output.stderr),
                        std::str::from_utf8(&bytes)
                            .ok()
                            .and_then(|line| line.split('|').nth(3))
                    );
                }
            }
        }
    }
}

fn alias_ack_matches(reply:&Agent,original:&Pane,context:&RenameContext,name:&AgentName)->bool {
    reply.name.as_deref()==Some(name.0.as_str())&&!reply.launch_pending
        &&reply.pane_id==context.pane&&reply.terminal_id==context.terminal
        &&reply.workspace_id==context.workspace&&reply.tab_id==context.tab
        &&reply.agent==original.agent&&reply.agent_session==original.agent_session
}
fn positive_agent_alias(snapshot:&Snapshot,pane:&Pane,provider:Provider)->bool {
    let kind=match provider {Provider::Omp=>"omp",Provider::Codex=>"codex"};
    pane.agent.as_deref()==Some(kind) && snapshot.agents.iter().filter(|agent|agent.pane_id==pane.pane_id).count()==1 && snapshot.agents.iter().any(|agent|agent.pane_id==pane.pane_id && agent.terminal_id==pane.terminal_id && agent.workspace_id==pane.workspace_id && agent.tab_id==pane.tab_id && agent.agent.as_deref()==Some(kind) && agent.agent_session==pane.agent_session && !agent.launch_pending)
}
impl Topology {
    pub(crate) async fn agent_alias_context(&self,target:&Target)->SecurityResult<(Source,RenameContext)> {
        let _update=self.updates.lock().await;
        let (source,context,summary)=self.logical_rename_context(target)?;
        let pane=source.snapshot.pane(&context.pane).map_err(policy)?;
        if !summary.capabilities.agent_rename||!positive_agent_alias(&source.snapshot,pane,summary.provider) {
            return Err(ErrorCode::NotReady);
        }
        Ok((source,context))
    }
    /// Before effect this is full readiness admission. After a correlated ack only
    /// the original authority survives; our own PaneUpdated may advance inventory.
    pub(crate) fn validate_agent_alias_context(&self,target:&Target,original:&Source,context:&RenameContext,ack:bool)->SecurityResult<()> {
        let (source,summary)=self.validate_logical_rename_context(target,original,context,ack)?;
        if !ack&&!summary.capabilities.agent_rename{return Err(ErrorCode::NotReady)}
        let pane=source.snapshot.pane(&context.pane).map_err(policy)?;
        let initial=original.snapshot.pane(&context.pane).map_err(policy)?;
        if !positive_agent_alias(&source.snapshot,pane,summary.provider)
            ||!positive_agent_alias(&original.snapshot,initial,summary.provider)
            ||pane.agent_session!=initial.agent_session {return Err(ErrorCode::StaleSession)}
        Ok(())
    }
}

impl Topology {
    pub(crate) async fn agent_alias_once(&self,security:&Security,auth:&Authorized,action:&Uuid,params:&AgentRenameParams)->SecurityResult<()> {
        let (source,context)=self.agent_alias_context(&params.target).await?;
        let client=Client::connect(&source.socket_path).await.map_err(policy)?;
        if client.evidence!=source.evidence {return Err(ErrorCode::StaleSession)}
        self.validate_agent_alias_context(&params.target,&source,&context,false)?;
        let fd=client.fence_socket().map_err(policy)?;
        let admission_source=source.clone();let admission_context=context.clone();
        security.agent_alias_boundary(auth,action.clone(),params.target.clone(),context.source_epoch.clone(),fd,|| {
            let _update=self.updates.try_lock().map_err(|_|ErrorCode::Busy)?;
            self.validate_agent_alias_context(&params.target,&admission_source,&admission_context,false)?;
            security.state.validate_session_mutation(&Request {id:action.clone(),body:RequestBody::AgentRename(params.clone())})
        },|checkpoint|async move {
            let lost=|result|crate::storage::ReceiptTransition {state:ReceiptState::Uncertain,proof:crate::storage::TransitionProof::LostOutcome,result,error:None,at:pairing::now_ms()};
            let response=client.agent_rename(&context.pane,&params.name).await;
            let (reply,normal)=match response {
                Ok(reply)=>(Some(reply),true),
                Err(ApiError::Changed(mut value))=>(value.get_mut("agent").and_then(|agent|serde_json::from_value(agent.take()).ok()),false),
                Err(ApiError::Rejected(code)) if matches!(code.as_str(),"invalid_agent_name"|"agent_name_taken"|"agent_launch_pending"|"agent_not_found"|"agent_target_ambiguous")=>return crate::storage::ReceiptTransition {state:ReceiptState::Failed,proof:crate::storage::TransitionProof::DefinitiveNoEffect(Id("native alias refused before mutation".into())),result:None,error:Some(crate::protocol::WireError {code:ErrorCode::NotReady,message:SafeMessage("Agent alias was refused before mutation".into())}),at:pairing::now_ms()},
                Err(_)=>(None,false),
            };
            let Some(reply)=reply else {return lost(None)};
            let Ok(original)=source.snapshot.pane(&context.pane) else{return lost(None)};
            if !alias_ack_matches(&reply,original,&context,&params.name){return lost(None)}
            let facts=ActionResultAgentRenamed {target:params.target.clone(),terminal_id:context.terminal.clone()};
            if checkpoint.retain(&params.target,&context.source_epoch,facts.clone()).is_err() {return lost(None)}
            let result=Some(ActionResult::AgentRenamed(facts));
            if checkpoint.checkpoint().await.is_err() || !normal || self.validate_agent_alias_context(&params.target,&source,&context,true).is_err() {return lost(result)}
            crate::storage::ReceiptTransition {state:ReceiptState::Succeeded,proof:crate::storage::TransitionProof::CorrelatedAcknowledgement(Id("native alias mutation acknowledged".into())),result,error:None,at:pairing::now_ms()}
        }).await
    }
    pub(crate) async fn workspace_rename_once(&self,security:&Security,auth:&Authorized,action:&Uuid,params:&WorkspaceRenameParams)->SecurityResult<()> {
        let (source,context)=self.workspace_rename_context(&params.target).await?;
        let client=Client::connect(&source.socket_path).await.map_err(policy)?;
        if client.evidence!=source.evidence{return Err(ErrorCode::StaleSession)}
        let fd=client.fence_socket().map_err(policy)?;
        let admission_source=source.clone();let admission_context=context.clone();
        security.boundary(auth,action.clone(),|| {
            self.validate_workspace_rename_context_now(&params.target,&admission_source,&admission_context)?;
            security.state.validate_session_mutation(&Request{id:action.clone(),body:RequestBody::WorkspaceRename(params.clone())})
        },fd,async move {
            let response=client.workspace_rename(&context.workspace,&params.label).await;
            let (reply,normal)=match response {
                Ok(reply)=>(Some(reply),true),
                Err(ApiError::Changed(mut value))=>(value.get_mut("workspace").and_then(|workspace|serde_json::from_value(workspace.take()).ok()),false),
                Err(ApiError::Rejected(code)) if matches!(code.as_str(),"workspace_not_found"|"invalid_request"|"invalid_params")=>{
                    return ReceiptTransition{state:ReceiptState::Failed,
                        proof:TransitionProof::DefinitiveNoEffect(Id("native workspace rename refused before mutation".into())),
                        result:None,error:Some(crate::transport::wire_error(ErrorCode::NotReady)),at:pairing::now_ms()};
                }
                Err(_)=>(None,false),
            };
            let matched=reply.as_ref().is_some_and(|reply|reply.workspace_id==context.workspace&&reply.label==params.label.0);
            let result=matched.then(||ActionResult::WorkspaceRenamed(ActionResultWorkspaceRenamed{
                target:params.target.clone(),workspace_id:context.workspace.clone(),
            }));
            if !normal||!matched||self.validate_logical_rename_context(&params.target,&source,&context,true).is_err() {
                return ReceiptTransition{state:ReceiptState::Uncertain,proof:TransitionProof::LostOutcome,result,error:None,at:pairing::now_ms()};
            }
            ReceiptTransition{state:ReceiptState::Succeeded,
                proof:TransitionProof::CorrelatedAcknowledgement(Id("native workspace effective label confirmed".into())),
                result,error:None,at:pairing::now_ms()}
        }).await
    }
}
