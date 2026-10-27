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
use tokio::time::Instant;
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
    write_frame_until(send, bytes, Instant::now() + Duration::from_secs(5)).await
}
fn frame_deadline() -> io::Error {
    FrameFailure::error(FrameStage::WriteDeadline, Some(FrameIoKind::TimedOut), None)
}
async fn write_frame_until(
    send: &mut SendStream,
    bytes: &[u8],
    original: Instant,
) -> io::Result<()> {
    let deadline = original.min(Instant::now() + Duration::from_secs(5));
    if Instant::now() >= deadline {
        return Err(frame_deadline());
    }
    frame_length(
        (u32::try_from(bytes.len())
            .map_err(|_| FrameFailure::error(FrameStage::WriteLength, None, None))?)
        .to_be_bytes(),
    )
    .map_err(|error| FrameFailure::protocol(FrameStage::WriteLength, error))?;
    let mut write = std::pin::pin!(async {
        if Instant::now() >= deadline {
            return Err(frame_deadline());
        }
        send.write_all(&(bytes.len() as u32).to_be_bytes())
            .await
            .map_err(|e| FrameFailure::write(FrameStage::WriteHeader, e))?;
        if Instant::now() >= deadline {
            return Err(frame_deadline());
        }
        send.write_all(bytes)
            .await
            .map_err(|e| FrameFailure::write(FrameStage::WritePayload, e))?;
        if Instant::now() >= deadline {
            return Err(frame_deadline());
        }
        Ok(())
    });
    // Check before EVERY resumed write poll, including partial header/payload progress.
    let guarded = std::future::poll_fn(|cx| {
        if Instant::now() >= deadline {
            return std::task::Poll::Ready(Err(frame_deadline()));
        }
        write.as_mut().poll(cx)
    });
    tokio::select! {biased; _=tokio::time::sleep_until(deadline)=>Err(frame_deadline()),result=guarded=>result}
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

fn closed_manager_io(kind: Option<io::ErrorKind>) -> &'static str {
    match kind {
        None => "none",
        Some(io::ErrorKind::NotFound) => "not_found",
        Some(io::ErrorKind::PermissionDenied) => "permission_denied",
        Some(io::ErrorKind::ConnectionRefused) => "connection_refused",
        Some(io::ErrorKind::ConnectionReset) => "connection_reset",
        Some(io::ErrorKind::ConnectionAborted) => "connection_aborted",
        Some(io::ErrorKind::NotConnected) => "not_connected",
        Some(io::ErrorKind::AddrInUse) => "addr_in_use",
        Some(io::ErrorKind::AddrNotAvailable) => "addr_not_available",
        Some(io::ErrorKind::BrokenPipe) => "broken_pipe",
        Some(io::ErrorKind::AlreadyExists) => "already_exists",
        Some(io::ErrorKind::WouldBlock) => "would_block",
        Some(io::ErrorKind::InvalidInput) => "invalid_input",
        Some(io::ErrorKind::InvalidData) => "invalid_data",
        Some(io::ErrorKind::TimedOut) => "timed_out",
        Some(io::ErrorKind::Interrupted) => "interrupted",
        Some(io::ErrorKind::UnexpectedEof) => "unexpected_eof",
        Some(io::ErrorKind::Unsupported) => "unsupported",
        Some(io::ErrorKind::OutOfMemory) => "out_of_memory",
        Some(io::ErrorKind::WriteZero) => "write_zero",
        Some(_) => "other",
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
    let (bytes, closed) = security
        .state
        .pop_tagged_output(&auth.connection_id)
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
        .map_err(|code| {
            if code == ErrorCode::Timeout {
                frame_deadline()
            } else {
                invalid()
            }
        })?;
    if let Some(output) = bytes {
        let bytes = output.bytes;
        let grid = output.grid;
        if let Some((lease,_,deadline))=&grid
            && let Err(code)=lease.current(*deadline).await
        {
            security.state.grid_failed(&auth.connection_id,&lease.id);
            return Err(if code==ErrorCode::Timeout{frame_deadline()}else{invalid()});
        }
        let guarded = if let Some((lease, _, deadline)) = &grid {
            lease.authorize(security, auth, *deadline).await
        } else {
            security.guard(auth).await
        };
        let _guard = match guarded {
            Ok(guard) => guard,
            Err(code) => {
                if let Some((lease, _, _)) = &grid {
                    security.state.grid_failed(&auth.connection_id, &lease.id);
                }
                if let Some(reporter) = reporter {
                    reporter.transport_failure(
                        &auth.connection_id,
                        "output_guard",
                        Some(code),
                        "none",
                        "none",
                    );
                }
                report_codex_transport(
                    codex_reporter,
                    &auth.connection_id,
                    "output_guard",
                    Some(code),
                    "none",
                );
                return Err(if code == ErrorCode::Timeout {
                    frame_deadline()
                } else {
                    invalid()
                });
            }
        };
        if let Some((lease, _, _)) = &grid
            && let Err(code) = lease.check_current()
        {
            security.state.grid_failed(&auth.connection_id, &lease.id);
            return Err(if code == ErrorCode::Timeout {
                frame_deadline()
            } else {
                invalid()
            });
        }
        // Parse only delivery metadata. No frame/control payload is logged or retained.
        let events = if grid.is_some() { Vec::new() } else { delivered_notifications(&bytes).inspect_err(|error| {
            report_transport_frame(reporter, &auth.connection_id, "delivery_metadata", error);
            report_codex_transport_frame(
                codex_reporter,
                &auth.connection_id,
                "delivery_metadata",
                error,
            );
        })? };
        let deadline=grid.as_ref().map_or_else(||Instant::now()+Duration::from_secs(5),|(_,_,deadline)|*deadline);
        let written=write_frame_until(send,&bytes,deadline);
        let result=if let Some((lease,_,_))=&grid {
            tokio::select! {biased; _=lease.cancelled()=>Err(invalid()),result=written=>result}
        } else {written.await};
        if let Err(error)=result {
            if let Some((lease,_,_))=&grid {security.state.grid_failed(&auth.connection_id,&lease.id);}
            report_transport_frame(reporter,&auth.connection_id,"output_write",&error);
            report_codex_transport_frame(codex_reporter,&auth.connection_id,"output_write",&error);
            return Err(error);
        }
        if let Some((_, completion, _)) = grid {
            let _ = completion.send(());
        }
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
        // Preserve ordinary idle authorization checks; a closed grid transport needs no private reply.
        if let Err(code) = security.guard(auth).await {
            if let Some(reporter) = reporter {
                reporter.transport_failure(
                    &auth.connection_id,
                    "output_guard",
                    Some(code),
                    "none",
                    "none",
                );
            }
            report_codex_transport(
                codex_reporter,
                &auth.connection_id,
                "output_guard",
                Some(code),
                "none",
            );
            return Err(invalid());
        }
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
    let push=crate::push::Publisher::new(topology.as_ref().map(|t|t.config.push_allowed_origins.clone()).unwrap_or_else(crate::config::default_push_allowed_origins));
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
    security: &Arc<Security>,
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
        let mut codex_manager_failure = None;
        let backends = security
            .backends
            .iter()
            .map(|backend| {
                if backend.provider == Provider::Omp {
                    omp.as_ref()
                        .map_or_else(|| backend.clone(), |owner| owner.backend())
                } else {
                    codex.as_ref().map_or_else(|| backend.clone(), |owner| {
                        owner.backend_observed(hello_capture.as_ref().map(|_| &mut codex_manager_failure))
                    })
                }
            })
            .collect();
        if let (Some(capture), Some(detail)) = (&hello_capture, codex_manager_failure) {
            capture.report("codex_manager_hello", detail.guard.label(), Some(ErrorCode::NotReady), closed_manager_io(detail.io));
        }
        let hello = ResultBody::Hello(HelloResult {
            host_id: security.host.clone(),
            device_id: auth.proof.peer_id().clone(),
            boot_id: security.boot.clone(),
            protocol: ProtocolVersion(2),
            backends,
            features: HelloFeatures { agent_ansi: topology.is_some(), shell_history:topology.is_some() },
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
                    let dispatch_deadline=Instant::now()+timeout;
                    let mut dispatch = Some(Box::pin(dispatch(
                        security,
                        &auth,
                        topology.clone(),
                        omp.clone(),
                        codex.clone(),
                        request,
                        &mut views,
                        dispatch_deadline,
                    )));
                    let mut deadline = Box::pin(tokio::time::sleep_until(dispatch_deadline));
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
async fn authorize_skill_document(
    auth:&Arc<Authorized>,publication:&Arc<crate::backends::SkillsPublication>,document:&Uuid,
    omp:Option<&Arc<crate::backends::omp::Runtime>>,codex:Option<&Arc<crate::backends::codex::Runtime>>,
)->SecurityResult<()> {
    match publication.origin.context.provider {
        Provider::Omp=>omp.ok_or(ErrorCode::NotReady)?.authorize_skill_document(auth,publication,document).await,
        Provider::Codex=>codex.ok_or(ErrorCode::NotReady)?.authorize_skill_document(auth,publication,document).await,
    }
}
async fn provider_project_context(
    provider:Provider,target:&Target,omp:Option<&Arc<crate::backends::omp::Runtime>>,codex:Option<&Arc<crate::backends::codex::Runtime>>,
    job:Arc<crate::herdr::artifacts::ReadJob>,
)->SecurityResult<crate::project_read::ProjectReadContext> {
    job.current()?;let target=target.clone();let owner=job.clone();
    let worker=match provider {
        Provider::Omp=>{let runtime=omp.cloned().ok_or(ErrorCode::NotReady)?;tokio::task::spawn_blocking(move||{owner.current()?;runtime.project_context(&target)})},
        Provider::Codex=>{let runtime=codex.cloned().ok_or(ErrorCode::NotReady)?;tokio::task::spawn_blocking(move||{owner.current()?;runtime.project_context(&target)})},
    };
    tokio::time::timeout_at(Instant::from_std(job.deadline),worker).await.map_err(|_|ErrorCode::Timeout)?.map_err(|_|ErrorCode::NotReady)?
}

async fn dispatch(
    // Routing reads current reducer authority, never desktop focus/native ID guesses.
    security: &Arc<Security>,
    auth: &Arc<Authorized>,
    topology: Option<Arc<crate::herdr::Topology>>,
    omp: Option<Arc<crate::backends::omp::Runtime>>,
    codex: Option<Arc<crate::backends::codex::Runtime>>,
    r: Request,
    views: &mut Viewing,
    deadline: Instant,
) -> SecurityResult<Option<ResultBody>> {
    let grid = matches!(
        &r.body,
        RequestBody::TerminalGridSubscribe(_) | RequestBody::ShellGridSubscribe(_)
    );
    let guard = if grid {
        crate::herdr::grid::before(deadline)?;
        let guard = tokio::time::timeout_at(deadline, security.guard(auth))
            .await
            .map_err(|_| ErrorCode::Timeout)??;
        crate::herdr::grid::before(deadline)?;
        guard
    } else {
        security.guard(auth).await?
    };
    let p = auth.proof.clone();
    let result = match r.body {
        RequestBody::CreationDirectories(params)=>{
            let topology=topology.ok_or(ErrorCode::NotReady)?;
            drop(guard);
            let prepared=topology.creation_directories(auth,&params).await?;
            let _guard=security.guard(auth).await?;
            topology.enqueue_creation_directories(auth,r.id,prepared).await?;
            return Ok(None);
        }
        body @ (RequestBody::ArtifactsList(_)|RequestBody::ArtifactsOpen(_)|RequestBody::ArtifactsRead(_)|RequestBody::ArtifactsClose(_))=>{
            let topology=topology.ok_or(ErrorCode::NotReady)?;
            let job=topology.reserve_project_read(auth,false,deadline.into_std())?;
            drop(guard);job.current()?;
            let initial=if let RequestBody::ArtifactsList(params)=&body {
                if params.parent.is_none()&&params.cursor.is_none() {
                    Some(match &params.source {
                        ArtifactSource::Session{target}=>provider_project_context(target_provider(&security.state,target)?,target,omp.as_ref(),codex.as_ref(),job.clone()).await?,
                        ArtifactSource::Shell{target}=>topology.shell_project_context(target,job.clone()).await?,
                    })
                }else{None}
            }else{None};
            let (context,result,mut pending)=topology.artifact_request(auth,&body,initial,job).await?;
            let _guard=security.guard(auth).await?;
            topology.enqueue_artifact(&auth.connection_id,r.id,&context,result,&pending)?;
            pending.published();return Ok(None);
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
                    let outcome=match topology {
                        Some(topology)=>topology.workspace_rename_once(security,auth,&r.id,&v).await,
                        None=>Err(ErrorCode::NotReady),
                    };
                    let proof=auth.proof.clone();let action=r.id.clone();
                    let receipt=security.storage.call(move|store|store.status(&proof,&action,now_ms()))
                        .await.map_err(store_error)?.ok_or(ErrorCode::NotReady)?;
                    if receipt.state==ReceiptState::Reserved {
                        workspace_rename_no_effect(security,auth,&r.id,outcome.err().unwrap_or(ErrorCode::NotReady)).await?
                    } else {receipt}
                }
            };
            ResultBody::WorkspaceRename(WorkspaceRenameResult { receipt })
        }
        body @ (RequestBody::ShellGridSubscribe(_) | RequestBody::TerminalGridSubscribe(_)) => {
            let topology = topology.ok_or(ErrorCode::NotReady)?;
            drop(guard);
            topology
                .grid_open(security.clone(), auth.clone(), r.id, body, deadline)
                .await?;
            return Ok(None);
        }
        RequestBody::TerminalCreate(params)=>{
            let request=Request{id:r.id.clone(),body:RequestBody::TerminalCreate(params)};drop(guard);
            let receipt=match topology {
                Some(topology)=>topology.terminal_create(security,auth,request,deadline).await?,
                None=>{let (_,existing)=security.preflight_request(auth,request).await?;existing.ok_or(ErrorCode::NotReady)?},
            };
            ResultBody::TerminalCreate(TerminalCreateResult{receipt})
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
                            let client = crate::herdr::Client::connect_source(&context.source)
                                .await
                                .map_err(|_| ErrorCode::NotReady)?;
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
        RequestBody::ShellInput(params) => {
            let request=Request {id:r.id.clone(),body:RequestBody::ShellInput(params.clone())};
            drop(guard);
            let (request,existing)=security.preflight_request(auth,request).await?;
            let reservation=if let Some(receipt)=existing {Reservation::Existing(receipt)}else{
                let owner=topology.as_ref().ok_or(ErrorCode::NotReady)?;
                owner.await_shell_input(&security.state,&auth.connection_id,&params,deadline).await?;
                let validator=owner.clone();let state=security.state.clone();let connection=auth.connection_id.clone();
                security.reserve(auth,request,move|request|{
                    let RequestBody::ShellInput(params)=&request.body else{return Err(ErrorCode::InvalidRequest)};
                    validator.validate_shell_input(&state,&connection,params).map(|_|())
                },|_|{}).await?
            };
            let receipt=match reservation {
                Reservation::Existing(receipt)=>receipt,
                Reservation::Reserved(_)=>{
                    let outcome=match topology {Some(topology)=>topology.shell_input_once(security,auth,&r.id,&params,deadline).await,None=>Err(ErrorCode::NotReady)};
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
        RequestBody::ShellResize(params)=>{
            let request=Request{id:r.id.clone(),body:RequestBody::ShellResize(params.clone())};
            let validator=topology.clone();let state=security.state.clone();let connection=auth.connection_id.clone();
            drop(guard);
            let reservation=security.reserve(auth,request,move|request|{
                let RequestBody::ShellResize(params)=&request.body else{return Err(ErrorCode::InvalidRequest);};
                validator.as_ref().ok_or(ErrorCode::NotReady)?.validate_shell_resize(&state,&connection,params).map(|_|())
            },|_|{}).await?;
            let receipt=match reservation {
                Reservation::Existing(receipt)=>receipt,
                Reservation::Reserved(_)=>{
                    let outcome=match topology {Some(topology)=>topology.shell_resize_once(security,auth,&r.id,&params,deadline).await,None=>Err(ErrorCode::NotReady)};
                    let proof=auth.proof.clone();let action=r.id.clone();
                    let mut receipt=security.storage.call(move|store|store.status(&proof,&action,now_ms())).await.map_err(store_error)?.ok_or(ErrorCode::NotReady)?;
                    if receipt.state==ReceiptState::Reserved {
                        let proof=auth.proof.clone();let action=r.id.clone();let code=outcome.err().unwrap_or(ErrorCode::NotReady);
                        receipt=security.storage.call(move|store|store.transition(&proof,&action,crate::storage::ReceiptTransition {
                            state:ReceiptState::Failed,proof:crate::storage::TransitionProof::DefinitiveNoEffect(Id("resize boundary not entered".into())),result:None,error:Some(wire_error(code)),at:now_ms(),
                        })).await.map_err(store_error)?;
                    }
                    receipt
                }
            };
            ResultBody::ShellResize(ShellResizeResult{receipt})
        }
        RequestBody::ShellScroll(params)=>{
            let request=Request{id:r.id.clone(),body:RequestBody::ShellScroll(params.clone())};
            let validator=topology.clone();let state=security.state.clone();let connection=auth.connection_id.clone();
            drop(guard);
            let reservation=security.reserve(auth,request,move|request|{
                let RequestBody::ShellScroll(params)=&request.body else{return Err(ErrorCode::InvalidRequest);};
                validator.as_ref().ok_or(ErrorCode::NotReady)?.validate_shell_scroll(&state,&connection,params).map(|_|())
            },|_|{}).await?;
            let receipt=match reservation {
                Reservation::Existing(receipt)=>receipt,
                Reservation::Reserved(_)=>{
                    let outcome=match topology {Some(topology)=>topology.shell_scroll_once(security,auth,&r.id,&params,deadline).await,None=>Err(ErrorCode::NotReady)};
                    let proof=auth.proof.clone();let action=r.id.clone();
                    let mut receipt=security.storage.call(move|store|store.status(&proof,&action,now_ms())).await.map_err(store_error)?.ok_or(ErrorCode::NotReady)?;
                    if receipt.state==ReceiptState::Reserved {
                        let proof=auth.proof.clone();let action=r.id.clone();let code=outcome.err().unwrap_or(ErrorCode::NotReady);
                        receipt=security.storage.call(move|store|store.transition(&proof,&action,crate::storage::ReceiptTransition {
                            state:ReceiptState::Failed,proof:crate::storage::TransitionProof::DefinitiveNoEffect(Id("scroll boundary not entered".into())),result:None,error:Some(wire_error(code)),at:now_ms(),
                        })).await.map_err(store_error)?;
                    }
                    receipt
                }
            };
            ResultBody::ShellScroll(ShellScrollResult{receipt})
        }
        RequestBody::WorktreesList(v) => {
            let topology=topology.ok_or(ErrorCode::NotReady)?;
            let provider=target_provider(&security.state,&v.target)?;
            let job=topology.reserve_project_read(auth,true,deadline.into_std())?;drop(guard);job.current()?;
            let context=provider_project_context(provider,&v.target,omp.as_ref(),codex.as_ref(),job.clone()).await?;
            let prepared=crate::project_read::git::observe(context,job,false).await?;
            let _guard=security.guard(auth).await?;
            prepared.enqueue(&security.state,&auth.connection_id,r.id)?;return Ok(None);
        }

        RequestBody::SkillsList(v)=>{
            let provider=target_provider(&security.state,&v.target)?;drop(guard);
            match provider {
                Provider::Codex=>{let owner=codex.ok_or(ErrorCode::NotReady)?;let publication=owner.skills_list(auth,&v.target).await?;let _guard=security.guard(auth).await?;owner.enqueue_skills_list(&auth.connection_id,r.id,publication)?;},
                Provider::Omp=>{let owner=omp.ok_or(ErrorCode::NotReady)?;let publication=owner.skills_list(auth,&v.target).await?;let _guard=security.guard(auth).await?;owner.enqueue_skills_list(&auth.connection_id,r.id,publication)?;},
            }return Ok(None);
        }
        RequestBody::SessionControlsRead(params)=>{
            let provider=target_provider(&security.state,&params.target)?;drop(guard);
            match provider {
                Provider::Omp=>{let owner=omp.ok_or(ErrorCode::NotReady)?;let publication=owner.controls_read(auth,&params.target).await?;let _guard=security.guard(auth).await?;owner.enqueue_controls(&auth.connection_id,r.id,publication)?;},
                Provider::Codex=>{let owner=codex.ok_or(ErrorCode::NotReady)?;let publication=owner.controls_read(auth,&params.target).await?;let _guard=security.guard(auth).await?;owner.enqueue_controls(&auth.connection_id,r.id,publication)?;},
            }return Ok(None);
        }
        RequestBody::SessionQueueList(params)=>{
            if target_provider(&security.state,&params.target)?!=Provider::Codex {return Err(ErrorCode::NotReady)}
            let owner=codex.ok_or(ErrorCode::NotReady)?;drop(guard);
            let read=owner.queue_list(auth,&params).await?;let _guard=security.guard(auth).await?;
            owner.enqueue_queue(&auth.connection_id,r.id,read)?;return Ok(None);
        }
        RequestBody::SkillsPreviewOpen(params)=>{
            let documents=topology.as_ref().ok_or(ErrorCode::NotReady)?.skill_documents.clone();
            let publication=security.state.skills_publication(auth,&params.target,&params.catalogue_generation)?;
            drop(guard);
            authorize_skill_document(auth,&publication,&params.document_ref,omp.as_ref(),codex.as_ref()).await?;
            let (result,mut pending)=documents.open(auth,publication.clone(),&params).await?;
            authorize_skill_document(auth,&publication,&params.document_ref,omp.as_ref(),codex.as_ref()).await?;
            let _guard=security.guard(auth).await?;let mut result=Some(ResultBody::SkillsPreviewOpen(result));
            documents.publish(&pending,||publication.origin.context.with_current(||security.state.skills_preview_response(&auth.connection_id,r.id.clone(),&publication,result.take().ok_or(ErrorCode::StaleSession)?)))?;
            pending.published();return Ok(None);
        }
        RequestBody::SkillsPreviewRead(params)=>{
            let documents=topology.as_ref().ok_or(ErrorCode::NotReady)?.skill_documents.clone();
            let (publication,document)=documents.authority(auth,&params)?;
            drop(guard);
            authorize_skill_document(auth,&publication,&document,omp.as_ref(),codex.as_ref()).await?;
            let (result,mut pending)=documents.read(auth,&params).await?;
            authorize_skill_document(auth,&publication,&document,omp.as_ref(),codex.as_ref()).await?;
            let _guard=security.guard(auth).await?;let mut result=Some(ResultBody::SkillsPreviewRead(result));
            documents.publish(&pending,||publication.origin.context.with_current(||security.state.skills_preview_response(&auth.connection_id,r.id.clone(),&publication,result.take().ok_or(ErrorCode::StaleSession)?)))?;
            pending.published();return Ok(None);
        }
        RequestBody::SkillsPreviewClose(params)=>{
            let documents=&topology.as_ref().ok_or(ErrorCode::NotReady)?.skill_documents;
            let result=documents.close(auth.proof.peer_id(),&params);
            ResultBody::SkillsPreviewClose(result)
        }
        RequestBody::ChangesRead(v) => {
            let topology=topology.ok_or(ErrorCode::NotReady)?;
            let provider=target_provider(&security.state,&v.target)?;
            let job=topology.reserve_project_read(auth,true,deadline.into_std())?;drop(guard);job.current()?;
            let context=provider_project_context(provider,&v.target,omp.as_ref(),codex.as_ref(),job.clone()).await?;
            let prepared=crate::project_read::git::observe(context,job,true).await?;
            let _guard=security.guard(auth).await?;
            prepared.enqueue(&security.state,&auth.connection_id,r.id)?;return Ok(None);
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
        | RequestBody::SessionControlsApply(_)
        | RequestBody::SessionQueueUpdate(_)
        | RequestBody::SessionQueueDelete(_)
        | RequestBody::SessionQueueReorder(_)
        | RequestBody::SessionQueueStart(_)
        | RequestBody::PromptSubmit(_)
        | RequestBody::TurnCancel(_)
        | RequestBody::RequestRespond(_)) => {
            let method = body.method();
            let codex_only=matches!(method,Method::SessionQueueUpdate|Method::SessionQueueDelete|Method::SessionQueueReorder|Method::SessionQueueStart);
            let request = Request { id: r.id, body };
            drop(guard);
            let provider = match &request.body {
                RequestBody::SessionCreate(p) => Some(p.provider),
                RequestBody::SessionControlsApply(p)=>target_provider(&security.state,&p.target).ok(),
                RequestBody::SessionQueueUpdate(p)=>target_provider(&security.state,&p.target).ok(),
                RequestBody::SessionQueueDelete(p)=>target_provider(&security.state,&p.target).ok(),
                RequestBody::SessionQueueReorder(p)=>target_provider(&security.state,&p.target).ok(),
                RequestBody::SessionQueueStart(p)=>target_provider(&security.state,&p.target).ok(),
                RequestBody::PromptSubmit(p) => target_provider(&security.state, &p.target).ok(),
                RequestBody::TurnCancel(p) => target_provider(&security.state, &p.target).ok(),
                RequestBody::RequestRespond(p) => target_provider(&security.state, &p.target).ok(),
                _ => None,
            };
            // An expired Target still reaches reserve: matching receipts precede stale checks.
            let receipt = if codex_only&&codex.is_none() {
                match security.reserve(auth,request,|_|Err(ErrorCode::NotReady),|_|{}).await? {
                    Reservation::Existing(receipt)=>receipt,
                    Reservation::Reserved(_)=>return Err(ErrorCode::NotReady),
                }
            } else if (codex_only||provider==Some(Provider::Codex))
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
                Method::SessionControlsApply=>ResultBody::SessionControlsApply(SessionControlsApplyResult {receipt}),
                Method::SessionQueueUpdate=>ResultBody::SessionQueueUpdate(SessionQueueResult {receipt}),
                Method::SessionQueueDelete=>ResultBody::SessionQueueDelete(SessionQueueResult {receipt}),
                Method::SessionQueueReorder=>ResultBody::SessionQueueReorder(SessionQueueResult {receipt}),
                Method::SessionQueueStart=>ResultBody::SessionQueueStart(SessionQueueResult {receipt}),
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
    #[test]
    fn hello_manager_observation_shares_the_existing_eight_line_budget() {
        let reporter = HelloCaptureReporter::new();
        let selected = reporter.select().unwrap();
        assert!(reporter.select().is_none());
        for phase in ["admission","stream_accept","hello_frame","hello_authorization","security_guard","backend_capabilities"] {
            selected.report(phase,"entered",None,"none");
        }
        selected.report("codex_manager_hello","process_evidence",Some(ErrorCode::NotReady),closed_manager_io(Some(io::ErrorKind::PermissionDenied)));
        selected.report("codex_manager_hello","process_evidence",Some(ErrorCode::NotReady),closed_manager_io(Some(io::ErrorKind::PermissionDenied)));
        selected.report("response_write","entered",None,"none");
        selected.report("complete","completed",None,"none");
        let state = reporter.0.lock().unwrap();
        assert_eq!(state.lines,8);
        assert!(state.observations.contains(&("codex_manager_hello","process_evidence")));
        assert!(!state.observations.contains(&("complete","completed")));
    }
    struct WriterFixture {
        host: Endpoint,
        phone: Endpoint,
        server: Connection,
        client: Connection,
        security: Arc<Security>,
        auth: Arc<Authorized>,
        send: SendStream,
        recv: RecvStream,
    }
    impl WriterFixture {
        async fn new() -> Self {
            use crate::storage::{PairedDevice, Storage, StorageOwner};
            let limits = iroh::endpoint::QuicTransportConfig::builder()
                .stream_receive_window(iroh::endpoint::VarInt::from_u32(1024))
                .receive_window(iroh::endpoint::VarInt::from_u32(1024))
                .send_window(1024)
                .build();
            let host = crate::endpoint_builder(iroh::SecretKey::from_bytes(&[61; 32]))
                .relay_mode(iroh::RelayMode::Disabled)
                .clear_address_lookup()
                .transport_config(limits.clone())
                .bind()
                .await
                .unwrap();
            let phone = crate::endpoint_builder(iroh::SecretKey::from_bytes(&[62; 32]))
                .relay_mode(iroh::RelayMode::Disabled)
                .clear_address_lookup()
                .transport_config(limits)
                .bind()
                .await
                .unwrap();
            let mut storage = Storage::memory([61; 32], now_ms()).unwrap();
            let device = PairedDevice {
                device_id: Uuid::random(),
                endpoint_id: EndpointId(phone.id().to_string()),
                device_name: Label("Grid writer".into()),
                paired_at: Uint(now_ms()),
                last_seen_at: None,
            };
            storage.pair_device(&device).unwrap();
            let boot = storage.boot_id.clone();
            let security = Security::new(
                StorageOwner::new(storage).unwrap(),
                Arc::new(crate::state::State::default()),
                EndpointId(host.id().to_string()),
                boot,
                vec![],
                None,
            );
            let endpoint = host.clone();
            let accept =
                tokio::spawn(async move { endpoint.accept().await.unwrap().await.unwrap() });
            let client = phone.connect(host.addr(), ALPN).await.unwrap();
            let server = accept.await.unwrap();
            let auth = Arc::new(
                security
                    .hello(
                        EndpointId(server.remote_id().to_string()),
                        HelloParams {
                            app_version: "grid-test".into(),
                            device_name: Label("Grid writer".into()),
                        },
                        server.clone(),
                    )
                    .await
                    .unwrap(),
            );
            let (mut probe, recv) = client.open_bi().await.unwrap();
            probe.write_all(&[0]).await.unwrap();
            let (send, mut incoming) = server.accept_bi().await.unwrap();
            let mut byte = [0];
            incoming.read_exact(&mut byte).await.unwrap();
            Self {
                host,
                phone,
                server,
                client,
                security,
                auth,
                send,
                recv,
            }
        }
        async fn close(self) {
            self.server.close(0u32.into(), b"test complete");
            self.client.close(0u32.into(), b"test complete");
            self.security.disconnect(&self.auth);
            crate::close_endpoint(&self.host).await.unwrap();
            crate::close_endpoint(&self.phone).await.unwrap();
        }
    }
    #[tokio::test]
    async fn stock_endpoint_begin_resize_confirmation_receipts_and_lost_result_never_replay() {
        use crate::herdr::endpoint::{fixture as native,Rect};
        use std::{os::unix::fs::PermissionsExt,sync::atomic::{AtomicBool,AtomicUsize,Ordering}};
        use tokio::io::{AsyncReadExt,AsyncWriteExt};
        async fn native_frame(stream:&mut tokio::net::UnixStream)->Vec<u8>{
            let n=stream.read_u32_le().await.unwrap() as usize;assert!(n>0&&n<=2*1024*1024);
            let mut bytes=vec![0;n];stream.read_exact(&mut bytes).await.unwrap();bytes
        }
        async fn drain(wire:&mut WriterFixture)->Vec<Envelope> {
            let done=AtomicBool::new(false);let security=wire.security.clone();let auth=wire.auth.clone();
            let send=&mut wire.send;let recv=&mut wire.recv;
            let writing=async {let mut response=false;while !done.load(Ordering::Acquire){
                assert!(!matches!(output_step(send,&security,&auth,false,None,None,&mut response).await.unwrap(),OutputStep::Closed));
                tokio::task::yield_now().await;
            }};
            let reading=async {let mut frames=Vec::new();for _ in 0..3{frames.push(decode_payload(&read_frame(recv,FRAME_MAX).await.unwrap(),Some(Method::ShellGridSubscribe)).unwrap());}done.store(true,Ordering::Release);frames};
            let ((),frames)=tokio::time::timeout(Duration::from_secs(15),async{tokio::join!(writing,reading)}).await.unwrap();frames
        }
        for (history,lose_second) in [(false,false),(false,true),(true,false),(true,true)] {
            let mut wire=WriterFixture::new().await;
            let fixture=crate::herdr::grid::queue_fixture(wire.security.state.clone(),Uuid::random()).await;
            let topology=fixture.topology();
            let ResultBody::ShellGridSubscribe(description)=fixture.opening() else{panic!("shell fixture");};
            let path=fixture.source.socket_path.with_file_name("native-client.sock");
            let listener=tokio::net::UnixListener::bind(&path).unwrap();
            std::fs::set_permissions(&path,std::fs::Permissions::from_mode(0o600)).unwrap();
            {let accept=listener.accept();tokio::pin!(accept);assert!(futures_util::poll!(&mut accept).is_pending(),"metadata alone cannot activate a native frontend");}
            let resizes=Arc::new(AtomicUsize::new(0));let native_resizes=resizes.clone();
            let deactivated=Arc::new(AtomicBool::new(false));let native_deactivated=deactivated.clone();
            let native_owner=tokio::spawn(async move {
                let (mut socket,_)=listener.accept().await.unwrap();
                let (tag,hello)=native::read_client(&native_frame(&mut socket).await);
                assert_eq!(tag,20);assert_eq!(hello["surface_active"],false);assert_eq!(hello["surface_size"],serde_json::json!({"cols":1,"rows":1}));
                socket.write_all(&native::native_control("endpoint.welcome.v1",serde_json::json!({"generation":1,"server_version":"fixture","snapshot_codec":"shell.snapshot.v1","surface_codec":"shell.surface.v1","input_codec":"shell.input.semantic.v1","blob_codec":"shell.blob.v1","methods":["client_shell.surface.set","workspace.focus","tab.focus","pane.focus"]}))).await.unwrap();
                socket.write_all(&native::native_control("shell.snapshot.v1",native::view(1))).await.unwrap();
                let mut surface_revision=0;
                loop {
                    let (tag,request)=native::read_client(&native_frame(&mut socket).await);
                    if tag==12 {
                        let count=native_resizes.fetch_add(1,Ordering::AcqRel)+1;
                        if lose_second&&count==2{socket.write_all(&[3,0,0,0,13]).await.unwrap();break;}
                        let columns=request["columns"].as_u64().unwrap() as u16;assert_eq!(request["rows"],1);
                        surface_revision+=1;let cells=vec![("x",0,0,0,false);usize::from(columns)];
                        socket.write_all(&native::packet(&native::surface(columns,surface_revision,2,&cells,Rect{x:0,y:0,width:columns,height:1},false))).await.unwrap();continue;
                    }
                    assert_eq!(tag,15);let id=request["id"].as_str().unwrap();
                    let result=match request["method"].as_str().unwrap() {
                        "client_shell.surface.set"=>{
                            if request["params"]["active"]==false{native_deactivated.store(true,Ordering::Release);break;}
                            serde_json::json!({"type":"client_shell_surface_set","active":true,"projection_revision":2})
                        },
                        "workspace.focus"=>serde_json::json!({"type":"workspace_info","workspace":{"workspace_id":"w"}}),
                        "tab.focus"=>serde_json::json!({"type":"tab_info","tab":{"workspace_id":"w","tab_id":"t"}}),
                        "pane.focus"=>serde_json::json!({"type":"pane_info","pane":{"pane_id":"p","terminal_id":"n","workspace_id":"w","tab_id":"t"}}),
                        _=>panic!("no input, alternate native method or provider action is allowed"),
                    };
                    socket.write_all(&native::response(id,result)).await.unwrap();
                    if surface_revision==0 {
                        surface_revision=1;socket.write_all(&native::native_control("shell.snapshot.v1",native::view(2))).await.unwrap();
                        socket.write_all(&native::packet(&native::surface_with_scroll(1,1,2,&[("x",0,0,0,false)],Rect{x:0,y:0,width:1,height:1},false,Some([0,20,1]),None,false))).await.unwrap();
                    }
                }
            });
            topology.grid_open(wire.security.clone(),wire.auth.clone(),Uuid::random(),RequestBody::ShellGridSubscribe(ShellGridSubscribeParams { target: description.target.clone(), history }),Instant::now()+Duration::from_secs(15)).await.unwrap();
            let frames=drain(&mut wire).await;
            let Envelope::Response(Response{outcome:Ok(ResultBody::ShellGridSubscribe(opened)),..})=&frames[0] else{panic!("actual owned opening descriptor");};
            assert_eq!(opened.frame.scroll.as_ref().map(|metrics|metrics.offset_from_bottom.0),history.then_some(0));
            let subscription=opened.subscription.clone();
            let request=Request{id:Uuid::random(),body:RequestBody::ShellResize(ShellResizeParams{boot_id:wire.security.boot.clone(),target:description.target.clone(),subscription:subscription.clone(),columns:Uint(2),rows:Uint(1)})};
            let mut views=Viewing::default();
            let result=dispatch(&wire.security,&wire.auth,Some(topology.clone()),None,None,request.clone(),&mut views,Instant::now()+Duration::from_secs(15)).await.unwrap().unwrap();
            let ResultBody::ShellResize(result)=result else{panic!("resize receipt");};assert_eq!(result.receipt.state,ReceiptState::Succeeded);
            let repeated=dispatch(&wire.security,&wire.auth,Some(topology.clone()),None,None,request.clone(),&mut views,Instant::now()+Duration::from_secs(15)).await.unwrap().unwrap();
            assert_eq!(repeated,ResultBody::ShellResize(result));assert_eq!(resizes.load(Ordering::Acquire),1);
            drain(&mut wire).await;
            if lose_second {
                let mut lost=request;lost.id=Uuid::random();if let RequestBody::ShellResize(params)=&mut lost.body{params.columns=Uint(3);}
                let result=dispatch(&wire.security,&wire.auth,Some(topology.clone()),None,None,lost.clone(),&mut views,Instant::now()+Duration::from_secs(15)).await.unwrap().unwrap();
                let ResultBody::ShellResize(receipt)=&result else{panic!("uncertain resize");};assert_eq!(receipt.receipt.state,ReceiptState::Uncertain);
                assert_eq!(dispatch(&wire.security,&wire.auth,Some(topology),None,None,lost,&mut views,Instant::now()+Duration::from_secs(15)).await.unwrap().unwrap(),result);
                assert_eq!(resizes.load(Ordering::Acquire),2);
            }else{wire.security.state.unsubscribe(&wire.auth.connection_id,&subscription).unwrap();}
            tokio::time::timeout(Duration::from_secs(5),native_owner).await.unwrap().unwrap();
            assert_eq!(deactivated.load(Ordering::Acquire),!lose_second);
            wire.close().await;
        }
    }
    #[tokio::test]
    async fn real_tagged_opening_and_commit_cannot_cross_an_expired_authorization_wait() {
        for opening in [true, false] {
            let mut wire = WriterFixture::new().await;
            let fixture =
                crate::herdr::grid::queue_fixture(wire.security.state.clone(), Uuid::random())
                    .await;
            let connection = &wire.auth.connection_id;
            let mut bytes = 0;
            let mut response_pending = false;
            let initial = Instant::now() + Duration::from_secs(2);
            wire.security
                .state
                .reserve_grid(connection, fixture.lease.clone(), initial)
                .unwrap();
            if !opening {
                let ack = wire
                    .security
                    .state
                    .grid_open_response(
                        connection,
                        Uuid::random(),
                        fixture.opening(),
                        fixture.lease.clone(),
                        initial,
                        &mut bytes,
                    )
                    .unwrap();
                output_step(
                    &mut wire.send,
                    &wire.security,
                    &wire.auth,
                    false,
                    None,
                    None,
                    &mut response_pending,
                )
                .await
                .unwrap();
                ack.await.unwrap();
                assert!(matches!(
                    decode_payload(
                        &read_frame(&mut wire.recv, FRAME_MAX).await.unwrap(),
                        Some(Method::ShellGridSubscribe)
                    )
                    .unwrap(),
                    Envelope::Response(_)
                ));
                let cell:GridCell=serde_json::from_value(serde_json::json!({"text":"x","width":1,"foreground":{"kind":"default"},"background":{"kind":"default"},"attributes":{"bold":false,"faint":false,"italic":false,"blink":false,"strike":false,"underline":"none"}})).unwrap();
                let ack = wire
                    .security
                    .state
                    .grid_event(
                        connection,
                        &fixture.lease,
                        EventBody::GridPart(GridPart {
                            revision: Uint(0),
                            index: Uint(0),
                            spans: vec![GridSpan {
                                row: Uint(0),
                                column: Uint(0),
                                cells: vec![cell],
                            }],
                        }),
                        initial,
                        &mut bytes,
                    )
                    .unwrap();
                output_step(
                    &mut wire.send,
                    &wire.security,
                    &wire.auth,
                    false,
                    None,
                    None,
                    &mut response_pending,
                )
                .await
                .unwrap();
                ack.await.unwrap();
                assert!(matches!(
                    decode_payload(&read_frame(&mut wire.recv, FRAME_MAX).await.unwrap(), None)
                        .unwrap(),
                    Envelope::Event(_)
                ));
            }
            let cutoff = Instant::now() + Duration::from_millis(100);
            let completion = if opening {
                wire.security
                    .state
                    .grid_open_response(
                        connection,
                        Uuid::random(),
                        fixture.opening(),
                        fixture.lease.clone(),
                        cutoff,
                        &mut bytes,
                    )
                    .unwrap()
            } else {
                wire.security
                    .state
                    .grid_event(
                        connection,
                        &fixture.lease,
                        EventBody::GridCommit(GridCommit { revision: Uint(0) }),
                        cutoff,
                        &mut bytes,
                    )
                    .unwrap()
            };
            let held = wire.security.guard(&wire.auth).await.unwrap();
            let release = async {
                tokio::time::sleep_until(cutoff + Duration::from_millis(20)).await;
                drop(held);
            };
            let (result, ()) = tokio::join!(
                output_step(
                    &mut wire.send,
                    &wire.security,
                    &wire.auth,
                    false,
                    None,
                    None,
                    &mut response_pending
                ),
                release
            );
            let error = result.err().expect("expired writer cannot succeed");
            assert_eq!(
                FrameFailure::from_error(&error).unwrap().stage,
                FrameStage::WriteDeadline
            );
            assert!(completion.await.is_err());
            assert!(!fixture.lease.active());
            let mut byte = [0];
            assert!(
                tokio::time::timeout(Duration::from_millis(20), wire.recv.read(&mut byte))
                    .await
                    .is_err()
            );
            wire.close().await;
        }
    }
    #[tokio::test]
    async fn real_partial_frame_keeps_the_original_cutoff_and_never_writes_a_suffix_retry() {
        let mut wire = WriterFixture::new().await;
        let payload = vec![b'x'; FRAME_MAX];
        let cutoff = Instant::now() + Duration::from_millis(300);
        let write = async { write_frame_until(&mut wire.send, &payload, cutoff).await };
        let receive = async {
            let mut header = [0; 4];
            wire.recv.read_exact(&mut header).await.unwrap();
            assert_eq!(u32::from_be_bytes(header) as usize, FRAME_MAX);
        };
        let (result, ()) = tokio::join!(write, receive);
        assert_eq!(
            FrameFailure::from_error(&result.unwrap_err())
                .unwrap()
                .stage,
            FrameStage::WriteDeadline
        );
        wire.server.close(0u32.into(), b"partial frame deadline");
        let mut received = 0;
        let mut buffer = [0; 4096];
        while let Ok(Some(n)) = wire.recv.read(&mut buffer).await {
            received += n;
        }
        assert!(
            received < FRAME_MAX,
            "a timed-out frame must not become a complete body"
        );
        wire.close().await;
    }

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
