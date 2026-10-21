// SPDX-License-Identifier: GPL-3.0-or-later
//! Real direct iroh transport; one bidirectional stream and one ordered state writer.
use crate::{ALPN, pairing::*, protocol::*, storage::Reservation};
use iroh::{
    Endpoint,
    endpoint::{Connection, RecvStream, SendStream},
};
use std::{
    collections::{HashMap, HashSet},
    future::Future,
    io,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    sync::{Semaphore, mpsc},
    task::JoinSet,
};
/// Public address/provenance only, written exclusively by the hidden local smoke invocation.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct LocalHints {
    pub endpoint_addr: iroh::EndpointAddr,
    pub package_version: String,
    pub companion_lock_sha256: String,
    pub core_iroh: String,
}
pub fn lock_sha256() -> &'static str {
    static HASH: std::sync::LazyLock<String> =
        std::sync::LazyLock::new(|| hex(&sha256(include_bytes!("../Cargo.lock"))));
    &HASH
}
fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid goatr frame")
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameStage {
    FrameHeader,
    FrameLength,
    FrameCap,
    FramePayload,
    WriteLength,
    WriteHeader,
    WritePayload,
    WriteDeadline,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameIoKind {
    UnexpectedEof,
    ConnectionReset,
    NotConnected,
    TimedOut,
    Other,
}
/// Closed diagnostic metadata only; no original error, frame bytes or byte counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameFailure {
    pub stage: FrameStage,
    pub io: Option<FrameIoKind>,
    pub code: Option<ErrorCode>,
}
impl FrameFailure {
    pub fn from_error(error: &io::Error) -> Option<Self> {
        error.get_ref()?.downcast_ref::<Self>().copied()
    }
    fn error(stage: FrameStage, io: Option<FrameIoKind>, code: Option<ErrorCode>) -> io::Error {
        let kind = if stage == FrameStage::WriteDeadline {
            io::ErrorKind::TimedOut
        } else {
            io::ErrorKind::InvalidData
        };
        io::Error::new(kind, Self { stage, io, code })
    }
    fn io_kind(kind: io::ErrorKind) -> FrameIoKind {
        match kind {
            io::ErrorKind::UnexpectedEof => FrameIoKind::UnexpectedEof,
            io::ErrorKind::ConnectionReset => FrameIoKind::ConnectionReset,
            io::ErrorKind::NotConnected => FrameIoKind::NotConnected,
            io::ErrorKind::TimedOut => FrameIoKind::TimedOut,
            _ => FrameIoKind::Other,
        }
    }
    fn read(stage: FrameStage, error: iroh::endpoint::ReadExactError) -> io::Error {
        let kind = match error {
            iroh::endpoint::ReadExactError::FinishedEarly(_) => FrameIoKind::UnexpectedEof,
            iroh::endpoint::ReadExactError::ReadError(error) => {
                Self::io_kind(io::Error::from(error).kind())
            }
        };
        Self::error(stage, Some(kind), None)
    }
    fn write(stage: FrameStage, error: iroh::endpoint::WriteError) -> io::Error {
        Self::error(
            stage,
            Some(Self::io_kind(io::Error::from(error).kind())),
            None,
        )
    }
    fn protocol(stage: FrameStage, error: ProtocolFailure) -> io::Error {
        let code = match error.kind {
            FailureKind::UnsupportedVersion => ErrorCode::UnsupportedVersion,
            FailureKind::UnsupportedMethod => ErrorCode::UnsupportedMethod,
            FailureKind::InvalidRequest | FailureKind::Incompatible => ErrorCode::InvalidRequest,
        };
        Self::error(stage, None, Some(code))
    }
}
impl std::fmt::Display for FrameFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "frame_stage={:?}; frame_io={:?}; frame_code={:?}",
            self.stage, self.io, self.code
        )
    }
}
impl std::error::Error for FrameFailure {}
pub async fn read_frame(recv: &mut RecvStream, cap: usize) -> io::Result<Vec<u8>> {
    let mut header = [0; 4];
    recv.read_exact(&mut header)
        .await
        .map_err(|error| FrameFailure::read(FrameStage::FrameHeader, error))?;
    let length = frame_length(header)
        .map_err(|error| FrameFailure::protocol(FrameStage::FrameLength, error))?;
    if length > cap {
        return Err(FrameFailure::error(FrameStage::FrameCap, None, None));
    }
    let mut bytes = vec![0; length];
    recv.read_exact(&mut bytes)
        .await
        .map_err(|error| FrameFailure::read(FrameStage::FramePayload, error))?;
    Ok(bytes)
}
pub async fn write_frame(send: &mut SendStream, bytes: &[u8]) -> io::Result<()> {
    frame_length(
        (u32::try_from(bytes.len())
            .map_err(|_| FrameFailure::error(FrameStage::WriteLength, None, None))?)
        .to_be_bytes(),
    )
    .map_err(|error| FrameFailure::protocol(FrameStage::WriteLength, error))?;
    tokio::time::timeout(Duration::from_secs(5), async {
        send.write_all(&(bytes.len() as u32).to_be_bytes())
            .await
            .map_err(|error| FrameFailure::write(FrameStage::WriteHeader, error))?;
        send.write_all(bytes)
            .await
            .map_err(|error| FrameFailure::write(FrameStage::WritePayload, error))
    })
    .await
    .map_err(|_| {
        FrameFailure::error(FrameStage::WriteDeadline, Some(FrameIoKind::TimedOut), None)
    })?
}
pub fn wire_error(code: ErrorCode) -> WireError {
    WireError {
        code,
        message: SafeMessage("Operation unavailable or request rejected".into()),
    }
}
async fn response(
    send: &mut SendStream,
    id: Uuid,
    result: Result<ResultBody, ErrorCode>,
) -> io::Result<()> {
    let bytes = encode_payload(&Envelope::Response(Response {
        id,
        outcome: result.map_err(wire_error),
    }))
    .map_err(|_| invalid())?;
    write_frame(send, &bytes).await
}
fn request(bytes: &[u8]) -> io::Result<Request> {
    match decode_payload(bytes, None).map_err(|_| invalid())? {
        Envelope::Request(r) => Ok(r),
        _ => Err(invalid()),
    }
}
async fn finish_reply(send: &mut SendStream) -> io::Result<()> {
    send.finish().map_err(|_| invalid())?;
    tokio::time::timeout(Duration::from_secs(5), send.stopped())
        .await
        .map_err(|_| invalid())?
        .map_err(|_| invalid())?;
    Ok(())
}
struct ReaderTask(tokio::task::JoinHandle<()>);
impl Drop for ReaderTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}
struct ConnectionPanicReporter {
    reporter: Option<crate::herdr::OmpCreationReporter>,
    connection: Uuid,
    guard: &'static str,
}
impl Drop for ConnectionPanicReporter {
    fn drop(&mut self) {
        if std::thread::panicking()
            && let Some(reporter) = &self.reporter
        {
            reporter.transport_failure(&self.connection, self.guard, None, "none", "none");
        }
    }
}
struct ActiveConnection<'a> {
    security: &'a Security,
    auth: &'a Authorized,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OutputStep {
    Continue,
    Closed,
}

enum DispatchPoll<T> {
    Complete(T),
    Deadline,
    Output(io::Result<OutputStep>),
    ExtraStream(bool),
}

/// Poll exactly the pending request, its original absolute deadline, and one ordered output
/// step. The caller retains response enqueueing so an output write cannot be canceled or
/// overtaken by that response.
async fn poll_dispatch_output<D, O, E>(
    mut dispatch: Pin<&mut D>,
    mut deadline: Pin<&mut tokio::time::Sleep>,
    mut output: Pin<&mut O>,
    mut extra: Pin<&mut E>,
) -> DispatchPoll<D::Output>
where
    D: Future,
    O: Future<Output = io::Result<OutputStep>>,
    E: Future<Output = bool>,
{
    tokio::select! {
        result = dispatch.as_mut() => DispatchPoll::Complete(result),
        _ = deadline.as_mut() => DispatchPoll::Deadline,
        result = output.as_mut() => DispatchPoll::Output(result),
        result = extra.as_mut() => DispatchPoll::ExtraStream(result),
    }
}

fn closed_frame_stage(stage: FrameStage) -> &'static str {
    match stage {
        FrameStage::FrameHeader => "frame_header",
        FrameStage::FrameLength => "frame_length",
        FrameStage::FrameCap => "frame_cap",
        FrameStage::FramePayload => "frame_payload",
        FrameStage::WriteLength => "write_length",
        FrameStage::WriteHeader => "write_header",
        FrameStage::WritePayload => "write_payload",
        FrameStage::WriteDeadline => "write_deadline",
    }
}

fn closed_frame_io(io: Option<FrameIoKind>) -> &'static str {
    match io {
        None => "none",
        Some(FrameIoKind::UnexpectedEof) => "unexpected_eof",
        Some(FrameIoKind::ConnectionReset) => "connection_reset",
        Some(FrameIoKind::NotConnected) => "not_connected",
        Some(FrameIoKind::TimedOut) => "timed_out",
        Some(FrameIoKind::Other) => "other",
    }
}

fn report_transport_frame(
    reporter: Option<&crate::herdr::OmpCreationReporter>,
    connection: &Uuid,
    guard: &'static str,
    error: &io::Error,
) {
    let (code, io, frame) =
        FrameFailure::from_error(error).map_or((None, "other", "none"), |failure| {
            (
                failure.code,
                closed_frame_io(failure.io),
                closed_frame_stage(failure.stage),
            )
        });
    if let Some(reporter) = reporter {
        reporter.transport_failure(connection, guard, code, io, frame);
    }
}

fn report_codex_transport_frame(
    reporter: Option<&crate::herdr::CodexInvalidationReporter>,
    connection: &Uuid,
    guard: &'static str,
    error: &io::Error,
) {
    let (code, io, frame) = FrameFailure::from_error(error).map_or(
        (None, "other", "none"),
        |failure| {
            (
                failure.code,
                closed_frame_io(failure.io),
                closed_frame_stage(failure.stage),
            )
        },
    );
    if let Some(reporter) = reporter {
        reporter.transport(connection, guard, code, io, 0, frame);
    }
}

