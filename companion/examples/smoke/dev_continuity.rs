// SPDX-License-Identifier: GPL-3.0-or-later
//! Focused fresh-provider continuity check. It is not Android or final acceptance.
use super::*;

const BROWSE_LIMIT: u64 = 100;
const BROWSE_TOTAL_LIMIT: usize = 1_000;

#[track_caller]
fn development_setup_failure(
    phase: &'static str,
    guard: &'static str,
    error: io::Error,
) -> io::Error {
    let kind = error.kind();
    crate::located_message(
        kind,
        &format!("development setup failed: phase={phase}; guard={guard}; no retry"),
        crate::helper_error_location(&error),
    )
}

fn selected(args: &clap::ArgMatches) -> io::Result<Vec<Provider>> {
    let selected = args
        .get_many::<String>("backend")
        .ok_or_else(|| failure("development continuity requires --backend omp --backend codex"))?
        .map(|provider| match provider.as_str() {
            "omp" => Provider::Omp,
            "codex" => Provider::Codex,
            _ => unreachable!("clap restricts providers"),
        })
        .collect::<Vec<_>>();
    if selected.as_slice() != [Provider::Omp, Provider::Codex] {
        return Err(failure(
            "development continuity requires exactly one fresh OMP and one fresh Codex provider",
        ));
    }
    Ok(selected)
}

pub(super) fn frozen_package(args: &clap::ArgMatches) -> io::Result<PathBuf> {
    let expected = args
        .get_one::<String>("expected-goatr-sha256")
        .ok_or_else(|| failure("development continuity requires a frozen package SHA-256"))?;
    if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(failure("development package SHA-256 format is invalid"));
    }
    let expected_lock = args
        .get_one::<String>("expected-companion-lock-sha256")
        .ok_or_else(|| failure("development continuity requires a frozen source lock SHA-256"))?;
    if expected_lock.len() != 64 || !expected_lock.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(failure("development source lock SHA-256 format is invalid"));
    }
    if expected_lock.to_ascii_lowercase() != transport::lock_sha256() {
        return Err(failure(
            "development runner source lock differs from the explicit frozen source lock",
        ));
    }
    let supplied = args
        .get_one::<PathBuf>("goatr")
        .ok_or_else(|| failure("development continuity requires --goatr"))?;
    goatr::config::absolute(supplied)?;
    let binary = fs::canonicalize(supplied)?;
    let metadata = fs::symlink_metadata(&binary)?;
    if binary != *supplied
        || !metadata.is_file()
        || metadata.uid() != uid()
        || metadata.mode() & 0o777 != 0o500
        || metadata.nlink() != 1
    {
        return Err(failure(
            "development companion must be the exact current-user regular 0500 nlink1 executable",
        ));
    }
    if binary_sha256(&binary)? != expected.to_ascii_lowercase() {
        return Err(failure(
            "selected development package does not match the explicit frozen SHA-256",
        ));
    }
    Ok(binary)
}

pub(super) fn zero_native_caps(process: &ProcessEvidence) -> io::Result<()> {
    let status = fs::read_to_string(format!("/proc/{}/status", process.pid))?;
    for key in ["CapInh:", "CapPrm:", "CapEff:", "CapAmb:"] {
        let value = status
            .lines()
            .find_map(|line| line.strip_prefix(key))
            .and_then(|value| u64::from_str_radix(value.trim(), 16).ok());
        if value != Some(0) {
            return Err(failure(
                "fresh native desktop child retained controller capability",
            ));
        }
    }
    Ok(())
}

fn bootstrap_record(
    evidence: &Path,
    run: &Uuid,
    stage: &str,
    state: &str,
    root: &Path,
    socket: &Path,
    value: Value,
) -> io::Result<()> {
    let path = evidence.join(format!("development-omp-bootstrap-{stage}-{}.json", run.0));
    let record = json!({
        "runId":run,
        "stage":stage,
        "state":state,
        "root":root,
        "herdrSocket":socket,
        "effect":"fresh owned public Herdr setup only",
        "noSessionCreate":true,
        "detail":value,
        "payloads":"not retained",
    });
    if state == "intent" {
        write_json(&path, &record)
    } else {
        write_native_outcome(&path, &record)
    }
}

// Keep only closed classifications from the existing startup observations.
fn surface_failure(
    phase: &'static str,
    error: &io::Error,
) -> (&'static str, &'static str, &'static str) {
    let guard = if error
        .get_ref()
        .is_some_and(|inner| inner.is::<serde_json::Error>())
    {
        "snapshot_decode"
    } else {
        match error.to_string().as_str() {
            "Herdr response timeout" => "response_timeout",
            "Herdr response ID/envelope mismatch" => "response_envelope",
            "Herdr result discriminant mismatch" => "result_discriminant",
            "unsupported Herdr topology protocol" => "topology_protocol",
            "unsafe Herdr socket" => "socket_identity",
            "Herdr socket changed during connect" | "Herdr process/socket epoch changed" => {
                "socket_changed"
            }
            "Herdr source changed after response" => "source_changed",
            "Herdr rejected public request" => "native_rejected",
            "invalid Herdr JSON" => "response_json",
            "truncated Herdr line" => "response_truncated",
            "snapshot absent" => "snapshot_absent",
            _ => "other",
        }
    };
    (phase, guard, OmpObserverFailure::io_kind(error.kind()))
}

