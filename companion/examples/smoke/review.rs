// SPDX-License-Identifier: GPL-3.0-or-later
//! Non-installed, synthetic provider-data fixture using the actual native host runtime.
//! It is not live provider proof and never launches providers or installs product commands.
use super::{failure, identity, private_dir, removable_tree, write_json};
use clap::{Arg, ArgAction, Command, value_parser};
use goatr::{pairing::*, protocol::*, state::*, storage::*};
use serde::Deserialize;
use serde_json::json;
use std::{
    fs,
    future::Future,
    io,
    os::unix::fs::{DirBuilderExt, MetadataExt},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};

#[derive(Deserialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
enum Control {
    Authorize { endpoint: EndpointId },
    Reset { provider: Provider },
    Pause,
    Resume,
    Activity { text: String },
    Resnapshot { title: String },
    CompletedContent { sample: CompletedSample },
    PendingView { sample: PendingViewSample },
    GenerationBegin,
    SourceBegin,
    Ready,
    Receipt { action: Uuid },
    ForgetPeer,
    Stop,
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum CompletedSample {
    Original,
    Replacement,
    Expired,
    InlineReplacement,
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum PendingViewSample {
    History,
    Open,
}
fn completed_text(sample: &CompletedSample) -> String {
    match sample {
        CompletedSample::Original | CompletedSample::Expired => {
            format!(
                "synthetic redacted completed body A\n{}",
                "€".repeat(21_846)
            )
        }
        CompletedSample::Replacement => {
            format!(
                "synthetic redacted completed body B\n{}",
                "界".repeat(21_846)
            )
        }
        CompletedSample::InlineReplacement => "synthetic current inline completed body".into(),
    }
}
struct Paused(Option<std::sync::mpsc::Sender<()>>);
impl Drop for Paused {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}
fn pause(owner: &StorageOwner) -> io::Result<Paused> {
    let (release, resume) = std::sync::mpsc::channel();
    let (entered, ready) = std::sync::mpsc::channel();
    let mut call = Box::pin(owner.call(move |_| {
        let _ = entered.send(());
        let _ = resume.recv();
        Ok(())
    }));
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    if call.as_mut().poll(&mut context).is_ready() {
        return Err(failure("SQLite pause admission failed"));
    }
    ready
        .recv_timeout(Duration::from_secs(2))
        .map_err(|_| failure("SQLite pause unavailable"))?;
    Ok(Paused(Some(release)))
}
fn state(provider: Provider) -> SessionState {
    SessionState {
        summary: SessionSummary {
            session_id: Uuid::random(),
            generation: Uuid::random(),
            server_id: Uuid::random(),
            workspace_id: Id("fixture-workspace".into()),
            terminal_id: Id("fixture-terminal".into()),
            pane_id: Id("fixture-pane".into()),
            provider,
            native_id: Id("fixture-native".into()),
            title: "synthetic provider-data fixture".into(),
            status: SessionStatus::Idle,
            connectivity: SessionConnectivity::Ready,
            capabilities: Capabilities {
                controls: false,
                terminal_create: false,
                artifacts: false,
                attachments: false,
                history: true,
                prompt: true,
                cancel: true,
                respond: true,
                prompt_mode: PromptMode::SubmitOrSteer,
                cancel_mode: if provider == Provider::Omp {
                    CancelMode::CurrentWork
                } else {
                    CancelMode::NativeTurn
                },
                terminal_read: false,
                worktrees_read: false,
                changes_read: false,
                workspace_rename: false,
                agent_rename: false,
                skills_read: false,
            },
            skills_generation: None,
            history_coverage: HistoryCoverage {
                scope: if provider == Provider::Omp {
                    HistoryScope::AllBranches
                } else {
                    HistoryScope::Thread
                },
                source: if provider == Provider::Omp {
                    HistorySource::PersistedAndLive
                } else {
                    HistorySource::NativePages
                },
                older_available: false,
                source_truncated: false,
                desktop_branch_known: false,
            },
            reason: None,
        },
        native_turn_id: None,
        desktop_selection: DesktopSelection::Unknown,
        activity: Activity {
            state: ActivityState::Idle,
            text: None,
            tool_record_id: None,
        },
    }
}
fn target(state: &SessionState) -> Target {
    Target {
        session_id: state.summary.session_id.clone(),
        generation: state.summary.generation.clone(),
    }
}
fn snapshot(state: &SessionState) -> ProviderSnapshot {
    ProviderSnapshot {
        state: state.clone(),
        pending: vec![],
        history: vec![],
        older_available: false,
        native_older: None,
    }
}
fn policy<T>(result: Result<T, ErrorCode>) -> io::Result<T> {
    result.map_err(|_| failure("fixture state transition rejected"))
}

/// Read the existing smoke runner's schema1 lease. Extra runner evidence is unrelated;
/// this fixture verifies the actual UID/root/boot/PID/start/executable and console AVD.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EmulatorLease {
    schema: u32,
    serial: String,
    owner_uid: u32,
    root: PathBuf,
    avd_name: String,
    adb_client_home: PathBuf,
    process_identity: ProcessIdentity,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProcessIdentity {
    linux_boot_id: String,
    pid: u32,
    start_ticks: String,
    executable: PathBuf,
}
async fn adb(lease: &EmulatorLease, args: &[&str]) -> io::Result<String> {
    let mut command = tokio::process::Command::new("adb");
    super::live::configure_adb_client_home(&mut command, &lease.root, &lease.adb_client_home)?;
    let output = tokio::time::timeout(
        Duration::from_secs(10),
        command
            .arg("-s")
            .arg(&lease.serial)
            .args(args)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| failure("owned adb command timed out"))??;
    if !output.status.success() || output.stdout.len() > 16384 {
        return Err(failure("owned adb command failed"));
    }
    String::from_utf8(output.stdout).map_err(io::Error::other)
}
fn lease_matches(lease: &EmulatorLease) -> bool {
    let process = &lease.process_identity;
    lease.schema == 1
        && lease.owner_uid == uid()
        && private_dir(&lease.root, false).is_ok()
        && super::boot().is_ok_and(|boot| boot == process.linux_boot_id)
        && super::start(process.pid).is_ok_and(|start| start == process.start_ticks)
        && fs::metadata(format!("/proc/{}", process.pid))
            .is_ok_and(|meta| meta.uid() == lease.owner_uid)
        && fs::read_link(format!("/proc/{}/exe", process.pid))
            .is_ok_and(|exe| exe == process.executable)
        && process.executable.starts_with("/nix/store")
}
fn mapping_present(list: &str, local: &str, remote: &str) -> bool {
    list.lines().any(|line| {
        let fields: Vec<_> = line.split_whitespace().collect();
        fields.len() == 3 && fields[1] == local && fields[2] == remote
    })
}

pub async fn run() -> io::Result<bool> {
    let args = Command::new("smoke")
        .arg(
            Arg::new("review-fixture")
                .long("review-fixture")
                .action(ArgAction::SetTrue)
                .required(true),
        )
        .arg(
            Arg::new("evidence-dir")
                .long("evidence-dir")
                .required(true)
                .value_parser(value_parser!(PathBuf)),
        )
        .arg(
            Arg::new("emulator-owner")
                .long("emulator-owner")
                .required(true)
                .value_parser(value_parser!(PathBuf)),
        )
        .get_matches();
    let lease_path = args
        .get_one::<PathBuf>("emulator-owner")
        .expect("required lease");
    private_file(lease_path)?;
    if fs::metadata(lease_path)?.len() > 65536 {
        return Err(failure("emulator lease exceeds bound"));
    }
    let lease: EmulatorLease = serde_json::from_slice(&fs::read(lease_path)?)
        .map_err(|_| failure("invalid owned emulator lease"))?;
    if lease.serial != "emulator-5580" || !lease_matches(&lease) {
        return Err(failure("owned emulator-5580 process lease does not match"));
    }
    if lease_path.parent() != Some(lease.root.as_path())
        || fs::canonicalize(lease_path)? != *lease_path
    {
        return Err(failure(
            "emulator lease is outside its canonical private root",
        ));
    }
    let avd = adb(&lease, &["emu", "avd", "name"]).await?;
    if avd.lines().next() != Some(lease.avd_name.as_str()) {
        return Err(failure(
            "selected serial AVD does not match ownership lease",
        ));
    }
    if adb(&lease, &["get-state"]).await?.trim() != "device" {
        return Err(failure("owned emulator is not ready"));
    }
    let evidence = std::path::absolute(
        args.get_one::<PathBuf>("evidence-dir")
            .expect("required evidence"),
    )?;
    private_dir(&evidence, true)?;
    let run = Uuid::random();
    let root = std::env::temp_dir().join(format!("goatr-review-{}", run.0));
    fs::DirBuilder::new().mode(0o700).create(&root)?;
    let root_identity = fs::symlink_metadata(&root)?;
    write_json(
        &evidence.join(format!("review-root-{}.json", run.0)),
        &json!({
            "gate":"review-native-consumer-fixture", "runId":run, "root":identity(&root)?,
            "emulatorLease":lease_path, "stage":"owned private root; no endpoint or reverse mapping yet"
        }),
    )?;
    let identity_owner = HostIdentity::acquire(root.clone())?;
    let storage = Storage::open(
        &root.join("state.sqlite3"),
        identity_owner.receipt_key,
        now_ms(),
        uid(),
    )
    .map_err(|_| failure("private fixture SQLite unavailable"))?;
    let boot = storage.boot_id.clone();
    let endpoint = goatr::endpoint_builder(identity_owner.secret.clone())
        .bind()
        .await
        .map_err(|_| failure("fixture endpoint unavailable"))?;
    let host = EndpointId(endpoint.id().to_string());
    let security = Security::new(
        StorageOwner::new(storage).map_err(|_| failure("fixture SQLite owner unavailable"))?,
        Arc::new(State::default()),
        host.clone(),
        boot.clone(),
        vec![],
        None,
    );
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
    let port = listener.local_addr()?.port();
    let mapping = format!("tcp:{port}");
    let previous = adb(&lease, &["reverse", "--list"]).await?;
    if previous
        .lines()
        .any(|line| line.split_whitespace().any(|field| field == mapping))
    {
        return Err(failure("refusing pre-existing adb reverse mapping"));
    }
    write_json(
        &evidence.join(format!("review-owned-{}.json", run.0)),
        &json!({
            "gate":"review-native-consumer-fixture", "runId":run, "emulatorLease":lease_path,
            "serial":lease.serial, "root":identity(&root)?, "reverse":{"local":mapping,"remote":mapping},
            "scope":"synthetic provider data; no live provider effects"
        }),
    )?;
    adb(&lease, &["reverse", "--no-rebind", &mapping, &mapping]).await?;
    let (stop, rx) = tokio::sync::watch::channel(false);
    let server_endpoint = endpoint.clone();
    let server_security = security.clone();
    let server = tokio::spawn(async move {
        goatr::transport::serve(&server_endpoint, server_security, None, None, None, rx).await
    });
    let mut paused = None;
    let result = async {
        tokio::time::timeout(Duration::from_secs(45), endpoint.online())
            .await
            .map_err(|_| failure("real N0 discovery/relay readiness unavailable"))?;
        write_json(
            &evidence.join(format!("review-ready-{}.json", run.0)),
            &json!({
                "gate":"review-native-consumer-fixture", "scope":"synthetic provider data; no live provider effects",
                "runId":run, "hostEndpoint":host, "bootId":boot, "controlPort":port,
                "serial":lease.serial, "emulatorLease":lease_path, "reverse":{"local":mapping,"remote":mapping},
                "root":identity(&root)?, "packageVersion":env!("CARGO_PKG_VERSION"),
                "companionLockSha256":goatr::transport::lock_sha256(), "coreIroh":"1.3.0"
            }),
        )?;
        println!(
            "{}",
            json!({"gate":"review-native-consumer-fixture","status":"ready","controlPort":port,
            "evidence":evidence.join(format!("review-ready-{}.json", run.0))})
        );
        let mut current = state(Provider::Omp);
        let source = policy(security.state.begin_install(current.clone()))?;
        policy(security.state.finish_install(&source, snapshot(&current)))?;
        let mut installing = None;
        let mut peer = None;
        loop {
            let (stream, address) = listener.accept().await?;
            if !address.ip().is_loopback() {
                return Err(failure("non-local fixture control refused"));
            }
            let mut stream = tokio::io::BufReader::new(stream);
            let mut bytes = Vec::new();
            let read = tokio::time::timeout(
                Duration::from_secs(5),
                (&mut stream).take(4097).read_until(b'\n', &mut bytes),
            )
            .await
            .map_err(|_| failure("fixture control read deadline"))??;
            if read == 0 || bytes.len() > 4096 || bytes.last() != Some(&b'\n') {
                return Err(failure("fixture control frame rejected"));
            }
            let command: Control = serde_json::from_value(
                parse_raw_json(&bytes).map_err(|_| failure("invalid fixture control JSON"))?,
            )
            .map_err(|_| failure("unexpected fixture control command"))?;
            let mut out = json!({"ok":true, "host":host, "boot":boot});
            let stopping = matches!(command, Control::Stop);
            match command {
                Control::Authorize { endpoint } => {
                    if paused.is_some() {
                        return Err(failure("authorize while paused refused"));
                    }
                    let device = PairedDevice {
                        device_id: Uuid::random(),
                        endpoint_id: endpoint,
                        device_name: Label("native consumer fixture".into()),
                        paired_at: Uint(now_ms()),
                        last_seen_at: None,
                    };
                    let (proof, device_id) = security
                        .storage
                        .call(move |s| {
                            let epoch = s.pair_device(&device)?;
                            Ok((
                                s.authenticated_hello(&device.device_id, epoch)?,
                                device.device_id,
                            ))
                        })
                        .await
                        .map_err(|_| failure("fixture authorization rejected"))?;
                    peer = Some(proof);
                    out["host"] = json!(host);
                    out["device"] = json!(device_id);
                    out["boot"] = json!(boot);
                }
                Control::Reset { provider } => {
                    if installing.is_some() {
                        return Err(failure("reset during installation refused"));
                    }
                    security.state.retire_session(&current.summary.session_id);
                    current = state(provider);
                    let source = policy(security.state.begin_install(current.clone()))?;
                    policy(security.state.finish_install(&source, snapshot(&current)))?;
                    out["target"] = json!(target(&current));
                }
                Control::Pause => {
                    if paused.is_some() {
                        return Err(failure("duplicate pause refused"));
                    }
                    paused = Some(pause(&security.storage)?);
                }
                Control::Resume => {
                    if paused.take().is_none() {
                        return Err(failure("resume without pause refused"));
                    }
                }
                Control::Activity { text } => {
                    let text = Label::new(text)
                        .map_err(|_| failure("fixture activity label rejected"))?
                        .0;
                    policy(security.state.update(
                        &target(&current),
                        SessionUpdate::Activity(Activity {
                            state: ActivityState::Idle,
                            text: Some(Content::Text(ContentText {
                                text: InlineText(text),
                            })),
                            tool_record_id: None,
                        }),
                    ))?;
                }
                Control::Resnapshot { title } => {
                    let title = Label::new(title)
                        .map_err(|_| failure("fixture title rejected"))?
                        .0;
                    if installing.is_some() {
                        return Err(failure("resnapshot during installation refused"));
                    }
                    let source = policy(security.state.begin_source_install(&target(&current)))?;
                    current.summary.title = title;
                    policy(security.state.finish_install(&source, snapshot(&current)))?;
                }
                Control::CompletedContent { sample } => {
                    if installing.is_some() || paused.is_some() {
                        return Err(failure(
                            "completed content during installation/pause refused",
                        ));
                    }
                    let target = target(&current);
                    let source = policy(security.state.source_epoch(&target))?;
                    let at = if matches!(sample, CompletedSample::Expired) {
                        now_ms().saturating_sub(CONTENT_TTL_MS + 1)
                    } else {
                        now_ms()
                    };
                    let content = policy(security.state.content(
                        &target,
                        Mime::TextPlain,
                        completed_text(&sample),
                        at,
                    ))?;
                    let record = HistoryRecord {
                        id: Id("fixture-completed-body".into()),
                        parent_id: None,
                        turn_id: None,
                        order: Uint(0),
                        timestamp: None,
                        role: Role::Assistant,
                        kind: RecordKind::Message,
                        text: Some(content.clone()),
                        tool: None,
                        original: None,
                        source: RecordSource::Live,
                        complete: true,
                        source_truncated: false,
                    };
                    policy(security.state.history_from_source(
                        &target,
                        &source,
                        vec![policy(SafeHistory::from_redacted_display(record))?],
                    ))?;
                    out["content"] = json!(content);
                }
                Control::PendingView { sample } => {
                    if installing.is_some()
                        || paused.is_some()
                        || current.summary.provider != Provider::Omp
                    {
                        return Err(failure(
                            "pending display requires the ready unpaused OMP projection fixture",
                        ));
                    }
                    let source = policy(security.state.begin_source_install(&target(&current)))?;
                    current.summary.status = if matches!(sample, PendingViewSample::Open) {
                        SessionStatus::Attention
                    } else {
                        SessionStatus::Idle
                    };
                    let mut display = snapshot(&current);
                    for index in 0..12 {
                        display
                            .history
                            .push(policy(SafeHistory::from_redacted_display(HistoryRecord {
                                id: Id(format!("fixture-history-{index}")),
                                parent_id: None,
                                turn_id: None,
                                order: Uint(index),
                                timestamp: None,
                                role: Role::Assistant,
                                kind: RecordKind::Message,
                                text: Some(Content::Text(ContentText {
                                    text: InlineText(format!(
                                        "synthetic completed history {index}\n{}",
                                        "Redacted native display line with whitespace preserved.\n"
                                            .repeat(64)
                                    )),
                                })),
                                tool: None,
                                original: None,
                                source: RecordSource::Live,
                                complete: true,
                                source_truncated: false,
                            }))?);
                    }
                    if matches!(sample, PendingViewSample::Open) {
                        for index in 0..2 {
                            display.pending.push(PendingRequest {
                                request_key: Uuid::random(),
                                provider: Provider::Omp,
                                kind: PresentationKind::OmpEditor,
                                presentation: Presentation::OmpEditor(PresentationOmpEditor {
                                    title: Content::Text(ContentText {
                                        text: InlineText(format!(
                                            "synthetic blocking editor {index}"
                                        )),
                                    }),
                                    prefill: Content::Text(ContentText {
                                        text: InlineText(String::new()),
                                    }),
                                }),
                            });
                        }
                    }
                    policy(security.state.finish_install(&source, display))?;
                }
                Control::GenerationBegin => {
                    if installing.is_some() {
                        return Err(failure("duplicate installation refused"));
                    }
                    current.summary.generation = Uuid::random();
                    current.summary.connectivity = SessionConnectivity::Synchronizing;
                    installing = Some(policy(security.state.begin_install(current.clone()))?);
                    policy(security.state.replace_inventory(
                        goatr::state::NativeInventory {
                            servers: vec![],
                            projects: vec![],
                            shells: vec![],
                            unbound: vec![],
                            hierarchy: vec![],
                        },
                        &[],
                        |_| Ok(()),
                    ))?;
                    out["target"] = json!(target(&current));
                }
                Control::SourceBegin => {
                    if installing.is_some() {
                        return Err(failure("duplicate installation refused"));
                    }
                    installing = Some(policy(
                        security.state.begin_source_install(&target(&current)),
                    )?);
                    current.summary.connectivity = SessionConnectivity::Synchronizing;
                    out["target"] = json!(target(&current));
                }
                Control::Ready => {
                    let source = installing
                        .take()
                        .ok_or_else(|| failure("ready without installation refused"))?;
                    current.summary.connectivity = SessionConnectivity::Ready;
                    policy(security.state.finish_install(&source, snapshot(&current)))?;
                    out["target"] = json!(target(&current));
                }
                Control::Receipt { action } => {
                    if paused.is_some() {
                        return Err(failure("receipt while paused refused"));
                    }
                    let proof = peer
                        .clone()
                        .ok_or_else(|| failure("receipt before authorization refused"))?;
                    let request = Request {
                        id: action.clone(),
                        body: RequestBody::PromptSubmit(PromptSubmitParams {
                            delivery: None,
                            native_turn_id: None,
                            images: None,
                            skill: None,
                            boot_id: boot.clone(),
                            target: target(&current),
                            text: PromptText("synthetic retained action".into()),
                        }),
                    };
                    security
                        .storage
                        .call(move |s| {
                            s.reserve(&proof, &request, now_ms(), |_| Ok(()))?;
                            s.transition(
                                &proof,
                                &action,
                                ReceiptTransition {
                                    state: ReceiptState::Uncertain,
                                    proof: TransitionProof::LostOutcome,
                                    result: None,
                                    error: None,
                                    at: now_ms(),
                                },
                            )
                        })
                        .await
                        .map_err(|_| failure("fixture retained receipt rejected"))?;
                }
                Control::ForgetPeer => {
                    drop(paused.take());
                    if let Some(source) = installing.take() {
                        policy(security.state.abort_install(&target(&current), &source))?;
                    }
                    if let Some(proof) = peer.take() {
                        policy(security.revoke(proof.peer_id().clone(), None).await)?;
                    }
                }
                Control::Stop => {}
            }
            out["target"] = json!(target(&current));
            let response = serde_json::to_vec(&out).map_err(io::Error::other)?;
            if response.len() > 16384 {
                return Err(failure("fixture response exceeded bound"));
            }
            tokio::time::timeout(Duration::from_secs(5), async {
                stream.get_mut().write_all(&response).await?;
                stream.get_mut().write_all(b"\n").await
            })
            .await
            .map_err(|_| failure("fixture control write deadline"))??;
            if stopping {
                break;
            }
        }
        Ok::<(), io::Error>(())
    };
    let result = tokio::select! {
        result = result => result,
        _ = tokio::signal::ctrl_c() => Err(failure("fixture cancelled")),
        _ = tokio::time::sleep(Duration::from_secs(900)) => Err(failure("fixture lifetime expired")),
    };
    drop(paused);
    let _ = stop.send(true);
    let transport = server
        .await
        .map_err(|_| failure("fixture transport task failed"));
    let settled = security.shutdown().await;
    let closed = goatr::close_endpoint(&endpoint).await;
    let mapping_list = adb(&lease, &["reverse", "--list"]).await;
    let cleanup = if lease_matches(&lease)
        && mapping_list
            .as_ref()
            .is_ok_and(|list| mapping_present(list, &mapping, &mapping))
    {
        adb(&lease, &["reverse", "--remove", &mapping])
            .await
            .map(|_| ())
    } else {
        Err(failure(
            "reverse mapping or emulator lease changed; cleanup refused",
        ))
    };
    let security =
        Arc::into_inner(security).ok_or_else(|| failure("fixture security still retained"))?;
    Arc::into_inner(security.storage)
        .ok_or_else(|| failure("fixture SQLite still retained"))?
        .close()
        .await
        .map_err(|_| failure("fixture SQLite closure failed"))?;
    drop(identity_owner);
    let observed_root = fs::symlink_metadata(&root)?;
    if observed_root.dev() != root_identity.dev() || observed_root.ino() != root_identity.ino() {
        return Err(failure("fixture root changed; cleanup refused"));
    }
    removable_tree(&root)?;
    fs::remove_dir_all(&root)?;
    write_json(
        &evidence.join(format!("review-cleanup-{}.json", run.0)),
        &json!({
            "gate":"review-native-consumer-fixture", "runId":run, "reverse":{"local":mapping,"remote":mapping},
            "mappingRemoved":cleanup.is_ok(), "rootRemoved":true, "scope":"synthetic provider data; no live provider effects"
        }),
    )?;
    cleanup?;
    transport??;
    closed?;
    settled.map_err(|_| failure("fixture entered boundary could not settle"))?;
    result?;
    Ok(true)
}