fn report_codex_transport(
    reporter: Option<&crate::herdr::CodexInvalidationReporter>,
    connection: &Uuid,
    guard: &'static str,
    code: Option<ErrorCode>,
    io: &'static str,
) {
    if let Some(reporter) = reporter {
        reporter.transport(connection, guard, code, io, 0, "none");
    }
}

/// Process-local first-connection Hello capture. Its selected connection is opaque and never
/// reaches stderr; each closed phase/guard emits at most once across the whole serve process.
#[derive(Clone)]
struct HelloCaptureReporter(Arc<Mutex<HelloCaptureState>>);
struct HelloCaptureState {
    selected: bool,
    lines: u8,
    observations: HashSet<(&'static str, &'static str)>,
}
impl HelloCaptureReporter {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(HelloCaptureState {
            selected: false,
            lines: 0,
            observations: HashSet::new(),
        })))
    }
    fn select(&self) -> Option<Self> {
        let mut state = self.0.lock().expect("hello capture reporter mutex");
        if state.selected {
            None
        } else {
            state.selected = true;
            Some(self.clone())
        }
    }
    fn report(&self, phase: &'static str, guard: &'static str, code: Option<ErrorCode>, io: &'static str) {
        let mut state = self.0.lock().expect("hello capture reporter mutex");
        if state.lines >= 8 || !state.observations.insert((phase, guard)) {
            return;
        }
        state.lines += 1;
        let code = code.map_or("none".to_owned(), |code| format!("{code:?}"));
        eprintln!("goatr_hello_capture phase={phase} guard={guard} code={code} io={io}");
    }
}

fn hello_frame_error(error: &io::Error) -> (Option<ErrorCode>, &'static str) {
    FrameFailure::from_error(error).map_or((None, "other"), |failure| {
        (failure.code, closed_frame_io(failure.io))
    })
}

async fn output_step(
    send: &mut SendStream,
    security: &Security,
    auth: &Authorized,
    close_after_output: bool,
    reporter: Option<&crate::herdr::OmpCreationReporter>,
    codex_reporter: Option<&crate::herdr::CodexInvalidationReporter>,
    omp_response_pending: &mut bool,
) -> io::Result<OutputStep> {
    let _guard = security
        .guard(auth)
        .await
        .inspect_err(|code| {
            if let Some(reporter) = reporter {
                reporter.transport_failure(
                    &auth.connection_id,
                    "output_guard",
                    Some(*code),
                    "none",
                    "none",
                );
            }
            report_codex_transport(
                codex_reporter,
                &auth.connection_id,
                "output_guard",
                Some(*code),
                "none",
            );
        })
        .map_err(|_| invalid())?;
    let (bytes, closed) = security
        .state
        .pop_output(&auth.connection_id)
        .inspect_err(|code| {
            if let Some(reporter) = reporter {
                reporter.transport_failure(
                    &auth.connection_id,
                    "output_pop_error",
                    Some(*code),
                    "none",
                    "none",
                );
            }
            report_codex_transport(
                codex_reporter,
                &auth.connection_id,
                "output_pop_error",
                Some(*code),
                "none",
            );
        })
        .map_err(|_| invalid())?;
    if let Some(bytes) = bytes {
        // Parse only delivery metadata. No frame/control payload is logged or retained.
        let events = delivered_notifications(&bytes).inspect_err(|error| {
            report_transport_frame(reporter, &auth.connection_id, "delivery_metadata", error);
            report_codex_transport_frame(
                codex_reporter,
                &auth.connection_id,
                "delivery_metadata",
                error,
            );
        })?;
        write_frame(send, &bytes).await.inspect_err(|error| {
            report_transport_frame(reporter, &auth.connection_id, "output_write", error);
            report_codex_transport_frame(
                codex_reporter,
                &auth.connection_id,
                "output_write",
                error,
            );
        })?;
        if *omp_response_pending {
            *omp_response_pending = false;
            if let Some(reporter) = reporter {
                reporter.report("transport", "output_written_after_queue", None);
            }
        }
        if !events.is_empty() {
            let peer = auth.proof.clone();
            security
                .storage
                .call(move |storage| storage.notifications_received(&peer, &events))
                .await
                .inspect_err(|_| {
                    if let Some(reporter) = reporter {
                        reporter.transport_failure(
                            &auth.connection_id,
                            "delivery_storage",
                            None,
                            "other",
                            "none",
                        );
                    }
                    report_codex_transport(
                        codex_reporter,
                        &auth.connection_id,
                        "delivery_storage",
                        None,
                        "other",
                    );
                })
                .map_err(|_| invalid())?;
        }
        Ok(OutputStep::Continue)
    } else if closed || close_after_output {
        if let Some(reporter) = reporter {
            reporter.transport_failure(
                &auth.connection_id,
                "output_queue_closed",
                None,
                "none",
                "none",
            );
        }
        report_codex_transport(
            codex_reporter,
            &auth.connection_id,
            "output_queue_closed",
            None,
            "none",
        );
        Ok(OutputStep::Closed)
    } else {
        Ok(OutputStep::Continue)
    }
}

impl Drop for ActiveConnection<'_> {
    fn drop(&mut self) {
        self.security.disconnect(self.auth);
    }
}
#[derive(Default)]
struct Viewing {
    subscriptions: HashMap<Uuid, Uuid>,
    session: Option<Uuid>,
}

#[derive(Clone, Copy)]
pub enum CodexTransportWinner {
    Admin,
    Transport,
    Topology,
    CtrlC,
    Sigterm,
}

#[derive(Clone)]
pub struct CodexTransportCapture(crate::herdr::CodexInvalidationReporter);
impl CodexTransportCapture {
    fn selected_guard(winner: CodexTransportWinner, failed: bool) -> &'static str {
        match (winner, failed) {
            (CodexTransportWinner::Admin, false) => "selected_admin_ok",
            (CodexTransportWinner::Admin, true) => "selected_admin_error",
            (CodexTransportWinner::Transport, false) => "selected_transport_ok",
            (CodexTransportWinner::Transport, true) => "selected_transport_error",
            (CodexTransportWinner::Topology, false) => "selected_topology_ok",
            (CodexTransportWinner::Topology, true) => "selected_topology_error",
            (CodexTransportWinner::CtrlC, _) => "selected_ctrl_c",
            (CodexTransportWinner::Sigterm, _) => "selected_sigterm",
        }
    }
    pub fn selected(&self, winner: CodexTransportWinner, result: Option<&io::Result<()>>) {
        match winner {
            CodexTransportWinner::CtrlC | CodexTransportWinner::Sigterm => {
                self.0.supervisor(Self::selected_guard(winner, false), None, "cancelled", 0);
            }
            _ => match result.expect("selected job has a result") {
                Ok(()) => self.0.supervisor(Self::selected_guard(winner, false), None, "none", 0),
                Err(error) => self.0.supervisor_error(Self::selected_guard(winner, true), error),
            },
        }
    }
    pub fn sibling_error(&self, winner: CodexTransportWinner, result: &io::Result<()>) {
        if let Err(error) = result {
            let guard = match winner {
                CodexTransportWinner::Admin => "sibling_admin_error",
                CodexTransportWinner::Transport => "sibling_transport_error",
                CodexTransportWinner::Topology => "sibling_topology_error",
                CodexTransportWinner::CtrlC | CodexTransportWinner::Sigterm => return,
            };
            self.0.supervisor_error(guard, error);
        }
    }
    fn reporter(&self) -> crate::herdr::CodexInvalidationReporter {
        self.0.clone()
    }
}

pub fn owned_codex_transport_capture(
    owned_omp_relay: Option<&crate::backends::omp::Relay>,
) -> Option<CodexTransportCapture> {
    (owned_omp_relay.is_some()
        && std::env::var_os("GOATR_OWNED_CODEX_DIAGNOSTIC").as_deref()
            == Some(std::ffi::OsStr::new("1")))
    .then(|| CodexTransportCapture(crate::herdr::CodexInvalidationReporter::new()))
}