async fn direct_omp_bootstrap(
    config: &Config,
    socket: &Path,
    root: &Path,
    evidence: &Path,
    run: &Uuid,
    overlay: &Path,
) -> io::Result<Id> {
    let (before_server, before) = surface(socket).await?;
    before.no_focus()?;
    let before_focus = focus(&before);
    let project = &config.projects[0];
    bootstrap_record(
        evidence,
        run,
        "workspace",
        "intent",
        root,
        socket,
        json!({"projectId":project.id,"project":project.path,"focus":false}),
    )?;
    let workspace = match Client::connect(socket)
        .await?
        .request(
            "workspace.create",
            json!({"cwd":project.path,"label":"Goatr development OMP","focus":false}),
            "workspace_created",
        )
        .await
    {
        Ok(value) => value,
        Err(_) => {
            bootstrap_record(
                evidence,
                run,
                "workspace",
                "uncertain",
                root,
                socket,
                json!({"category":"public_workspace_response_unavailable"}),
            )?;
            return Err(failure(
                "fresh OMP workspace outcome is uncertain; retained without retry",
            ));
        }
    };
    let resources = (|| {
        let pane = serde_json::from_value(
            workspace
                .get("root_pane")
                .and_then(|value| value.get("pane_id"))
                .cloned()
                .ok_or_else(|| failure("fresh OMP workspace root pane absent"))?,
        )
        .map_err(|_| failure("fresh OMP workspace root pane identifier invalid"))?;
        let terminal = serde_json::from_value(
            workspace
                .get("root_pane")
                .and_then(|value| value.get("terminal_id"))
                .cloned()
                .ok_or_else(|| failure("fresh OMP workspace terminal absent"))?,
        )
        .map_err(|_| failure("fresh OMP workspace terminal identifier invalid"))?;
        let workspace_id = serde_json::from_value(
            workspace
                .get("workspace")
                .and_then(|value| value.get("workspace_id"))
                .cloned()
                .ok_or_else(|| failure("fresh OMP workspace identifier absent"))?,
        )
        .map_err(|_| failure("fresh OMP workspace identifier invalid"))?;
        Ok::<(Id, Id, Id), io::Error>((pane, terminal, workspace_id))
    })();
    let (pane, terminal, workspace_id) = match resources {
        Ok(resources) => resources,
        Err(error) => {
            bootstrap_record(
                evidence,
                run,
                "workspace",
                "uncertain",
                root,
                socket,
                json!({"category":"workspace_response_correlation"}),
            )?;
            return Err(error);
        }
    };
    let (after_server, after) = match surface(socket).await {
        Ok(value) => value,
        Err(_) => {
            bootstrap_record(
                evidence,
                run,
                "workspace",
                "uncertain",
                root,
                socket,
                json!({"category":"workspace_post_effect_source"}),
            )?;
            return Err(failure(
                "fresh OMP workspace post-effect source unavailable; retained without launch retry",
            ));
        }
    };
    let actual = after.pane(&pane)?;
    if after_server != before_server
        || focus(&after) != before_focus
        || actual.terminal_id != terminal
        || actual.workspace_id != workspace_id
        || actual.focused
    {
        bootstrap_record(
            evidence,
            run,
            "workspace",
            "uncertain",
            root,
            socket,
            json!({"category":"fresh_workspace_topology_changed"}),
        )?;
        return Err(failure(
            "fresh OMP workspace correlation/topology changed; retained without launch retry",
        ));
    }
    bootstrap_record(
        evidence,
        run,
        "workspace",
        "acknowledged",
        root,
        socket,
        json!({"pane":pane,"terminal":terminal,"workspace":workspace_id,"focusPreserved":true}),
    )?;

    let alias = format!("goatr-omp-{}", &run.0[..8]);
    let project_path = project
        .path
        .to_str()
        .ok_or_else(|| failure("fresh OMP project path is not UTF-8"))?;
    let overlay_path = overlay
        .to_str()
        .ok_or_else(|| failure("fresh OMP overlay path is not UTF-8"))?;
    let mut args = vec![
        "--cwd".to_owned(),
        project_path.to_owned(),
        "--config".to_owned(),
        overlay_path.to_owned(),
    ];
    if overlay.file_name().and_then(|name| name.to_str()) == Some("omp-collab-producer.yml") {
        args.extend([
            "--extension".to_owned(),
            overlay
                .parent()
                .ok_or_else(|| failure("fresh OMP producer overlay parent missing"))?
                .join("goatr-omp-producer-state.ts")
                .into_os_string()
                .into_string()
                .map_err(|_| failure("fresh OMP producer hook path is not UTF-8"))?,
        ]);
    }
    bootstrap_record(
        evidence,
        run,
        "agent_start",
        "intent",
        root,
        socket,
        json!({"pane":pane,"terminal":terminal,"workspace":workspace_id,"alias":alias,"kind":"omp"}),
    )?;
    let started = match Client::connect(socket)
        .await?
        .request(
            "agent.start",
            json!({"name":alias,"kind":"omp","pane_id":pane,"args":args,"timeout_ms":30000}),
            "agent_started",
        )
        .await
    {
        Ok(value) => value,
        Err(_) => {
            bootstrap_record(
                evidence,
                run,
                "agent_start",
                "uncertain",
                root,
                socket,
                json!({"category":"public_agent_start_response_unavailable"}),
            )?;
            return Err(failure(
                "fresh OMP agent.start outcome is uncertain; retained without retry",
            ));
        }
    };
    let agent_value = match started.get("agent").cloned() {
        Some(agent) => agent,
        None => {
            bootstrap_record(
                evidence,
                run,
                "agent_start",
                "uncertain",
                root,
                socket,
                json!({"category":"agent_start_response_correlation"}),
            )?;
            return Err(failure(
                "fresh OMP agent.start response cannot be correlated; retained without retry",
            ));
        }
    };
    let agent: herdr::Agent = match serde_json::from_value(agent_value) {
        Ok(agent) => agent,
        _ => {
            bootstrap_record(
                evidence,
                run,
                "agent_start",
                "uncertain",
                root,
                socket,
                json!({"category":"agent_start_response_correlation"}),
            )?;
            return Err(failure(
                "fresh OMP agent.start response cannot be correlated; retained without retry",
            ));
        }
    };
    let argv = match started.get("argv").and_then(Value::as_array) {
        Some(argv) => argv,
        None => {
            bootstrap_record(
                evidence,
                run,
                "agent_start",
                "uncertain",
                root,
                socket,
                json!({"category":"agent_start_response_correlation"}),
            )?;
            return Err(failure(
                "fresh OMP agent.start argv cannot be correlated; retained without retry",
            ));
        }
    };
    let expected_argv = std::iter::once("omp")
        .chain(args.iter().map(String::as_str))
        .collect::<Vec<_>>();
    if agent.pane_id != pane
        || agent.terminal_id != terminal
        || agent.workspace_id != workspace_id
        || agent.name.as_deref() != Some(alias.as_str())
        || !(agent.launch_pending || agent.agent.as_deref() == Some("omp"))
        || argv.len() != expected_argv.len()
        || !argv
            .iter()
            .zip(expected_argv)
            .all(|(actual, expected)| actual.as_str() == Some(expected))
    {
        bootstrap_record(
            evidence,
            run,
            "agent_start",
            "uncertain",
            root,
            socket,
            json!({"category":"agent_start_correlation"}),
        )?;
        return Err(failure(
            "fresh OMP agent.start correlation failed; retained without retry",
        ));
    }
    bootstrap_record(
        evidence,
        run,
        "agent_start",
        "acknowledged",
        root,
        socket,
        json!({"pane":pane,"terminal":terminal,"workspace":workspace_id,"alias":alias}),
    )?;
    Ok(pane)
}

async fn discovered_omp(
    phone: &mut Phone,
    config: &Config,
    socket: &Path,
    pane: &Id,
) -> io::Result<NativeProof> {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        let inventory = phone.host().await?;
        let matches = inventory
            .sessions
            .iter()
            .filter(|session| {
                session.provider == Provider::Omp
                    && session.pane_id == *pane
                    && session.connectivity == SessionConnectivity::Ready
            })
            .collect::<Vec<_>>();
        if matches.len() == 1 {
            let proof = proof(config, socket, matches[0], None).await?;
            zero_native_caps(&proof.owner)?;
            return Ok(proof);
        }
        if Instant::now() >= deadline {
            // Classify only this already-read projection, not the initiating native cause.
            let bound = inventory
                .sessions
                .iter()
                .filter(|session| session.provider == Provider::Omp && session.pane_id == *pane)
                .collect::<Vec<_>>();
            let unbound = inventory
                .unbound
                .iter()
                .filter(|row| row.provider == Provider::Omp && row.pane_id == *pane)
                .collect::<Vec<_>>();
            let guard = if bound.len() + unbound.len() > 1 {
                "ambiguous_native"
            } else if let Some(session) = bound.first() {
                match session.connectivity {
                    SessionConnectivity::Synchronizing => "bound_synchronizing",
                    SessionConnectivity::Offline => "bound_offline",
                    SessionConnectivity::SetupRequired => "bound_setup_required",
                    SessionConnectivity::Unavailable => "bound_unavailable",
                    SessionConnectivity::Ready => "native_binding",
                }
            } else if let Some(row) = unbound.first() {
                if row.state == UnboundState::Ambiguous {
                    "ambiguous_native"
                } else {
                    match row.reason.as_str() {
                        "Herdr native session report is absent" => "native_session_report_absent",
                        "Herdr native session report agent is not OMP" => {
                            "native_session_report_agent_mismatch"
                        }
                        "Herdr OMP native session report is not a path" => {
                            "native_session_report_non_path"
                        }
                        "OMP collaboration registry is not configured" => "registry_configuration",
                        "No readable private stock collaboration publication; enable desktop control collaboration explicitly" => {
                            "registry_publication"
                        }
                        "Herdr process evidence is unavailable"
                        | "Herdr source evidence is unavailable" => "herdr_process",
                        "Herdr terminal process evidence is unavailable" => "terminal_process",
                        "Herdr reporting namespace cannot be proved" => "herdr_namespace",
                        "Exact native pane/terminal/workspace report changed" => {
                            "pane_report_changed"
                        }
                        "Stock collaboration registry has no candidates" => "registry_empty",
                        "Stock collaboration registry snapshots are unavailable" => {
                            "registry_snapshot_unavailable"
                        }
                        "Registry native ID and exact kernel terminal/owner namespace ancestry join could not be proved" => {
                            "registry_owner_join"
                        }
                        "Stock collaboration registry snapshots and terminal/owner joins are unavailable" => {
                            "registry_snapshot_and_owner_join"
                        }
                        "Stock room authentication/protocol failed; this room will not reconnect until its native generation changes" => {
                            "room_auth_failed"
                        }
                        "Native failed-room evidence capacity reached" => "room_evidence_capacity",
                        "Desktop collaboration is view-only or relay disconnected; control is unavailable" => {
                            "collaboration_control"
                        }
                        "Exact optional native history path/header or namespace object views cannot be proved" => {
                            "native_history"
                        }
                        "Authenticated control link or registry generation changed" => {
                            "control_link_changed"
                        }
                        "Verified stock relay connection timed out" => "relay_connection_timeout",
                        "Verified stock relay connection unavailable" => {
                            "relay_connection_unavailable"
                        }
                        "Stock guest authentication unavailable" => "guest_auth_unavailable",
                        "Stock guest authentication write failed" => "guest_auth_write",
                        "Stock writable welcome unavailable" => "guest_welcome_unavailable",
                        "Stock welcome protocol mismatch" => "welcome_protocol",
                        "Writable stock welcome/header identity mismatch" => {
                            "welcome_header_identity"
                        }
                        "Authenticated registry/welcome cwd changed" => "registry_welcome_cwd",
                        "Registry changed before guest installation" => "registry_before_install",
                        "Registry/process/topology identity changed before native installation" => {
                            "registry_source_before_install"
                        }
                        "Actual connected stock WebSocket FD unavailable" => "guest_socket_fd",
                        "Native identity/reducer installation unavailable" => "native_install",
                        "Stock guest snapshot, history, crypto or source identity became unavailable" => {
                            "guest_stream_lost"
                        }
                        "Native owner/conversation/terminal binding has not been proved" => {
                            "binding_unproved"
                        }
                        _ => "native_binding",
                    }
                }
            } else {
                "pane_not_reported"
            };
            return Err(development_setup_failure(
                "omp_ready",
                guard,
                prerequisite("fresh direct OMP pane did not reach one proven Ready native session"),
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn history_head(
    phone: &mut Phone,
    proof: &NativeProof,
    phase: &'static str,
) -> io::Result<SessionHistoryResult> {
    let ResultBody::SessionHistory(page) = phone
        .rpc(
            RequestBody::SessionHistory(SessionHistoryParams {
                target: proof.target.clone(),
                view_id: None,
                cursor: None,
                limit: Some(Uint(BROWSE_LIMIT)),
            }),
            phase,
        )
        .await?
    else {
        return Err(failure("development history head result mismatch"));
    };
    if page.records.len() > BROWSE_LIMIT as usize {
        return Err(failure("development history head exceeded explicit bound"));
    }
    Ok(page)
}

async fn prepare_bounded_history(
    phone: &mut Phone,
    proof: &NativeProof,
    inputs: &NativeInputs<'_>,
    run: &Uuid,
) -> io::Result<usize> {
    let mut latest = phone.start_generation(&proof.target).await?;
    if target(&latest.state.summary) != proof.target
        || latest.state.summary.native_id != proof.native
    {
        return Err(failure(
            "development history generation changed original native Target",
        ));
    }
    for command in 0..64usize {
        if latest.state.summary.connectivity != SessionConnectivity::Ready
            || latest.state.summary.status != SessionStatus::Idle
            || !latest.pending_requests.is_empty()
        {
            return Err(failure(
                "development bounded history lost its original Ready/Idle boundary",
            ));
        }
        let marker = format!(
            "goatr-development-history-{}-{:?}-{command}",
            &run.0[..8],
            proof.provider
        );
        let shell = format!("printf '%s\\n' '{marker}'");
        let started = Instant::now();
        let deadline = history_command_deadline(
            false,
            proof.provider,
            started + Duration::from_secs(30),
            started,
        );
        phone.history_command = Some(HistoryCommandTrace::default());
        if let Some(observer) = &proof.omp_observer {
            observer.healthy(&proof.target)?;
            observer.state.lock().register(&shell, &marker, deadline)?;
        }
        let completion = tokio::time::timeout_at(deadline.into(), async {
            inputs
                .send(
                    phone,
                    proof,
                    NativeInput::Prompt(&format!("!{shell}")),
                    None,
                    "development_bounded_history",
                )
                .await?;
            inputs.shell_completed(phone, proof, &shell, &marker).await
        })
        .await;
        let (complete, _) = match completion {
            Ok(Ok(value)) => value,
            Ok(Err(error)) => {
                if let Some(observer) = &proof.omp_observer {
                    observer
                        .state
                        .lock()
                        .fail(OmpObserverFailure::CommandFailed(
                            OmpObserverFailure::io_kind(error.kind()),
                        ));
                }
                phone.history_command = None;
                return Err(error);
            }
            Err(_) => {
                if let Some(observer) = &proof.omp_observer {
                    observer
                        .state
                        .lock()
                        .fail(OmpObserverFailure::CommandDeadline);
                }
                let error = phone
                    .history_command
                    .as_ref()
                    .expect("development command context")
                    .deadline();
                phone.history_command = None;
                return Err(error);
            }
        };
        phone.history_command = None;
        latest = complete;
    }
    // Release the original subscription before browse_once opens its paging snapshot.
    let final_snapshot = phone.finish_generation(&proof.target).await?;
    if target(&final_snapshot.state.summary) != proof.target
        || final_snapshot.state.summary.native_id != proof.native
        || final_snapshot.state.summary.connectivity != SessionConnectivity::Ready
        || final_snapshot.state.summary.status != SessionStatus::Idle
        || !final_snapshot.pending_requests.is_empty()
    {
        return Err(failure(
            "development history final snapshot lost original native Target or Ready/Idle boundary",
        ));
    }
    Ok(64)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ToolOutputMarkerState {
    Pending,
    Rejected,
    Ready,
}

pub(super) fn completed_tool_output_state(records: &[HistoryRecord], marker: &str) -> ToolOutputMarkerState {
    let mut matching = records.iter().filter(|record| {
        record.complete
            && record
                .tool
                .as_ref()
                .and_then(|tool| tool.output.as_ref())
                .is_some_and(
                    |output| matches!(output, Content::Text(text) if text.text.0.contains(marker)),
                )
    });
    let Some(first) = matching.next() else {
        return ToolOutputMarkerState::Pending;
    };
    let Some(first_tool) = first.tool.as_ref() else {
        return ToolOutputMarkerState::Rejected;
    };
    let Some(call) = first_tool.call_id.as_ref() else {
        return ToolOutputMarkerState::Rejected;
    };
    let mut pending = matches!(first_tool.status, ToolStatus::Running | ToolStatus::Unknown);
    if first_tool.status == ToolStatus::Failed {
        return ToolOutputMarkerState::Rejected;
    }
    for record in matching {
        let Some(tool) = record.tool.as_ref() else {
            return ToolOutputMarkerState::Rejected;
        };
        if tool.call_id.as_ref() != Some(call) || tool.status == ToolStatus::Failed {
            return ToolOutputMarkerState::Rejected;
        }
        pending |= matches!(tool.status, ToolStatus::Running | ToolStatus::Unknown);
    }
    if pending {
        ToolOutputMarkerState::Pending
    } else {
        ToolOutputMarkerState::Ready
    }
}

pub(super) async fn wait_completed_tool_output(
    phone: &mut Phone,
    proof: &NativeProof,
    marker: &str,
    phase: &'static str,
) -> io::Result<SessionOpenResult> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    loop {
        let open =
            completed_tool_open_before_deadline(deadline, phone.open(&proof.target, phase)).await?;
        if target(&open.state.summary) != proof.target
            || open.state.summary.native_id != proof.native
        {
            return Err(failure(
                "development tool output changed original native Target",
            ));
        }
        match completed_tool_output_state(&open.history, marker) {
            ToolOutputMarkerState::Ready => return Ok(open),
            ToolOutputMarkerState::Rejected => {
                return Err(failure(
                    "development prompt native tool output was rejected or ambiguous",
                ));
            }
            ToolOutputMarkerState::Pending => {}
        }
        let next = tokio::time::Instant::now() + Duration::from_millis(200);
        tokio::time::sleep_until(if next < deadline { next } else { deadline }).await;
    }
}

async fn completed_tool_open_before_deadline<T>(
    deadline: tokio::time::Instant,
    open: impl Future<Output = io::Result<T>>,
) -> io::Result<T> {
    // The absolute deadline includes the complete read-only open path and expires before a
    // terminal history observation can be accepted.
    if tokio::time::Instant::now() >= deadline {
        return Err(prerequisite(
            "development prompt lacked a new completed native tool output marker",
        ));
    }
    let open = tokio::time::timeout_at(deadline, open)
        .await
        .map_err(|_| {
            prerequisite("development prompt lacked a new completed native tool output marker")
        })??;
    if tokio::time::Instant::now() >= deadline {
        return Err(prerequisite(
            "development prompt lacked a new completed native tool output marker",
        ));
    }
    Ok(open)
}

async fn browse_once(
    phone: &mut Phone,
    proof: &NativeProof,
    inputs: &NativeInputs<'_>,
    run: &Uuid,
) -> io::Result<Value> {
    let initial_live = phone
        .open(&proof.target, "development_initial_live_window")
        .await?;
    let initial_ids = initial_live
        .history
        .iter()
        .map(|record| record.id.clone())
        .collect::<HashSet<_>>();
    let mut head = history_head(phone, proof, "development_history_head").await?;
    let commands = if head.next_cursor.is_none() {
        prepare_bounded_history(phone, proof, inputs, run).await?;
        head = history_head(phone, proof, "development_history_head_after_prepare").await?;
        64
    } else {
        0
    };
    let latest = phone
        .open(&proof.target, "development_latest_live_window")
        .await?;
    let latest_ids = latest
        .history
        .iter()
        .map(|record| record.id.clone())
        .collect::<HashSet<_>>();
    let Some(cursor) = head.next_cursor.clone() else {
        return Ok(json!({
            "status":"unproven",
            "reason":"no_native_successor_cursor_within_64_bounded_commands",
            "initialLiveRecords":initial_ids.len(),
            "latestLiveRecords":latest_ids.len(),
            "nativeCommands":commands,
        }));
    };
    let params = SessionHistoryParams {
        target: proof.target.clone(),
        view_id: Some(head.view_id.clone()),
        cursor: Some(cursor),
        limit: Some(Uint(BROWSE_LIMIT)),
    };
    let ResultBody::SessionHistory(next) = phone
        .rpc(
            RequestBody::SessionHistory(params.clone()),
            "development_history_successor",
        )
        .await?
    else {
        return Err(failure("development successor history result mismatch"));
    };
    let ResultBody::SessionHistory(repeated) = phone
        .rpc(
            RequestBody::SessionHistory(params),
            "development_history_successor_repeat",
        )
        .await?
    else {
        return Err(failure(
            "development successor history repeat result mismatch",
        ));
    };
    if next.view_id != head.view_id
        || repeated.view_id != next.view_id
        || repeated.next_cursor != next.next_cursor
        || !next
            .records
            .iter()
            .map(|record| (&record.id, &record.order))
            .eq(repeated
                .records
                .iter()
                .map(|record| (&record.id, &record.order)))
    {
        return Err(failure(
            "development explicit successor cursor was not stable in its original browse view",
        ));
    }
    if head.records.len() + next.records.len() > BROWSE_TOTAL_LIMIT
        || next.records.len() > BROWSE_LIMIT as usize
    {
        return Err(failure(
            "development bounded native browse exceeded its cap",
        ));
    }
    let source = if proof.provider == Provider::Omp {
        RecordSource::Persisted
    } else {
        RecordSource::NativePage
    };
    let older = next.records.iter().find(|record| {
        record.complete && !latest_ids.contains(&record.id) && record.source == source
    });
    let Some(older) = older else {
        return Ok(json!({
            "status":"unproven",
            "reason":"successor_page_lacked_native_record_outside_latest_live_window",
            "initialLiveRecords":initial_ids.len(),
            "latestLiveRecords":latest_ids.len(),
            "nativeCommands":commands,
            "view":head.view_id,
        }));
    };
    Ok(json!({
        "status":"proven",
        "view":head.view_id,
        "nativeCommands":commands,
        "initialLiveRecords":initial_ids.len(),
        "latestLiveRecords":latest_ids.len(),
        "olderRecord":older.id,
        "source":source,
        "stableExplicitSuccessorRepeat":true,
    }))
}

async fn mobile_then_desktop(
    phone: &mut Phone,
    proof: &NativeProof,
    inputs: &NativeInputs<'_>,
    run: &Uuid,
) -> io::Result<Value> {
    let mobile_marker = format!(
        "goatr-development-mobile-{}-{:?}",
        &run.0[..8],
        proof.provider
    );
    let receipt = receipt_result(
        phone
            .request(
                Uuid::random(),
                RequestBody::PromptSubmit(PromptSubmitParams { delivery: None, native_turn_id: None, images: None, skill: None, boot_id: phone.hello.boot_id.clone(),
                target: proof.target.clone(),
                text: PromptText(format!(
                    "In this exact existing session, run the native shell tool to execute printf '%s\\n' '{mobile_marker}'. Do not write files or create/switch sessions. Then repeat {mobile_marker}."
                )), }),
            )
            .await?
            .map_err(|error| {
                failure(&format!(
                    "development mobile prompt rejected: code={:?}; no retry",
                    error.code
                ))
            })?,
    )?;
    if !matches!(
        receipt.state,
        ReceiptState::Dispatched | ReceiptState::Succeeded
    ) || receipt.target.as_ref() != Some(&proof.target)
        || receipt.error.is_some()
    {
        return Err(failure(
            "development mobile prompt was not accepted on original Target",
        ));
    }
    wait_completed_tool_output(
        phone,
        proof,
        &mobile_marker,
        "development_mobile_completed_tool_output",
    )
    .await?;
    inputs.current(phone, proof).await?;
    let desktop_marker = format!(
        "goatr-development-desktop-{}-{:?}",
        &run.0[..8],
        proof.provider
    );
    inputs.marker(phone, proof, &desktop_marker).await?;
    wait_history(phone, &proof.target, &desktop_marker, false).await?;
    inputs.current(phone, proof).await?;
    Ok(json!({
        "provider":proof.provider,
        "target":proof.target,
        "mobileReceiptState":receipt.state,
        "mobileNativeEffectObserved":true,
        "desktopContinuationObserved":true,
    }))
}

pub(super) async fn reconnect_original(
    phone: Phone,
    endpoint: &iroh::Endpoint,
    host: &EndpointId,
    config: &Config,
    socket: &Path,
    original: &[NativeProof],
) -> io::Result<Phone> {
    phone.connection.close(
        0u32.into(),
        b"development continuity reconnect; no write replay",
    );
    let mut reconnected = connect(endpoint, host, None).await?;
    let inventory = reconnected.host().await?;
    for proof_before in original {
        let session = inventory
            .sessions
            .iter()
            .find(|session| {
                target(session) == proof_before.target
                    && session.provider == proof_before.provider
                    && session.native_id == proof_before.native
                    && session.connectivity == SessionConnectivity::Ready
            })
            .ok_or_else(|| failure("reconnected phone lost the original native session"))?;
        let fresh = proof(config, socket, session, None).await?;
        if !proof_before.same_input_binding(&fresh) {
            return Err(failure(
                "reconnected phone observed a different native source; no retarget",
            ));
        }
        fresh.recheck_processes()?;
    }
    Ok(reconnected)
}

pub async fn run(args: &clap::ArgMatches, evidence: &Path) -> io::Result<bool> {
    if !args.get_flag("consent-owned-providers") || !args.get_flag("automate-native-inputs") {
        return Err(failure(
            "development continuity requires explicit owned-provider and native-input consent",
        ));
    }
    selected(args)?;
    let binary = frozen_package(args)?;
    let deployment = Config::load(
        args.get_one::<PathBuf>("config")
            .ok_or_else(|| failure("development continuity requires deployment --config"))?,
    )?;
    for provider in [Provider::Omp, Provider::Codex] {
        if let Some(reason) = deployment.backend_reason(provider)? {
            return Err(prerequisite(reason));
        }
    }
    if !deployment.allow_scoped_shell_probe {
        return Err(failure(
            "actual setup must retain scoped local wrapper consent before fresh Codex bootstrap",
        ));
    }
    let workspace = fs::canonicalize(
        args.get_one::<PathBuf>("workspace-root")
            .ok_or_else(|| failure("development continuity requires --workspace-root"))?,
    )?;
    goatr::config::project_directory(&workspace)?;
    let omp_receiver_capture = owned_omp_receiver_capture_requested();
    let omp_backend_capture = owned_omp_backend_capture_requested();
    if owned_omp_capture_conflicts() {
        return Err(failure(
            "owned OMP creation, post-Ready offline, and backend-only captures are mutually exclusive",
        ));
    }
    // Validate the private diagnostic executable before it can enter this
    // run's serialized configuration or any owned process command.
    let private_herdr = owned_herdr_report_binary(omp_receiver_capture, &workspace)?;
    let runtime = PathBuf::from(
        std::env::var_os("XDG_RUNTIME_DIR")
            .ok_or_else(|| failure("unchanged real user XDG_RUNTIME_DIR required"))?,
    );
    private_dir(&runtime, false)?;
    let run = Uuid::random();
    let suffix = hex(&random32()?[..6]);
    let root = workspace.join(format!(".gs-development-{suffix}"));
    let admin = runtime.join(format!("gd-{suffix}"));
    fs::DirBuilder::new().mode(0o700).create(&root)?;
    fs::DirBuilder::new().mode(0o700).create(&admin)?;
    for path in [
        root.join("state"),
        root.join("runtime"),
        root.join("runtime/b"),
        root.join("project"),
        admin.join("config"),
        admin.join("config/herdr"),
        admin.join("config/herdr/sessions"),
    ] {
        fs::DirBuilder::new().mode(0o700).create(path)?;
    }
    let desktop_config_home = admin.join("config");
    let name = format!("goatr-development-{}", &run.0[..8]);
    let session_dir = admin.join(format!("config/herdr/sessions/{name}"));
    fs::DirBuilder::new().mode(0o700).create(&session_dir)?;
    let herdr_socket = session_dir.join("herdr.sock");
    goatr::config::socket_path(&herdr_socket)?;
    // Herdr also binds this longer native client socket in the same session.
    goatr::config::socket_path(&session_dir.join("herdr-client.sock"))?;
    let socket = admin.join("admin.sock");
    goatr::config::socket_path(&socket)?;

    let mut config = deployment.clone();
    config.runtime_dir = root.join("runtime");
    config.runtime_dir_agent = config.runtime_dir.clone();
    config.herdr_sockets = vec![herdr_socket.clone()];
    config.projects = vec![ConfiguredProject {
        allow_read_tools: false,
        id: Uuid::random(),
        label: Label::new("Goatr development continuity").map_err(io::Error::other)?,
        path: root.join("project"),
    }];
    if let Some(private_herdr) = private_herdr {
        config.executables.herdr = private_herdr;
    }
    let owner = CodexOwner {
        id: Uuid::random(),
        socket_host: root.join("runtime/c.sock"),
        socket_agent: root.join("runtime/c.sock"),
        bindings_dir: root.join("runtime/b"),
        unit: format!("goatr-development-{}.service", &run.0[..8]),
    };
    config.default_codex_owner_id = Some(owner.id.clone());
    config.codex_owners = vec![owner];
    let config_path = root.join("config.toml");
    let lock = goatr::setup::SetupLock::acquire(&root)?;
    lock.publish(
        &config_path,
        &goatr::setup::ExpectedFile::Absent,
        config.serialized()?.as_bytes(),
    )?;
    drop(lock);

    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
    let port = listener.local_addr()?.port();
    listener.set_nonblocking(true)?;
    let metadata = fs::metadata(format!("/proc/self/fd/{}", listener.as_raw_fd()))?;
    let listener_record = goatr::backends::omp::OwnedRelayListener {
        v: 1,
        fd: listener.as_raw_fd(),
        device: metadata.dev(),
        inode: metadata.ino(),
        port,
        parent: herdr::process_evidence(std::process::id())?,
        runtime_dir: config.runtime_dir.clone(),
    };
    let relay_owner = config.runtime_dir.join("omp-relay.owner");
    let relay_lock = goatr::setup::SetupLock::acquire(&config.runtime_dir)?;
    relay_lock.publish(
        &relay_owner,
        &goatr::setup::ExpectedFile::Absent,
        &serde_json::to_vec(&listener_record).map_err(io::Error::other)?,
    )?;
    drop(relay_lock);
    let mut herdr_report_capture = if omp_receiver_capture {
        // The private receiver starts only after the owned listener record and
        // matching OMP overlay exist for this exact run.
        Some(herdr_report_capture::Capture::bind(
            &root,
            uid(),
            hex(&random32()?),
            owned_omp_offline_capture_requested(),
        )?)
    } else {
        None
    };
    let overlay = if let Some(capture) = herdr_report_capture.as_mut() {
        if capture.prepare_omp_producer(&config, port)? {
            config.runtime_dir_agent.join("omp-collab-producer.yml")
        } else {
            goatr::backends::omp::overlay(&config, port)?
        }
    } else {
        goatr::backends::omp::overlay(&config, port)?
    };
    write_json(
        &evidence.join(format!("development-resources-{}.json", run.0)),
        &json!({
            "runId":run,
            "root":identity(&root)?,
            "admin":identity(&admin)?,
            "package":binary,
            "packageSha256":binary_sha256(&binary)?,
            "config":identity(&config_path)?,
            "herdrSocket":herdr_socket,
            "listener":listener_record,
            "overlay":overlay,
            "herdrReportCapture":herdr_report_capture.is_some(),
            "ompBackendCapture":omp_backend_capture,
            "scope":"fresh owned development continuity; no Android/full acceptance claim",
        }),
    )?;
    let mut command = std::process::Command::new(&config.executables.herdr);
    // A fresh private-XDG server must not inherit the controller Herdr context.
    // This is the same bounded context scrub Herdr uses for its own subprocesses.
    for key in [
        "HERDR_SOCKET_PATH",
        "HERDR_CLIENT_SOCKET_PATH",
        "HERDR_SESSION",
        "HERDR_BIN_PATH",
        "HERDR_ENV",
        "HERDR_WORKSPACE_ID",
        "HERDR_TAB_ID",
        "HERDR_PANE_ID",
        "GOATR_OWNED_OMP_PRODUCER_CAPTURE",
        "GOATR_OWNED_OMP_PRODUCER_METADATA_READER",
        "GOATR_OWNED_OMP_PRODUCER_METADATA_READER_SHA256",
        "GOATR_OWNED_OMP_PRODUCER_BUN",
        "GOATR_OWNED_OMP_PRODUCER_BUN_SHA256",
    ] {
        command.env_remove(key);
    }
    command
        .args(["--session", &name, "server"])
        .current_dir(&config.projects[0].path)
        .env("HERDR_STARTUP_CWD", &config.projects[0].path)
        .env("XDG_CONFIG_HOME", &desktop_config_home)
        .env("XDG_STATE_HOME", root.join("state"));
    if let Some(capture) = &herdr_report_capture {
        capture.configure(&mut command);
    }
    let mut desktop = Desktop::spawn(command)
        .map_err(|error| development_setup_failure("herdr_launch", "spawn", error))?;
    if let Some(capture) = &herdr_report_capture {
        capture.pin(&desktop.process);
    }
    write_json(
        &evidence.join(format!("development-herdr-owned-{}.json", run.0)),
        &json!({"runId":run,"process":desktop.process,"expectedSocket":herdr_socket}),
    )?;
    let startup_seconds =
        if owned_omp_creation_capture_requested() && herdr_report_capture.is_some() {
            60
        } else {
            20
        };
    let deadline = Instant::now() + Duration::from_secs(startup_seconds);
    let mut first_surface_failure = None;
    loop {
        // Same connect and snapshot as surface(), with no additional request.
        let observation = match Client::connect(&herdr_socket).await {
            Ok(client) => client
                .snapshot()
                .await
                .map_err(|error| surface_failure("herdr_snapshot", &error)),
            Err(error) => Err(surface_failure("herdr_connect", &error)),
        };
        match observation {
            Ok(_) => {
                if let Some(capture) = &herdr_report_capture {
                    capture.startup_success();
                }
                desktop.recheck().map_err(|error| {
                    development_setup_failure("herdr_ready", "fresh_owned_child", error)
                })?;
                break;
            }
            Err(observation) => {
                first_surface_failure.get_or_insert(observation);
            }
        }
        if let Some(status) = desktop
            .child
            .try_wait()
            .map_err(|error| development_setup_failure("herdr_launch", "child_status", error))?
        {
            let guard = match status.code() {
                Some(0) => "exited_success",
                Some(1) => "exited_1",
                Some(2) => "exited_2",
                Some(126) => "exited_126",
                Some(127) => "exited_127",
                Some(_) => "exited_other",
                None => "signaled",
            };
            return Err(development_setup_failure(
                "herdr_launch",
                guard,
                prerequisite("fresh owned Herdr exited before publishing its expected socket"),
            ));
        }
        if Instant::now() >= deadline {
            if let (Some(capture), Some((phase, guard, io))) =
                (&herdr_report_capture, first_surface_failure)
            {
                capture.startup_failure(phase, guard, io);
            }
            return Err(development_setup_failure(
                "herdr_ready",
                "fresh_socket",
                prerequisite(
                    "fresh owned Herdr source/socket prerequisites unavailable; desktop retained",
                ),
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let mut serve = start_serve_with_codex_capture(
        &binary,
        &config_path,
        &root,
        &socket,
        Some(&listener),
        false,
        herdr_report_capture.as_ref(),
    )
    .map_err(|error| development_setup_failure("companion_launch", "spawn", error))?;
    healthy(&mut serve, &socket)
        .await
        .map_err(|error| development_setup_failure("companion_ready", "admin_socket", error))?;
    let owner_feedback = args.get_flag("development-owner-feedback");
    let android_only = args.get_flag("development-android-only") || owner_feedback;
    // The ordinary continuity path keeps its direct OMP bootstrap. Android feedback
    // instead admits both providers before its regular, receipt-fenced SessionCreate.
    let omp_pane = if android_only {
        None
    } else {
        // Local native startup does not depend on the phone's public-relay readiness.
        Some(direct_omp_bootstrap(&config, &herdr_socket, &root, evidence, &run, &overlay).await?)
    };
    let endpoint = goatr::endpoint_builder(iroh::SecretKey::from_bytes(&random32()?))
        .bind()
        .await
        .map_err(|_| {
            development_setup_failure(
                "phone_endpoint",
                "bind",
                io::Error::other("phone endpoint bind failed"),
            )
        })?;
    let mut phone = pair(&binary, &root, &socket, &endpoint).await?;
    if let Some(budget) = serve.omp_capture_budget.clone() {
        phone.set_omp_capture(budget);
    }
    let host = phone.hello.host_id.clone();
    let discovered_omp = if let Some(omp_pane) = omp_pane.as_ref() {
        Some(discovered_omp(&mut phone, &config, &herdr_socket, omp_pane).await?)
    } else {
        None
    };
    let admission_deadline = Instant::now() + Duration::from_secs(30);
    let server = loop {
        let inventory = phone.host().await?;
        if let Some(server) = inventory.servers.iter().find(|server| {
            server.name.0 == name
                && server.connectivity == ServerConnectivity::Online
                && server.can_create.codex
                && (!android_only || server.can_create.omp)
        }) {
            break server.server_id.clone();
        }
        if Instant::now() >= admission_deadline {
            return Err(prerequisite(if android_only {
                "fresh owned OMP/Codex creation admission unavailable; native setup retained"
            } else {
                "fresh Codex bootstrap admission unavailable; fresh OMP setup retained"
            }));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let mut feedback_presentation = Vec::new();
    let omp = if android_only {
        if owner_feedback {
            let (proof, receipt, presentation) = create_feedback(
                &mut phone,
                &config,
                &herdr_socket,
                Provider::Omp,
                &server,
                evidence,
                &run,
            )
            .await?;
            feedback_presentation.push((receipt, presentation));
            proof
        } else {
            let (proof, _) = create(
                &mut phone,
                &config,
                &herdr_socket,
                Provider::Omp,
                &server,
                evidence,
                &run,
            )
            .await?;
            proof
        }
    } else {
        discovered_omp.ok_or_else(|| failure("fresh direct OMP bootstrap proof absent"))?
    };
    let (codex, codex_receipt) = if android_only {
        if owner_feedback {
            let (session, receipt, presentation) = create_session_feedback(
                &mut phone,
                &config,
                &herdr_socket,
                Provider::Codex,
                &server,
                evidence,
                &run,
            )
            .await?;
            let deadline = Instant::now() + Duration::from_secs(30);
            let proof =
                created_codex_proof(&config, &herdr_socket, &session, &receipt, deadline).await?;
            feedback_presentation.push((receipt.clone(), presentation));
            (proof, receipt)
        } else {
            let (session, receipt) = create_session(
                &mut phone,
                &config,
                &herdr_socket,
                Provider::Codex,
                &server,
                evidence,
                &run,
            )
            .await?;
            let deadline = Instant::now() + Duration::from_secs(60);
            let proof =
                created_codex_proof(&config, &herdr_socket, &session, &receipt, deadline).await?;
            (proof, receipt)
        }
    } else {
        let (proof, receipt) = create(
            &mut phone,
            &config,
            &herdr_socket,
            Provider::Codex,
            &server,
            evidence,
            &run,
        )
        .await?;
        (proof, receipt)
    };
    zero_native_caps(&codex.owner)?;
    let herdr = herdr::process_evidence(desktop.process.pid)?;
    if herdr != desktop.process {
        return Err(failure("fresh owned Herdr process identity changed"));
    }
    let inputs = NativeInputs {
        enabled: true,
        paging_proof: false,
        config: &config,
        socket: &herdr_socket,
        herdr: &herdr,
        evidence,
        run: &run,
        mcp: None,
        plan: RefCell::new(None),
        mcp_dialogs: RefCell::new(HashMap::new()),
    };
    let mut native = vec![omp, codex];
    let mut provider_results = Vec::new();
    for proof in &native {
        if !android_only || proof.provider != Provider::Codex {
            inputs.current(&mut phone, proof).await?;
        }
        if android_only {
            provider_results.push(json!({"native":proof.evidence(),"scope":"fresh native binding only; Rust continuity prelude omitted"}));
        } else {
            let browse = browse_once(&mut phone, proof, &inputs, &run).await?;
            let continuity = mobile_then_desktop(&mut phone, proof, &inputs, &run).await?;
            provider_results
                .push(json!({"browse":browse,"continuity":continuity,"native":proof.evidence()}));
        }
        proof.recheck_processes()?;
    }
    if !android_only {
        phone =
            reconnect_original(phone, &endpoint, &host, &config, &herdr_socket, &native).await?;
    }
    for proof in &native {
        inputs.current(&mut phone, proof).await?;
    }
    if owner_feedback {
        for (receipt, presentation) in &feedback_presentation {
            write_feedback_presentation(evidence, &run, receipt, *presentation)?;
        }
    }
    // Retain completed native evidence before the independent Android gate.
    write_json(
        &evidence.join(format!("development-native-continuity-{}.json", run.0)),
        &json!({
            "runId":run,
            "providers":&provider_results,
            "codexBootstrapReceipt":&codex_receipt,
            "phoneReconnectedBeforeAndroid":!android_only,
            "scope":if android_only {"fresh native bindings only; Rust continuity prelude omitted; Android acceptance not claimed"} else {"native pre-Android continuity; Android acceptance not claimed"},
        }),
    )?;
    let feedback_observations = feedback_presentation
        .iter()
        .map(|(_, presentation)| *presentation)
        .collect::<Vec<_>>();
    ui::development(
        args,
        evidence,
        ui::Context {
            binary: &binary,
            root: &root,
            socket: &socket,
            config: &config,
            herdr_socket: &herdr_socket,
            herdr_process: &herdr,
            mcp: None,
            serve: &mut serve,
            listener: &listener,
            endpoint: &endpoint,
            phone: &mut phone,
            native: &mut native,
            history_witnesses: &[],
            feedback_presentation: &feedback_observations,
        },
        &run,
        &name,
        &desktop_config_home,
    )
    .await?;
    if owner_feedback {
        return Ok(true);
    }
    write_json(
        &evidence.join(format!("development-continuity-outcome-{}.json", run.0)),
        &json!({
            "gate":"development-continuity",
            "status":"stopped",
            "runId":run,
            "package":binary,
            "packageSha256":binary_sha256(&binary)?,
            "companionLockSha256":transport::lock_sha256(),
            "freshHerdr":desktop.process,
            "freshCompanion":serve.process,
            "providers":provider_results,
            "codexBootstrapReceipt":codex_receipt,
            "phoneReconnectedBeforeAndroid":!android_only,
            "scope":if android_only {"fresh OMP/Codex native bindings and actual Android feedback continuity; Rust prelude omitted and Android history observation is not a paging pass"} else {"fresh OMP direct Herdr bootstrap plus Codex SessionCreate; bounded browse, actual Android foreground consumer and real mobile-to-desktop continuation before retained feedback shutdown"},
            "notClaimed":["final Android acceptance","full checkpoint dialogs/Stop","exhaustive older-history crossing"],
        }),
    )?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marker_record(id: &str, call: Option<&str>, status: &str) -> HistoryRecord {
        let mut tool = serde_json::json!({
            "name":"commandExecution",
            "status":status,
            "output":{"kind":"text","text":"goatr-terminal-marker"}
        });
        if let Some(call) = call {
            tool["callId"] = serde_json::json!(call);
        }
        serde_json::from_value(serde_json::json!({
            "id":id,
            "order":0,
            "role":"tool",
            "kind":"tool",
            "source":"live",
            "complete":true,
            "sourceTruncated":false,
            "tool":tool
        }))
        .unwrap()
    }

    #[test]
    fn completed_tool_output_requires_one_terminal_successful_call() {
        let marker = "goatr-terminal-marker";
        let running = marker_record("stable", Some("call-a"), "running");
        let unknown = marker_record("unknown", Some("call-a"), "unknown");
        let succeeded = marker_record("stable", Some("call-a"), "succeeded");
        let failed = marker_record("failed", Some("call-a"), "failed");
        let unkeyed = marker_record("unkeyed", None, "succeeded");
        let second = marker_record("second", Some("call-b"), "succeeded");

        let mut replacement = vec![running.clone()];
        assert_eq!(
            completed_tool_output_state(&replacement, marker),
            ToolOutputMarkerState::Pending
        );
        replacement[0] = succeeded.clone();
        assert_eq!(
            completed_tool_output_state(&replacement, marker),
            ToolOutputMarkerState::Ready
        );
        assert_eq!(
            completed_tool_output_state(&[unknown], marker),
            ToolOutputMarkerState::Pending
        );
        assert_eq!(
            completed_tool_output_state(&[failed.clone()], marker),
            ToolOutputMarkerState::Rejected
        );
        assert_eq!(
            completed_tool_output_state(&[unkeyed], marker),
            ToolOutputMarkerState::Rejected
        );
        assert_eq!(
            completed_tool_output_state(&[succeeded.clone(), failed], marker),
            ToolOutputMarkerState::Rejected
        );
        assert_eq!(
            completed_tool_output_state(&[succeeded, second], marker),
            ToolOutputMarkerState::Rejected
        );
    }

    #[tokio::test]
    async fn completed_tool_output_deadline_covers_a_stalled_open() {
        let error = completed_tool_open_before_deadline(
            tokio::time::Instant::now() + Duration::from_millis(10),
            std::future::pending::<io::Result<()>>(),
        )
        .await
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotConnected);
        assert!(
            error
                .to_string()
                .contains("development prompt lacked a new completed native tool output marker")
        );
    }

    #[tokio::test]
    async fn completed_tool_output_rejects_expired_or_late_open_before_acceptance() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };

        struct LateReady(Arc<AtomicBool>);
        impl Future for LateReady {
            type Output = io::Result<()>;

            fn poll(
                self: std::pin::Pin<&mut Self>,
                _: &mut std::task::Context<'_>,
            ) -> std::task::Poll<Self::Output> {
                self.0.store(true, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(10));
                std::task::Poll::Ready(Ok(()))
            }
        }

        let unpolled = Arc::new(AtomicBool::new(false));
        assert!(
            completed_tool_open_before_deadline(
                tokio::time::Instant::now(),
                LateReady(Arc::clone(&unpolled)),
            )
            .await
            .is_err()
        );
        assert!(!unpolled.load(Ordering::SeqCst));

        let late = Arc::new(AtomicBool::new(false));
        assert!(
            completed_tool_open_before_deadline(
                tokio::time::Instant::now() + Duration::from_millis(1),
                LateReady(Arc::clone(&late)),
            )
            .await
            .is_err()
        );
    }
}