pub async fn serve(
    endpoint: &Endpoint,
    security: Arc<Security>,
    topology: Option<Arc<crate::herdr::Topology>>,
    owned_omp_relay: Option<crate::backends::omp::Relay>,
    codex_transport_capture: Option<CodexTransportCapture>,
    mut stop: tokio::sync::watch::Receiver<bool>,
) -> io::Result<()> {
    let terminal_topology = topology.clone();
    let push=crate::push::Publisher::new(topology.as_ref().map(|t|t.config.push_allowed_origins.clone()).unwrap_or_else(||vec!["https://ntfy.sh".into()]));
    security.push.set(push.clone()).map_err(|_|invalid())?;
    let (push_stop,push_receiver)=tokio::sync::watch::channel(false);
    let push_job=tokio::spawn(push.run(Arc::downgrade(&security),push_receiver));
    // Only the smoke runner's already validated owned relay can opt into a diagnostic.
    // A run selects one provider capture so its shared eight-line budget cannot overlap.
    let owned = owned_omp_relay.is_some();
    let omp_creation_capture = owned
        && std::env::var_os("GOATR_OWNED_OMP_CREATION_CAPTURE").as_deref()
            == Some(std::ffi::OsStr::new("1"));
    let omp_offline_capture = owned
        && std::env::var_os("GOATR_OWNED_OMP_OFFLINE_CAPTURE").as_deref()
            == Some(std::ffi::OsStr::new("1"));
    let omp_backend_capture = owned
        && std::env::var_os("GOATR_OWNED_OMP_BACKEND_CAPTURE").as_deref()
            == Some(std::ffi::OsStr::new("1"));
    let codex_capture = owned
        && std::env::var_os("GOATR_OWNED_CODEX_DIAGNOSTIC").as_deref()
            == Some(std::ffi::OsStr::new("1"));
    let hello_capture = owned
        && std::env::var_os("GOATR_OWNED_HELLO_CAPTURE").as_deref()
            == Some(std::ffi::OsStr::new("1"));
    let omp_capture = omp_creation_capture || omp_offline_capture || omp_backend_capture;
    if u8::from(omp_creation_capture)
        + u8::from(omp_offline_capture)
        + u8::from(omp_backend_capture)
        > 1
        || (codex_capture && omp_capture)
        || (hello_capture && (omp_capture || codex_capture))
    {
        return Err(invalid());
    }
    let omp_first_guard = if omp_creation_capture {
        Some(crate::herdr::OmpCreationReporter::new())
    } else if omp_backend_capture {
        Some(crate::herdr::OmpCreationReporter::new_backend())
    } else if omp_offline_capture {
        Some(crate::herdr::OmpCreationReporter::new_offline())
    } else {
        None
    };
    if codex_transport_capture.is_some() != codex_capture {
        return Err(invalid());
    }
    let codex_invalidation_reporter = codex_transport_capture
        .as_ref()
        .map(CodexTransportCapture::reporter);
    let hello_capture = hello_capture.then(HelloCaptureReporter::new);
    let mut jobs = JoinSet::new();
    let omp = topology.as_ref().map(|owner| {
        crate::backends::omp::Runtime::new_with_first_guard(
            owner.clone(),
            security.clone(),
            owned_omp_relay,
            omp_first_guard.clone(),
        )
    });
    let codex = topology.map(|owner| {
        crate::backends::codex::Runtime::new_with_invalidation_reporter(
            owner,
            security.clone(),
            codex_invalidation_reporter.clone(),
        )
    });
    let codex_job = codex.as_ref().map(|owner| {
        let owner = owner.clone();
        let stop = stop.clone();
        tokio::spawn(async move {
            let _ = owner.run(stop).await;
        })
    });
    let omp_job = omp.as_ref().map(|owner| {
        let owner = owner.clone();
        let stop = stop.clone();
        tokio::spawn(async move {
            let _ = owner.run(stop).await;
        })
    });
    let mut attachment_ticks=tokio::time::interval(Duration::from_secs(30));
    loop {
        tokio::select! {
            _=attachment_ticks.tick()=>{let _=security.attachment_maintenance(now_ms()).await;},
            changed=stop.changed()=>{if changed.is_err() || *stop.borrow() {break;}},
            incoming=endpoint.accept()=>{let Some(incoming)=incoming else {break;}; let Ok(permit)=security.admission() else {incoming.refuse();continue;}; let security=security.clone(); let topology=terminal_topology.clone(); let omp=omp.clone(); let codex=codex.clone(); let omp_first_guard=omp_first_guard.clone(); let codex_invalidation_reporter=codex_invalidation_reporter.clone(); let hello_capture=hello_capture.clone().and_then(|capture| capture.select()); jobs.spawn(async move {if let Some(capture)=&hello_capture {capture.report("admission","entered",None,"none");} let connection=match tokio::time::timeout(Duration::from_secs(10),incoming).await {Ok(Ok(connection))=>connection,Ok(Err(_))=>{if let Some(capture)=&hello_capture {capture.report("admission","incoming_error",None,"other");}return;},Err(_)=>{if let Some(capture)=&hello_capture {capture.report("admission","incoming_deadline",Some(ErrorCode::Timeout),"timed_out");}return;}}; let remote=EndpointId(connection.remote_id().to_string()); let Some(attempt)=security.handshake(&remote) else {if let Some(capture)=&hello_capture {capture.report("admission","handshake_rejected",None,"none");} connection.close(0u32.into(),b"admission"); return;}; let _=connection_loop(connection.clone(),&security,topology,omp,codex,omp_first_guard,codex_invalidation_reporter,hello_capture,permit,attempt).await; connection.close(0u32.into(),b"closed");});},
            Some(_)=jobs.join_next(),if !jobs.is_empty()=>{},
        }
    }
    jobs.abort_all();
    while jobs.join_next().await.is_some() {}
    if let Some(push)=security.push.get(){push.cancel_all();}
    let _=push_stop.send(true);
    let _=push_job.await;
    if let Some(job) = omp_job {
        let _ = job.await;
    }
    if let Some(job) = codex_job {
        let _ = job.await;
    }
    Ok(())
}
async fn connection_loop(
    connection: Connection,
    security: &Security,
    topology: Option<Arc<crate::herdr::Topology>>,
    omp: Option<Arc<crate::backends::omp::Runtime>>,
    codex: Option<Arc<crate::backends::codex::Runtime>>,
    omp_first_guard: Option<crate::herdr::OmpCreationReporter>,
    codex_invalidation_reporter: Option<crate::herdr::CodexInvalidationReporter>,
    hello_capture: Option<HelloCaptureReporter>,
    permit: tokio::sync::OwnedSemaphorePermit,
    attempt: HandshakeAttempt<'_>,
) -> io::Result<()> {
    if connection.alpn() != ALPN {
        if let Some(capture) = &hello_capture {
            capture.report("admission", "alpn_mismatch", None, "none");
        }
        return Err(invalid());
    }
    let remote = EndpointId(connection.remote_id().to_string());
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    if let Some(capture) = &hello_capture {
        capture.report("stream_accept", "entered", None, "none");
    }
    let (mut send, mut recv) = match tokio::time::timeout_at(deadline, connection.accept_bi()).await {
        Ok(Ok(stream)) => {
            stream
        }
        Ok(Err(_)) => {
            if let Some(capture) = &hello_capture {
                capture.report("stream_accept", "stream_error", None, "other");
            }
            return Err(invalid());
        }
        Err(_) => {
            if let Some(capture) = &hello_capture {
                capture.report("stream_accept", "deadline", Some(ErrorCode::Timeout), "timed_out");
            }
            return Err(invalid());
        }
    };
    if let Some(capture) = &hello_capture {
        capture.report("hello_frame", "entered", None, "none");
    }
    let mut first = match tokio::time::timeout_at(deadline, read_frame(&mut recv, 4096)).await {
        Ok(Ok(frame)) => frame,
        Ok(Err(error)) => {
            if let Some(capture) = &hello_capture {
                let (code, io) = hello_frame_error(&error);
                capture.report("hello_frame", "read_error", code, io);
            }
            return Err(error);
        }
        Err(_) => {
            if let Some(capture) = &hello_capture {
                capture.report("hello_frame", "deadline", Some(ErrorCode::Timeout), "timed_out");
            }
            return Err(invalid());
        }
    };
    let mut handshake = match request(&first) {
        Ok(request) => request,
        Err(error) => {
            if let Some(capture) = &hello_capture {
                capture.report("hello_frame", "invalid_request", Some(ErrorCode::InvalidRequest), "none");
            }
            return Err(error);
        }
    };
    first.fill(0);
    if let RequestBody::PairConfirm(params) = handshake.body {
        let result = security.confirm(&connection, params).await;
        let paired = result.is_ok();
        response(&mut send, handshake.id, result.map(ResultBody::PairConfirm)).await?;
        if !paired {
            let _ = finish_reply(&mut send).await;
            return Err(invalid());
        }
        // A spent token is never retried. Only authenticated known-peer hello may follow.
        let mut bytes = match tokio::time::timeout_at(deadline, read_frame(&mut recv, 4096)).await {
            Ok(Ok(frame)) => frame,
            Ok(Err(error)) => {
                if let Some(capture) = &hello_capture {
                    let (code, io) = hello_frame_error(&error);
                    capture.report("hello_frame", "read_error", code, io);
                }
                return Err(error);
            }
            Err(_) => {
                if let Some(capture) = &hello_capture {
                    capture.report("hello_frame", "deadline", Some(ErrorCode::Timeout), "timed_out");
                }
                return Err(invalid());
            }
        };
        handshake = match request(&bytes) {
            Ok(request) => request,
            Err(error) => {
                if let Some(capture) = &hello_capture {
                    capture.report("hello_frame", "invalid_request", Some(ErrorCode::InvalidRequest), "none");
                }
                return Err(error);
            }
        };
        bytes.fill(0);
    }
    let RequestBody::Hello(params) = handshake.body else {
        if let Some(capture) = &hello_capture {
            capture.report("hello_frame", "not_hello", Some(ErrorCode::Unauthorized), "none");
        }
        response(&mut send, handshake.id, Err(ErrorCode::Unauthorized)).await?;
        let _ = finish_reply(&mut send).await;
        return Err(invalid());
    };
    if let Some(capture) = &hello_capture {
        capture.report("hello_authorization", "entered", None, "none");
    }
    let auth = match security.hello(remote, params, connection.clone()).await {
        Ok(a) => {
            Arc::new(a)
        }
        Err(code) => {
            if let Some(capture) = &hello_capture {
                capture.report("hello_authorization", "rejected", Some(code), "none");
            }
            response(&mut send, handshake.id, Err(code)).await?;
            let _ = finish_reply(&mut send).await;
            return Err(invalid());
        }
    };
    let _active = ActiveConnection {
        security,
        auth: &auth,
    };
    attempt.succeeded();
    drop(permit);
    {
        if let Some(capture) = &hello_capture {
            capture.report("security_guard", "entered", None, "none");
        }
        let _guard = match security.guard(&auth).await {
            Ok(guard) => {
                guard
            }
            Err(code) => {
                if let Some(capture) = &hello_capture {
                    capture.report("security_guard", "rejected", Some(code), "none");
                }
                return Err(invalid());
            }
        };
        if let Some(capture) = &hello_capture {
            capture.report("backend_capabilities", "entered", None, "none");
        }
        let backends = security
            .backends
            .iter()
            .map(|backend| {
                if backend.provider == Provider::Omp {
                    omp.as_ref()
                        .map_or_else(|| backend.clone(), |owner| owner.backend())
                } else {
                    codex
                        .as_ref()
                        .map_or_else(|| backend.clone(), |owner| owner.backend())
                }
            })
            .collect();
        let hello = ResultBody::Hello(HelloResult {
            host_id: security.host.clone(),
            device_id: auth.proof.peer_id().clone(),
            boot_id: security.boot.clone(),
            protocol: ProtocolVersion(2),
            backends,
        });
        if let Some(capture) = &hello_capture {
            capture.report("response_write", "entered", None, "none");
        }
        match response(&mut send, handshake.id, Ok(hello)).await {
            Ok(()) => {}
            Err(error) => {
                if let Some(capture) = &hello_capture {
                    let (code, io) = hello_frame_error(&error);
                    capture.report("response_write", "write_error", code, io);
                }
                return Err(error);
            }
        }
    }
    if let Some(capture) = &hello_capture {
        capture.report("complete", "completed", None, "none");
    }
    let (tx, mut rx) = mpsc::channel(64);
    let inflight = Arc::new(Semaphore::new(64));
    let reader_reporter = omp_first_guard.clone();
    let reader_codex_reporter = codex_invalidation_reporter.clone();
    let reader_connection = auth.connection_id.clone();
    let mut reader = ReaderTask(tokio::spawn(async move {
        let _reader_panic_reporter = ConnectionPanicReporter {
            reporter: reader_reporter.clone(),
            connection: reader_connection.clone(),
            guard: "reader_task_panicked",
        };
        let mut rate = RequestRate::default();
        loop {
            let Ok(permit) = inflight.clone().try_acquire_owned() else {
                report_codex_transport(
                    reader_codex_reporter.as_ref(),
                    &reader_connection,
                    "reader_inflight",
                    None,
                    "none",
                );
                return;
            };
            let mut bytes = match read_frame(&mut recv, FRAME_MAX).await {
                Ok(bytes) => bytes,
                Err(error) => {
                    report_transport_frame(
                        reader_reporter.as_ref(),
                        &reader_connection,
                        "reader_frame",
                        &error,
                    );
                    report_codex_transport_frame(
                        reader_codex_reporter.as_ref(),
                        &reader_connection,
                        "reader_frame",
                        &error,
                    );
                    return;
                }
            };
            let frame = decode_payload(&bytes, None);
            bytes.fill(0);
            if !rate.take() {
                let id = match &frame {
                    Ok(Envelope::Request(request)) => Some(request.id.clone()),
                    Err(failure) => failure.request_id.clone(),
                    _ => None,
                };
                if tx
                    .send((Err(ErrorCode::RateLimited), id, false, permit))
                    .await
                    .is_err()
                {
                    report_codex_transport(
                        reader_codex_reporter.as_ref(),
                        &reader_connection,
                        "reader_rate_queue",
                        Some(ErrorCode::RateLimited),
                        "cancelled",
                    );
                    return;
                }
                continue;
            }
            match frame {
                Ok(Envelope::Request(request)) => {
                    if tx.send((Ok(request), None, false, permit)).await.is_err() {
                        report_codex_transport(
                            reader_codex_reporter.as_ref(),
                            &reader_connection,
                            "reader_request_queue",
                            None,
                            "cancelled",
                        );
                        return;
                    }
                }
                Err(failure) => {
                    let code = match failure.kind {
                        FailureKind::UnsupportedVersion => ErrorCode::UnsupportedVersion,
                        FailureKind::UnsupportedMethod => ErrorCode::UnsupportedMethod,
                        _ => ErrorCode::InvalidRequest,
                    };
                    let close = failure.close;
                    if tx
                        .send((Err(code), failure.request_id, close, permit))
                        .await
                        .is_err()
                    {
                        report_codex_transport(
                            reader_codex_reporter.as_ref(),
                            &reader_connection,
                            "reader_decode_queue",
                            Some(code),
                            "cancelled",
                        );
                        return;
                    }
                    if close {
                        report_codex_transport(
                            reader_codex_reporter.as_ref(),
                            &reader_connection,
                            "reader_decode_close",
                            Some(code),
                            "none",
                        );
                        return;
                    }
                }
                _ => {
                    report_codex_transport(
                        reader_codex_reporter.as_ref(),
                        &reader_connection,
                        "reader_non_request",
                        Some(ErrorCode::InvalidRequest),
                        "none",
                    );
                    return;
                }
            }
        }
    }));
    let _panic_reporter = ConnectionPanicReporter {
        reporter: omp_first_guard.clone(),
        connection: auth.connection_id.clone(),
        guard: "connection_task_panicked",
    };
    let mut views = Viewing::default();
    let mut close_after_output = false;
    let mut omp_response_pending = false;
    let mut reader_joined = false;
    let result = async {
        let mut ticks = tokio::time::interval(Duration::from_millis(10));
        loop {
            tokio::select! {
                extra = connection.accept_bi() => {
                    if let Some(reporter) = &omp_first_guard {
                        reporter.transport_failure(&auth.connection_id, if extra.is_ok() { "extra_stream" } else { "extra_stream_error" }, None, "none", "none");
                    }
                    report_codex_transport(
                        codex_invalidation_reporter.as_ref(),
                        &auth.connection_id,
                        if extra.is_ok() { "extra_stream" } else { "extra_stream_error" },
                        None,
                        "none",
                    );
                    return Err(invalid());
                }
                item = rx.recv(), if !close_after_output => {
                    let Some((frame, id, close, _permit)) = item else {
                        if reader.0.is_finished() {
                            let _ = (&mut reader.0).await;
                            reader_joined = true;
                            report_codex_transport(
                                codex_invalidation_reporter.as_ref(),
                                &auth.connection_id,
                                "output_drained_after_reader",
                                None,
                                "none",
                            );
                        }
                        close_after_output = true;
                        continue;
                    };
                    let request = match frame {
                        Ok(request) => request,
                        Err(code) => {
                            if let Some(id) = id {
                                let _guard = security.guard(&auth).await.inspect_err(|code| {
                                    report_codex_transport(codex_invalidation_reporter.as_ref(), &auth.connection_id, "response_guard", Some(*code), "none");
                                }).map_err(|_| invalid())?;
                                security.state.response(&auth.connection_id, Response { id, outcome: Err(wire_error(code)) }).inspect_err(|code| {
                                    report_codex_transport(codex_invalidation_reporter.as_ref(), &auth.connection_id, "response_queue", Some(*code), "none");
                                }).map_err(|_| invalid())?;
                                close_after_output = close;
                            } else { return Err(invalid()); }
                            continue;
                        }
                    };
                    if let RequestBody::DeviceUnpair(params) = &request.body {
                        {
                            let _guard = security.guard(&auth).await.map_err(|_| invalid())?;
                            if params.boot_id != security.boot {
                                security.state.response(&auth.connection_id, Response { id: request.id, outcome: Err(wire_error(ErrorCode::StaleHost)) }).map_err(|_| invalid())?;
                                continue;
                            }
                        }
                        security.revoke(auth.proof.peer_id().clone(), Some(auth.connection_id.clone())).await.map_err(|_| invalid())?;
                        // Only this writer may bypass the now-removed epoch for this exact final reply.
                        security.state.self_unpair_final(&auth.connection_id, request.id).map_err(|_| invalid())?;
                        let (bytes, _) = security.state.pop_output(&auth.connection_id).map_err(|_| invalid())?;
                        let final_result = if let Some(bytes) = bytes {
                            match write_frame(&mut send, &bytes).await { Ok(()) => finish_reply(&mut send).await, Err(error) => Err(error) }
                        } else { Err(invalid()) };
                        security.state.revoke(auth.proof.peer_id());
                        connection.close(0u32.into(), b"unpaired");
                        return final_result;
                    }
                    let id = request.id.clone();
                    let omp_create = matches!(&request.body, RequestBody::SessionCreate(params) if params.provider == Provider::Omp);
                    let codex_create = matches!(&request.body, RequestBody::SessionCreate(params) if params.provider == Provider::Codex);
                    if omp_create && let Some(reporter) = &omp_first_guard { reporter.arm_transport(&auth.connection_id); }
                    if codex_create && let Some(reporter) = &codex_invalidation_reporter { reporter.arm_transport(&auth.connection_id); }
                    let timeout = if matches!(&request.body, RequestBody::SessionCreate(_)) { Duration::from_secs(120) } else { Duration::from_secs(15) };
                    let mut dispatch = Some(Box::pin(dispatch(
                        security,
                        &auth,
                        topology.clone(),
                        omp.clone(),
                        codex.clone(),
                        request,
                        &mut views,
                    )));
                    let mut deadline = Box::pin(tokio::time::sleep(timeout));
                    let mut extra = Box::pin(async { connection.accept_bi().await.is_ok() });
                    let mut dispatch_timed_out = false;
                    let result = loop {
                        tokio::select! {
                            result = dispatch.as_mut().expect("pending dispatch") => break result,
                            _ = &mut deadline => {
                                dispatch_timed_out = true;
                                break Err(ErrorCode::Timeout);
                            }
                            success = &mut extra => {
                                if let Some(reporter) = &omp_first_guard {
                                    reporter.transport_failure(
                                        &auth.connection_id,
                                        if success { "extra_stream" } else { "extra_stream_error" },
                                        None,
                                        "none",
                                        "none",
                                    );
                                }
                                report_codex_transport(
                                    codex_invalidation_reporter.as_ref(),
                                    &auth.connection_id,
                                    if success { "extra_stream" } else { "extra_stream_error" },
                                    None,
                                    "none",
                                );
                                drop(dispatch.take());
                                return Err(invalid());
                            }
                            _ = ticks.tick() => {
                                let mut output = Box::pin(output_step(
                                    &mut send,
                                    security,
                                    &auth,
                                    close_after_output,
                                    omp_first_guard.as_ref(),
                                    codex_invalidation_reporter.as_ref(),
                                    &mut omp_response_pending,
                                ));
                                match poll_dispatch_output(
                                    dispatch.as_mut().expect("pending dispatch").as_mut(),
                                    deadline.as_mut(),
                                    output.as_mut(),
                                    extra.as_mut(),
                                ).await {
                                    DispatchPoll::Complete(result) => {
                                        drop(dispatch.take());
                                        let step = (&mut output).await?;
                                        drop(output);
                                        match step {
                                            OutputStep::Continue => break result,
                                            OutputStep::Closed => return finish_reply(&mut send).await.inspect_err(|error| {
                                                report_transport_frame(omp_first_guard.as_ref(), &auth.connection_id, "output_finish", error);
                                                report_codex_transport_frame(codex_invalidation_reporter.as_ref(), &auth.connection_id, "output_finish", error);
                                            }),
                                        }
                                    }
                                    DispatchPoll::Deadline => {
                                        dispatch_timed_out = true;
                                        drop(dispatch.take());
                                        let step = (&mut output).await?;
                                        drop(output);
                                        match step {
                                            OutputStep::Continue => break Err(ErrorCode::Timeout),
                                            OutputStep::Closed => return finish_reply(&mut send).await.inspect_err(|error| {
                                                report_transport_frame(omp_first_guard.as_ref(), &auth.connection_id, "output_finish", error);
                                                report_codex_transport_frame(codex_invalidation_reporter.as_ref(), &auth.connection_id, "output_finish", error);
                                            }),
                                        }
                                    }
                                    DispatchPoll::ExtraStream(success) => {
                                        drop(output);
                                        drop(dispatch.take());
                                        if let Some(reporter) = &omp_first_guard {
                                            reporter.transport_failure(
                                                &auth.connection_id,
                                                if success { "extra_stream" } else { "extra_stream_error" },
                                                None,
                                                "none",
                                                "none",
                                            );
                                        }
                                        report_codex_transport(
                                            codex_invalidation_reporter.as_ref(),
                                            &auth.connection_id,
                                            if success { "extra_stream" } else { "extra_stream_error" },
                                            None,
                                            "none",
                                        );
                                        return Err(invalid());
                                    }
                                    DispatchPoll::Output(step) => {
                                        drop(output);
                                        match step? {
                                            OutputStep::Continue => continue,
                                            OutputStep::Closed => {
                                                drop(dispatch.take());
                                                return finish_reply(&mut send).await.inspect_err(|error| {
                                                    report_transport_frame(omp_first_guard.as_ref(), &auth.connection_id, "output_finish", error);
                                                    report_codex_transport_frame(codex_invalidation_reporter.as_ref(), &auth.connection_id, "output_finish", error);
                                                });
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    };
                    drop(dispatch.take());
                    drop(extra);
                    if omp_create && let Some(reporter) = &omp_first_guard {
                        if dispatch_timed_out {
                            reporter.report("transport", "dispatch_deadline", Some(ErrorCode::Timeout));
                        } else {
                            reporter.report("transport", "dispatch_returned", result.as_ref().err().copied());
                        }
                    }
                    if let Some(outcome) = result.transpose() {
                        let _guard = security.guard(&auth).await.inspect_err(|code| {
                            if let Some(reporter) = &omp_first_guard {
                                reporter.transport_failure(&auth.connection_id, "response_guard", Some(*code), "none", "none");
                            }
                            report_codex_transport(codex_invalidation_reporter.as_ref(), &auth.connection_id, "response_guard", Some(*code), "none");
                        }).map_err(|_| invalid())?;
                        security.state.response(&auth.connection_id, Response { id, outcome: outcome.map_err(wire_error) })
                            .inspect_err(|code| {
                                if let Some(reporter) = &omp_first_guard {
                                    reporter.transport_failure(&auth.connection_id, "response_queue", Some(*code), "none", "none");
                                }
                            report_codex_transport(codex_invalidation_reporter.as_ref(), &auth.connection_id, "response_queue", Some(*code), "none");
                            })
                            .map_err(|_| invalid())?;
                        if omp_create && let Some(reporter) = &omp_first_guard {
                            reporter.report("transport", "response_queued", None);
                        }
                        if omp_create { omp_response_pending = true; }
                    }
                }
                _ = ticks.tick() => {
                    match output_step(&mut send, security, &auth, close_after_output, omp_first_guard.as_ref(), codex_invalidation_reporter.as_ref(), &mut omp_response_pending).await? {
                        OutputStep::Continue => {}
                        OutputStep::Closed => {
                            return finish_reply(&mut send).await.inspect_err(|error| {
                                report_transport_frame(omp_first_guard.as_ref(), &auth.connection_id, "output_finish", error);
                                report_codex_transport_frame(codex_invalidation_reporter.as_ref(), &auth.connection_id, "output_finish", error);
                            });
                        }
                    }
                }
            }
        }
    }.await;
    connection.close(0u32.into(), b"closed");
    if !reader_joined {
        reader.0.abort();
        let _ = (&mut reader.0).await;
    }
    security.disconnect(&auth);
    result
}

fn workspace_rename_lost() -> crate::storage::ReceiptTransition {
    crate::storage::ReceiptTransition {
        state: ReceiptState::Uncertain,
        proof: crate::storage::TransitionProof::LostOutcome,
        result: None,
        error: None,
        at: now_ms(),
    }
}
async fn workspace_rename_no_effect(
    security: &Security,
    auth: &Authorized,
    action: &Uuid,
    code: ErrorCode,
) -> SecurityResult<Receipt> {
    let proof = auth.proof.clone();
    let action = action.clone();
    security.storage.call(move |store| store.transition(&proof, &action, crate::storage::ReceiptTransition {
        state: ReceiptState::Failed,
        proof: crate::storage::TransitionProof::DefinitiveNoEffect(Id("workspace rename native boundary not entered".into())),
        result: None,
        error: Some(wire_error(code)),
        at: now_ms(),
    })).await.map_err(store_error)
}

fn delivered_notifications(bytes: &[u8]) -> io::Result<Vec<AttentionEvent>> {
    let raw = parse_raw_json(bytes).map_err(|_| invalid())?;
    if raw.get("type").and_then(|v| v.as_str()) == Some("response")
        && let Some(v) = raw.get("result")
        && v.get("events").is_some()
        && v.get("latestSequence").is_some()
    {
        return serde_json::from_value::<NotificationsSyncResult>(v.clone())
            .map(|r| r.events)
            .map_err(|_| invalid());
    }
    if raw.get("kind").and_then(|v| v.as_str()) == Some("notification.upsert") {
        return serde_json::from_value::<NotificationUpsertEvent>(raw["data"].clone())
            .map(|e| vec![e.event])
            .map_err(|_| invalid());
    }
    Ok(Vec::new())
}
async fn dispatch(
    // Routing reads current reducer authority, never desktop focus/native ID guesses.
    security: &Security,
    auth: &Arc<Authorized>,
    topology: Option<Arc<crate::herdr::Topology>>,
    omp: Option<Arc<crate::backends::omp::Runtime>>,
    codex: Option<Arc<crate::backends::codex::Runtime>>,
    r: Request,
    views: &mut Viewing,
) -> SecurityResult<Option<ResultBody>> {
    let guard = security.guard(auth).await?;
    let p = auth.proof.clone();
    let result = match r.body {
        body @ (RequestBody::ArtifactsList(_)|RequestBody::ArtifactsOpen(_)|RequestBody::ArtifactsRead(_)|RequestBody::ArtifactsClose(_))=>{
            let initial=if let RequestBody::ArtifactsList(params)=&body{if params.parent.is_none()&&params.cursor.is_none(){Some(match &params.source{ArtifactSource::Session{target}=>match target_provider(&security.state,target)?{Provider::Omp=>omp.as_ref().ok_or(ErrorCode::NotReady)?.artifact_context(target)?,Provider::Codex=>codex.as_ref().ok_or(ErrorCode::NotReady)?.artifact_context(target)?},ArtifactSource::Shell{target}=>topology.as_ref().ok_or(ErrorCode::NotReady)?.shell_artifact_context(target)?})}else{None}}else{None};
            let topology=topology.ok_or(ErrorCode::NotReady)?;drop(guard);let (context,result,mut opening)=topology.artifact_request(auth.proof.peer_id(),&body,initial).await?;let _guard=security.guard(auth).await?;topology.enqueue_artifact(&auth.connection_id,r.id,&context,result)?;if let Some(opening)=opening.as_mut(){opening.published()}return Ok(None);
        }
        RequestBody::HostSnapshot(_) => {
            security.state.host_snapshot(&auth.connection_id, r.id)?;
            return Ok(None);
        }
        RequestBody::SessionOpen(v) => {
            let provider = target_provider(&security.state, &v.target)?;
            let sub = if provider == Provider::Codex {
                if let Some(owner) = &codex {
                    drop(guard);
                    owner.session_open(auth, r.id, &v.target).await?
                } else {
                    security
                        .state
                        .session_open(&auth.connection_id, r.id, &v.target, now_ms())?
                }
            } else if let Some(owner) = &omp {
                owner.session_open(auth, r.id, &v.target)?
            } else {
                security
                    .state
                    .session_open(&auth.connection_id, r.id, &v.target, now_ms())?
            };
            views.session = Some(v.target.session_id.clone());
            views.subscriptions.insert(sub, v.target.session_id);
            return Ok(None);
        }
        RequestBody::SessionHistory(v) => {
            if target_provider(&security.state, &v.target)? == Provider::Codex
                && let Some(owner) = &codex
            {
                drop(guard);
                ResultBody::SessionHistory(owner.history(&p, &v).await?)
            } else if let Some(owner) = &omp {
                ResultBody::SessionHistory(owner.history(&p, &v).await?)
            } else {
                return Err(ErrorCode::NotReady);
            }
        }
        RequestBody::SessionUnsubscribe(v) => {
            security
                .state
                .unsubscribe(&auth.connection_id, &v.subscription)?;
            if views.subscriptions.remove(&v.subscription).as_ref() == views.session.as_ref() {
                views.session = None;
            }
            ResultBody::SessionUnsubscribe(SessionUnsubscribeResult {
                unsubscribed: TrueValue(true),
            })
        }
        RequestBody::ContentRead(v) => {
            ResultBody::ContentRead(security.state.read_content(&p, &v, now_ms())?)
        }
        RequestBody::WorkspaceRename(v) => {
            // Reserve before topology acquisition: an exact duplicate returns its durable receipt
            // even if the original desktop source is now unavailable.
            let request = Request { id: r.id.clone(), body: RequestBody::WorkspaceRename(v.clone()) };
            let state = security.state.clone();
            drop(guard);
            let reservation = security.reserve(auth, request, move |request| {
                state.validate_session_mutation(request)
            }, |_| {}).await?;
            let receipt = match reservation {
                Reservation::Existing(receipt) => receipt,
                Reservation::Reserved(_) => {
                    match topology {
                        None => workspace_rename_no_effect(security, auth, &r.id, ErrorCode::NotReady).await?,
                        Some(topology) => match topology.workspace_rename_context(&v.target).await {
                        Err(code) => workspace_rename_no_effect(security, auth, &r.id, code).await?,
                        Ok((source, context)) => match crate::herdr::Client::connect(&source.socket_path).await {
                            Err(_) => workspace_rename_no_effect(security, auth, &r.id, ErrorCode::NotReady).await?,
                            Ok(client) if client.evidence != source.evidence =>
                                workspace_rename_no_effect(security, auth, &r.id, ErrorCode::StaleSession).await?,
                            Ok(client) => match topology.validate_workspace_rename_context(&v.target, &source, &context).await {
                                Err(code) => workspace_rename_no_effect(security, auth, &r.id, code).await?,
                                Ok(()) => match client.fence_socket() {
                                    Err(_) => workspace_rename_no_effect(security, auth, &r.id, ErrorCode::NotReady).await?,
                                    Ok(fd) => {
                                        let target = v.target.clone();
                                        let label = v.label.clone();
                                        let boot = security.boot.clone();
                                        let expected = context.clone();
                                        let validation_state = security.state.clone();
                                        let validation_topology = topology.clone();
                                        let validation_source = source.clone();
                                        let validation_context = context.clone();
                                        let effect_target = target.clone();
                                        let boundary = security.boundary(auth, r.id.clone(), move || {
                                            validation_topology.validate_workspace_rename_context_now(&target, &validation_source, &validation_context)?;
                                            validation_state.validate_session_mutation(&Request {
                                                id: Uuid::random(),
                                                body: RequestBody::WorkspaceRename(WorkspaceRenameParams {
                                                    boot_id: boot, target, label,
                                                }),
                                            })
                                        }, fd, async move {
                                            let accepted = |reply: &crate::herdr::WorkspaceRenameBoundResult| {
                                                reply.label == v.label.0
                                                    && reply.scope.pane_id == expected.pane
                                                    && reply.scope.terminal_id == expected.terminal
                                                    && reply.scope.workspace_id == expected.workspace
                                                    && reply.scope.tab_id == expected.tab
                                                    && reply.scope.read_identity == expected.read_identity
                                            };
                                            let success = || crate::storage::ReceiptTransition {
                                                state: ReceiptState::Succeeded,
                                                proof: crate::storage::TransitionProof::CorrelatedAcknowledgement(Id("native workspace rename acknowledged".into())),
                                                result: Some(ActionResult::WorkspaceRenamed(ActionResultWorkspaceRenamed {
                                                    target: effect_target.clone(), workspace_id: expected.workspace.clone(),
                                                })), error: None, at: now_ms(),
                                            };
                                            match client.workspace_rename_bound(&expected.pane, &expected.terminal, &expected.read_identity, &v.label).await {
                                                Ok(reply) if accepted(&reply) => success(),
                                                Ok(_) => workspace_rename_lost(),
                                                Err(crate::herdr::ApiError::Changed(value)) => match crate::herdr::decode_workspace_rename_bound(value) {
                                                    Ok(reply) if accepted(&reply) => success(),
                                                    _ => workspace_rename_lost(),
                                                },
                                                Err(error @ crate::herdr::ApiError::Rejected(_)) => crate::storage::ReceiptTransition {
                                                    state: ReceiptState::Failed,
                                                    proof: crate::storage::TransitionProof::DefinitiveNoEffect(Id("native workspace rename rejected before acknowledgement".into())),
                                                    result: None, error: Some(wire_error(crate::herdr::workspace_rename_error_code(error))), at: now_ms(),
                                                },
                                                Err(crate::herdr::ApiError::Unavailable(_)) => workspace_rename_lost(),
                                            }
                                        }).await;
                                        let proof = auth.proof.clone();
                                        let action = r.id.clone();
                                        let receipt = security.storage.call(move |store| store.status(&proof, &action, now_ms())).await.map_err(store_error)?
                                            .ok_or(ErrorCode::NotReady)?;
                                        boundary?;
                                        receipt
                                    }
                                },
                            },
                        },
                    }
                }
                }
            };
            ResultBody::WorkspaceRename(WorkspaceRenameResult { receipt })
        }
        RequestBody::ShellRead(params) => {
            let topology=topology.ok_or(ErrorCode::NotReady)?;drop(guard);
            let (context,result)=topology.shell_read(&params.target).await?;
            let _guard=security.guard(auth).await?;
            topology.enqueue_shell_read(&auth.connection_id,r.id,&context,result).await?;
            return Ok(None);
        }
        RequestBody::TerminalCreate(params)=>{
            // Pin the original provider context before the first reservation await.
            // Existing receipt dedupe still wins even if that original context is unavailable.
            let captured=(||{match target_provider(&security.state,&params.target)? {Provider::Omp=>omp.as_ref().ok_or(ErrorCode::NotReady)?.agent_terminal_context(&params.target),Provider::Codex=>codex.as_ref().ok_or(ErrorCode::NotReady)?.agent_terminal_context(&params.target)}})();
            let expected_source=captured.as_ref().map(|context|context.epoch.clone()).map_err(|code|*code);
            let state=security.state.clone();let request=Request {id:r.id.clone(),body:RequestBody::TerminalCreate(params.clone())};drop(guard);
            let reservation=security.reserve(auth,request,move|request|{let source=expected_source.as_ref().map_err(|code|*code)?;state.validate_session_mutation_checked(request,Some(source)).map(|_|()).map_err(|(_,code)|code)},|_|{}).await?;
            let receipt=match reservation {Reservation::Existing(receipt)=>receipt,Reservation::Reserved(_)=>{
                let run=async {let context=captured?;
                    topology.ok_or(ErrorCode::NotReady)?.agent_terminal_create_once(security,auth,&r.id,context).await
                }.await;
                let proof=auth.proof.clone();let action=r.id.clone();let mut receipt=security.storage.call(move|store|store.status(&proof,&action,now_ms())).await.map_err(store_error)?.ok_or(ErrorCode::NotReady)?;
                if receipt.state==ReceiptState::Reserved {let proof=auth.proof.clone();let action=r.id.clone();let code=run.err().unwrap_or(ErrorCode::NotReady);receipt=security.storage.call(move|store|store.transition(&proof,&action,crate::storage::ReceiptTransition {state:ReceiptState::Failed,proof:crate::storage::TransitionProof::DefinitiveNoEffect(Id("agent terminal boundary not entered".into())),result:None,error:Some(wire_error(code)),at:now_ms()})).await.map_err(store_error)?;}receipt
            }};ResultBody::TerminalCreate(TerminalCreateResult {receipt})
        }
        RequestBody::AttachmentsStatus(params)=>{
            let context=match target_provider(&security.state,&params.target).ok() {Some(Provider::Omp)=>omp.as_ref().and_then(|runtime|runtime.attachment_context(&params.target).ok()),Some(Provider::Codex)=>codex.as_ref().and_then(|runtime|runtime.attachment_context(&params.target).ok()),None=>None};
            let original=context.clone();let upload=params.upload_id.clone();let proof=auth.proof.clone();drop(guard);
            let (mut status,record)=security.storage.call(move|store|store.authorized_attachment_status(&proof,&params,context.as_ref())).await.map_err(store_error)?;
            let _guard=security.guard(auth).await?;
            if security.attachment_claim_busy(auth,&upload){status.insertion_ready=false;status.state=AttachmentUploadState::Uncertain}
            if let Some(original)=original {original.enqueue_status(&security.state,&auth.connection_id,r.id,status,&record)?;}
            else {status.insertion_ready=false;security.state.attachments_status_response(&auth.connection_id,r.id,status,None)?;}return Ok(None);
        }
        body @ (RequestBody::AttachmentsBegin(_)|RequestBody::AttachmentsChunk(_)|RequestBody::AttachmentsComplete(_)|RequestBody::AttachmentsDiscard(_))=>{
            let request=Request {id:r.id.clone(),body:body.clone()};let state=security.state.clone();drop(guard);
            let reservation=security.reserve(auth,request,move|request|state.validate_attachment_mutation(request),|_|{}).await?;
            let receipt=match reservation {
                Reservation::Existing(receipt)=>receipt,
                Reservation::Reserved(_)=>{
                    let run=async {
                        let (_,_,target,upload)=body.attachment_action().ok_or(ErrorCode::InvalidRequest)?;
                        if let RequestBody::AttachmentsDiscard(params)=&body {
                            let proof=auth.proof.clone();let owned=params.clone();
                            let cleanup=security.storage.call(move|store|store.attachment_cleanup_context(&proof,&owned)).await.map_err(store_error)?;
                            let effect_body=body.clone();let peer=auth.proof.peer_id().clone();let action=r.id.clone();
                            security.attachment_cleanup_boundary(auth,r.id.clone(),params,&cleanup,|checkpoint|async move {checkpoint.execute(peer,action,effect_body,None).await}).await
                        }else{
                            let provider=target_provider(&security.state,target)?;
                            let context=match provider {Provider::Omp=>omp.as_ref().ok_or(ErrorCode::NotReady)?.attachment_context(target)?,Provider::Codex=>codex.as_ref().ok_or(ErrorCode::NotReady)?.attachment_context(target)?};
                            let client=crate::herdr::Client::connect(&context.source.socket_path).await.map_err(|_|ErrorCode::NotReady)?;
                            if client.evidence!=context.source.evidence{return Err(ErrorCode::StaleSession)}context.recheck()?;
                            let fd=client.fence_socket().map_err(|_|ErrorCode::NotReady)?;
                            let original=context.clone();let effect_body=body.clone();let peer=auth.proof.peer_id().clone();let action=r.id.clone();
                            security.attachment_boundary(auth,r.id.clone(),target.clone(),upload.clone(),fd,||{original.recheck()?;security.state.validate_attachment_mutation(&Request {id:r.id.clone(),body:body.clone()})},|checkpoint|async move {checkpoint.execute(peer,action,effect_body,Some(context)).await}).await
                        }
                    }.await;
                    let proof=auth.proof.clone();let action=r.id.clone();let mut receipt=security.storage.call(move|store|store.status(&proof,&action,now_ms())).await.map_err(store_error)?.ok_or(ErrorCode::NotReady)?;
                    if receipt.state==ReceiptState::Reserved {let proof=auth.proof.clone();let action=r.id.clone();let code=run.err().unwrap_or(ErrorCode::NotReady);receipt=security.storage.call(move|store|store.transition(&proof,&action,crate::storage::ReceiptTransition {state:ReceiptState::Failed,proof:crate::storage::TransitionProof::DefinitiveNoEffect(Id("attachment boundary not entered".into())),result:None,error:Some(wire_error(code)),at:now_ms()})).await.map_err(store_error)?;}
                    receipt
                }
            };
            let result=AttachmentsResult {receipt};match body.method(){Method::AttachmentsBegin=>ResultBody::AttachmentsBegin(result),Method::AttachmentsChunk=>ResultBody::AttachmentsChunk(result),Method::AttachmentsComplete=>ResultBody::AttachmentsComplete(result),Method::AttachmentsDiscard=>ResultBody::AttachmentsDiscard(result),_=>return Err(ErrorCode::InvalidRequest)}
        }
        RequestBody::WorktreesRemovePreview(params)=>{
            let topology=topology.ok_or(ErrorCode::NotReady)?;drop(guard);
            let observation=topology.remove_preview(&params).await?;
            let _guard=security.guard(auth).await?;
            topology.enqueue_remove_preview(&auth.connection_id,r.id,observation).await?;return Ok(None);
        }
        RequestBody::WorktreesRemove(params) => {
            let request=Request {id:r.id.clone(),body:RequestBody::WorktreesRemove(params.clone())};
            let state=security.state.clone();drop(guard);
            let reservation=security.reserve(auth,request,move|request|state.validate_session_mutation(request),|_|{}).await?;
            let receipt=match reservation {
                Reservation::Existing(receipt)=>receipt,
                Reservation::Reserved(_)=>{
                    let outcome=match topology {Some(topology)=>topology.remove_worktree_once(security,auth,&r.id,&params).await,None=>Err(ErrorCode::NotReady)};
                    let proof=auth.proof.clone();let action=r.id.clone();
                    let mut receipt=security.storage.call(move|store|store.status(&proof,&action,now_ms())).await.map_err(store_error)?.ok_or(ErrorCode::NotReady)?;
                    if receipt.state==ReceiptState::Reserved {
                        let proof=auth.proof.clone();let action=r.id.clone();let code=outcome.err().unwrap_or(ErrorCode::NotReady);
                        receipt=security.storage.call(move|store|store.transition(&proof,&action,crate::storage::ReceiptTransition {
                            state:ReceiptState::Failed,proof:crate::storage::TransitionProof::DefinitiveNoEffect(Id("worktree removal native boundary not entered".into())),result:None,error:Some(wire_error(code)),at:now_ms(),
                        })).await.map_err(store_error)?;
                    }
                    receipt
                }
            };
            ResultBody::WorktreesRemove(WorktreesRemoveResult {receipt})
        }
        RequestBody::AgentRename(params) => {
            let request=Request {id:r.id.clone(),body:RequestBody::AgentRename(params.clone())};
            let state=security.state.clone();drop(guard);
            let reservation=security.reserve(auth,request,move|request|state.validate_session_mutation(request),|_|{}).await?;
            let receipt=match reservation {
                Reservation::Existing(receipt)=>receipt,
                Reservation::Reserved(_)=>{
                    let outcome=match topology {Some(topology)=>topology.agent_alias_once(security,auth,&r.id,&params).await,None=>Err(ErrorCode::NotReady)};
                    let proof=auth.proof.clone();let action=r.id.clone();
                    let mut receipt=security.storage.call(move|store|store.status(&proof,&action,now_ms())).await.map_err(store_error)?.ok_or(ErrorCode::NotReady)?;
                    if receipt.state==ReceiptState::Reserved {
                        let proof=auth.proof.clone();let action=r.id.clone();let code=outcome.err().unwrap_or(ErrorCode::NotReady);
                        receipt=security.storage.call(move|store|store.transition(&proof,&action,crate::storage::ReceiptTransition {
                            state:ReceiptState::Failed,proof:crate::storage::TransitionProof::DefinitiveNoEffect(Id("agent alias native boundary not entered".into())),result:None,error:Some(wire_error(code)),at:now_ms(),
                        })).await.map_err(store_error)?;
                    }
                    receipt
                }
            };
            ResultBody::AgentRename(AgentRenameResult {receipt})
        }
        RequestBody::ShellCreate(params) => {
            let request=Request {id:r.id.clone(),body:RequestBody::ShellCreate(params.clone())};
            let state=security.state.clone();drop(guard);
            let reservation=security.reserve(auth,request,move|request|state.validate_shell_mutation(request),|_|{}).await?;
            let receipt=match reservation {
                Reservation::Existing(receipt)=>receipt,
                Reservation::Reserved(_)=>{
                    let outcome=match topology {Some(topology)=>topology.shell_create_once(security,auth,&r.id,&params.target).await,None=>Err(ErrorCode::NotReady)};
                    let proof=auth.proof.clone();let action=r.id.clone();
                    let mut receipt=security.storage.call(move|store|store.status(&proof,&action,now_ms())).await.map_err(store_error)?.ok_or(ErrorCode::NotReady)?;
                    if receipt.state==ReceiptState::Reserved {
                        let proof=auth.proof.clone();let action=r.id.clone();let code=outcome.err().unwrap_or(ErrorCode::NotReady);
                        receipt=security.storage.call(move|store|store.transition(&proof,&action,crate::storage::ReceiptTransition {
                            state:ReceiptState::Failed,proof:crate::storage::TransitionProof::DefinitiveNoEffect(Id("shell native boundary not entered".into())),result:None,error:Some(wire_error(code)),at:now_ms(),
                        })).await.map_err(store_error)?;
                    }
                    receipt
                }
            };
            ResultBody::ShellCreate(ShellCreateResult {receipt})
        }
        RequestBody::ShellInput(params) => {
            let request=Request {id:r.id.clone(),body:RequestBody::ShellInput(params.clone())};
            let state=security.state.clone();drop(guard);
            let reservation=security.reserve(auth,request,move|request|state.validate_shell_mutation(request),|_|{}).await?;
            let receipt=match reservation {
                Reservation::Existing(receipt)=>receipt,
                Reservation::Reserved(_)=>{
                    let outcome=match topology {Some(topology)=>topology.shell_input_once(security,auth,&r.id,&params.target,&params.input).await,None=>Err(ErrorCode::NotReady)};
                    let proof=auth.proof.clone();let action=r.id.clone();
                    let mut receipt=security.storage.call(move|store|store.status(&proof,&action,now_ms())).await.map_err(store_error)?.ok_or(ErrorCode::NotReady)?;
                    if receipt.state==ReceiptState::Reserved {
                        let proof=auth.proof.clone();let action=r.id.clone();let code=outcome.err().unwrap_or(ErrorCode::NotReady);
                        receipt=security.storage.call(move|store|store.transition(&proof,&action,crate::storage::ReceiptTransition {
                            state:ReceiptState::Failed,proof:crate::storage::TransitionProof::DefinitiveNoEffect(Id("shell native boundary not entered".into())),result:None,error:Some(wire_error(code)),at:now_ms(),
                        })).await.map_err(store_error)?;
                    }
                    receipt
                }
            };
            ResultBody::ShellInput(ShellInputResult {receipt})
        }
        RequestBody::TerminalRead(v) => {
            let topology = topology.ok_or(ErrorCode::NotReady)?;
            drop(guard);
            let (source, result) = topology.terminal_read(&v.target).await?;
            let _guard = security.guard(auth).await?;
            topology
                .enqueue_terminal_read(&auth.connection_id, r.id, &source, result)
                .await?;
            return Ok(None);
        }
        RequestBody::WorktreesList(v) => {
            let topology = topology.ok_or(ErrorCode::NotReady)?;
            drop(guard);
            let (context, result) = topology.worktrees_list(&v.target).await?;
            let _guard = security.guard(auth).await?;
            topology
                .enqueue_worktrees_list(&auth.connection_id, r.id, &context, result)
                .await?;
            return Ok(None);
        }
        RequestBody::SkillsList(v) => {
            let provider=target_provider(&security.state,&v.target)?;drop(guard);
            match provider {
                Provider::Codex=>{let owner=codex.ok_or(ErrorCode::NotReady)?;let(context,result)=owner.skills_list(&v.target).await?;let _guard=security.guard(auth).await?;owner.enqueue_skills_list(&auth.connection_id,r.id,&context,result)?;},
                Provider::Omp=>{let owner=omp.ok_or(ErrorCode::NotReady)?;let(context,result)=owner.skills_list(&v.target).await?;let _guard=security.guard(auth).await?;owner.enqueue_skills_list(&auth.connection_id,r.id,&context,result)?;},
            }return Ok(None);
        }
        RequestBody::SkillsPreviewOpen(params)=>{
            if target_provider(&security.state,&params.target)?!=Provider::Omp{return Err(ErrorCode::NotReady)}let owner=omp.ok_or(ErrorCode::NotReady)?;drop(guard);
            let(context,result,mut pending)=owner.skills_preview_open(auth.proof.peer_id(),&params).await?;let _guard=security.guard(auth).await?;
            owner.enqueue_skills_preview(&auth.connection_id,r.id,&context,ResultBody::SkillsPreviewOpen(result))?;pending.published();return Ok(None);
        }
        RequestBody::SkillsPreviewRead(params)=>{
            if target_provider(&security.state,&params.target)?!=Provider::Omp{return Err(ErrorCode::NotReady)}let owner=omp.ok_or(ErrorCode::NotReady)?;drop(guard);
            let(context,result,mut pending)=owner.skills_preview_read(auth.proof.peer_id(),&params).await?;let _guard=security.guard(auth).await?;
            owner.enqueue_skills_preview(&auth.connection_id,r.id,&context,ResultBody::SkillsPreviewRead(result))?;pending.published();return Ok(None);
        }
        RequestBody::SkillsPreviewClose(params)=>{
            let owner=omp.ok_or(ErrorCode::NotReady)?;drop(guard);
            let result=owner.skills_preview_close(auth.proof.peer_id(),&params).await?;let _guard=security.guard(auth).await?;
            security.state.response(&auth.connection_id,Response{id:r.id,outcome:Ok(ResultBody::SkillsPreviewClose(result))})?;return Ok(None);
        }
        RequestBody::ChangesRead(v) => {
            let topology = topology.ok_or(ErrorCode::NotReady)?;
            drop(guard);
            let (context, result) = topology.changes_read(&v.target).await?;
            let _guard = security.guard(auth).await?;
            topology.enqueue_changes_read(&auth.connection_id, r.id, &context, result).await?;
            return Ok(None);
        }
        RequestBody::ActionStatus(v) => {
            let receipt = security
                .storage
                .call(move |s| s.status(&p, &v.action_id, now_ms()))
                .await
                .map_err(store_error)?;
            ResultBody::ActionStatus(match receipt {
                Some(receipt) => ActionStatusResult::Known(KnownActionStatus { receipt }),
                None => ActionStatusResult::Unknown(UnknownActionStatus {
                    state: UnknownValue("unknown".into()),
                }),
            })
        }
        RequestBody::NotificationsSync(v) => ResultBody::NotificationsSync(
            security
                .storage
                .call(move |s| {
                    s.notifications_sync(
                        &p,
                        v.after_sequence.unwrap_or(Uint(0)).0,
                        v.limit.unwrap_or(Uint(100)).0 as usize,
                        now_ms(),
                    )
                })
                .await
                .map_err(store_error)?,
        ),
        RequestBody::NotificationsRead(v) => {
            if v.boot_id != security.boot {
                return Err(ErrorCode::StaleHost);
            }
            let viewing = views.session.as_ref() == Some(&v.session_id);
            ResultBody::NotificationsRead(NotificationsReadResult {
                read: security
                    .storage
                    .call(move |s| {
                        s.notifications_read(&p, &v.session_id, v.through_sequence.0, viewing)
                    })
                    .await
                    .map_err(store_error)?,
            })
        }
        body @ (RequestBody::SessionCreate(_)
        | RequestBody::PromptSubmit(_)
        | RequestBody::TurnCancel(_)
        | RequestBody::RequestRespond(_)) => {
            let method = body.method();
            let request = Request { id: r.id, body };
            drop(guard);
            let provider = match &request.body {
                RequestBody::SessionCreate(p) => Some(p.provider),
                RequestBody::PromptSubmit(p) => target_provider(&security.state, &p.target).ok(),
                RequestBody::TurnCancel(p) => target_provider(&security.state, &p.target).ok(),
                RequestBody::RequestRespond(p) => target_provider(&security.state, &p.target).ok(),
                _ => None,
            };
            // An expired Target still reaches reserve: matching receipts precede stale checks.
            let receipt = if provider == Some(Provider::Codex)
                && let Some(owner) = codex.clone()
            {
                owner.mutate(auth.clone(), request).await?
            } else if let Some(owner) = omp {
                owner.mutate(auth.clone(), request).await?
            } else if let Some(owner) = codex {
                owner.mutate(auth.clone(), request).await?
            } else {
                match security
                    .reserve(auth, request, |_| Err(ErrorCode::NotReady), |_| {})
                    .await?
                {
                    Reservation::Existing(receipt) => receipt,
                    Reservation::Reserved(_) => return Err(ErrorCode::NotReady),
                }
            };
            match method {
                Method::SessionCreate => ResultBody::SessionCreate(SessionCreateResult { receipt }),
                Method::PromptSubmit => ResultBody::PromptSubmit(PromptSubmitResult { receipt }),
                Method::TurnCancel => ResultBody::TurnCancel(TurnCancelResult { receipt }),
                Method::RequestRespond => {
                    ResultBody::RequestRespond(RequestRespondResult { receipt })
                }
                _ => return Err(ErrorCode::InvalidRequest),
            }
        }
        RequestBody::PushRegister(v)=>{
            if v.boot_id!=security.boot{return Err(ErrorCode::StaleHost)}
            let push=security.push.get().ok_or(ErrorCode::NotReady)?;push.validate(&v.endpoint)?;let peer=p.peer_id().clone();
            let (result,changed)=security.storage.call(move|s|s.push_register(&p,v,now_ms())).await.map_err(store_error)?;if changed{push.cancel(&peer)}drop(guard);if changed{push.drain_peer(&peer).await?;}ResultBody::PushRegister(result)
        },
        RequestBody::PushProbeAck(v)=>ResultBody::PushProbeAck(security.storage.call(move|s|s.push_ack(&p,v,now_ms())).await.map_err(store_error)?),
        RequestBody::PushUnregister(v)=>{let peer=p.peer_id().clone();let result=security.storage.call(move|s|s.push_unregister(&p,v,now_ms())).await.map_err(store_error)?;if result.unregistered&&let Some(push)=security.push.get(){push.cancel(&peer)}drop(guard);if result.unregistered&&let Some(push)=security.push.get(){push.drain_peer(&peer).await?;}ResultBody::PushUnregister(result)},
        RequestBody::Hello(_) | RequestBody::PairConfirm(_) | RequestBody::DeviceUnpair(_) => {
            return Err(ErrorCode::InvalidRequest);
        }
    };
    Ok(Some(result))
}

fn target_provider(state: &crate::state::State, target: &Target) -> SecurityResult<Provider> {
    let epoch = state.source_epoch(target)?;
    Ok(state.session_state(target, &epoch)?.summary.provider)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn connection_output_polls_queued_output_while_dispatch_is_pending() {
        let mut dispatch = Box::pin(std::future::pending::<Result<(), ErrorCode>>());
        let mut deadline = Box::pin(tokio::time::sleep(Duration::from_secs(1)));
        let mut output = Box::pin(async { Ok(OutputStep::Continue) });
        let mut extra = Box::pin(std::future::pending::<bool>());
        assert!(matches!(
            poll_dispatch_output(
                dispatch.as_mut(),
                deadline.as_mut(),
                output.as_mut(),
                extra.as_mut(),
            )
            .await,
            DispatchPoll::Output(Ok(OutputStep::Continue))
        ));
    }

    #[tokio::test]
    async fn connection_output_dispatch_completes_while_output_waits_for_guard() {
        let guard = Arc::new(tokio::sync::Mutex::new(()));
        let held = guard.lock().await;
        let output_guard = guard.clone();
        let mut dispatch = Box::pin(async { Ok::<(), ErrorCode>(()) });
        let mut deadline = Box::pin(tokio::time::sleep(Duration::from_secs(1)));
        let mut output = Box::pin(async move {
            let _guard = output_guard.lock().await;
            Ok(OutputStep::Continue)
        });
        let mut extra = Box::pin(std::future::pending::<bool>());
        assert!(matches!(
            poll_dispatch_output(
                dispatch.as_mut(),
                deadline.as_mut(),
                output.as_mut(),
                extra.as_mut(),
            )
            .await,
            DispatchPoll::Complete(Ok(()))
        ));
        drop(held);
    }

    #[tokio::test]
    async fn connection_output_keeps_the_original_dispatch_deadline() {
        let mut dispatch = Box::pin(std::future::pending::<Result<(), ErrorCode>>());
        let mut deadline = Box::pin(tokio::time::sleep(Duration::ZERO));
        let mut output = Box::pin(std::future::pending::<io::Result<OutputStep>>());
        let mut extra = Box::pin(std::future::pending::<bool>());
        assert!(matches!(
            poll_dispatch_output(
                dispatch.as_mut(),
                deadline.as_mut(),
                output.as_mut(),
                extra.as_mut(),
            )
            .await,
            DispatchPoll::Deadline
        ));
    }

    #[test]
    fn connection_output_frame_metadata_is_closed_and_content_free() {
        let error = FrameFailure::write(
            FrameStage::WritePayload,
            iroh::endpoint::WriteError::Stopped(iroh::endpoint::VarInt::from_u32(987654)),
        );
        let failure = FrameFailure::from_error(&error).unwrap();
        assert_eq!(closed_frame_stage(failure.stage), "write_payload");
        assert_eq!(closed_frame_io(failure.io), "connection_reset");
        assert!(!closed_frame_stage(failure.stage).contains("987654"));
        assert!(!closed_frame_io(failure.io).contains("987654"));
    }

    #[test]
    fn frame_failures_keep_io_classification_and_drop_native_details() {
        for stage in [FrameStage::FrameHeader, FrameStage::FramePayload] {
            let error =
                FrameFailure::read(stage, iroh::endpoint::ReadExactError::FinishedEarly(123456));
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert_eq!(
                FrameFailure::from_error(&error),
                Some(FrameFailure {
                    stage,
                    io: Some(FrameIoKind::UnexpectedEof),
                    code: None,
                })
            );
            assert!(!error.to_string().contains("123456"));
        }
        let reset = FrameFailure::write(
            FrameStage::WritePayload,
            iroh::endpoint::WriteError::Stopped(iroh::endpoint::VarInt::from_u32(123456)),
        );
        assert_eq!(reset.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            FrameFailure::from_error(&reset).unwrap().io,
            Some(FrameIoKind::ConnectionReset)
        );
        assert!(!reset.to_string().contains("123456"));
        let invalid_length =
            FrameFailure::protocol(FrameStage::FrameLength, frame_length([0; 4]).unwrap_err());
        assert_eq!(invalid_length.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            FrameFailure::from_error(&invalid_length).unwrap().code,
            Some(ErrorCode::InvalidRequest)
        );
        let cap = FrameFailure::error(FrameStage::FrameCap, None, None);
        assert_eq!(cap.kind(), io::ErrorKind::InvalidData);
        assert_ne!(
            FrameFailure::from_error(&cap).unwrap().stage,
            FrameFailure::from_error(&invalid_length).unwrap().stage
        );
        let deadline =
            FrameFailure::error(FrameStage::WriteDeadline, Some(FrameIoKind::TimedOut), None);
        assert_eq!(deadline.kind(), io::ErrorKind::TimedOut);
        assert!(FrameFailure::from_error(&io::Error::other("private native message")).is_none());
    }
}
