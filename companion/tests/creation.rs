// SPDX-License-Identifier: GPL-3.0-or-later
use goatr::{protocol::*, storage::*};
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};
fn id(s: &str) -> Id {
    Id::new(s).unwrap()
}
fn peer(store: &mut Storage) -> AuthenticatedPeer {
    let device = PairedDevice {
        device_id: Uuid::random(),
        endpoint_id: EndpointId(iroh::SecretKey::from_bytes(&[79; 32]).public().to_string()),
        device_name: Label::new("phone").unwrap(),
        paired_at: Uint(0),
        last_seen_at: None,
    };
    let epoch = store.pair_device(&device).unwrap();
    store.authenticated_hello(&device.device_id, epoch).unwrap()
}
// The ledger consumes a caller's matched result; the real producer is exercised by
// the owned public-Herdr/same-shell fixture in security.rs.
fn confirmed_shell_probe(
    store: &mut Storage,
    peer: &AuthenticatedPeer,
    action: &Uuid,
    resources: &CreationResources,
    at: u64,
) {
    let process = goatr::herdr::process_evidence(std::process::id()).unwrap();
    let metadata = StageMetadata {
        workspace_operation: WorkspaceOperation::ShellProbe,
        shell_probe: Some(ShellProbeMetadata {
            attempt_id: Uuid::random(),
            action_id: action.clone(),
            workspace_id: resources.workspace_id.clone().unwrap(),
            pane_id: resources.pane_id.clone().unwrap(),
            terminal_id: resources.terminal_id.clone().unwrap(),
            shell: process,
            executable: std::env::current_exe().unwrap().to_str().unwrap().into(),
            result_path: "/tmp/owned-ledger-result".into(),
            result_device: 1,
            result_inode: 1,
            parent_device: 1,
            parent_inode: 1,
            focused_workspace_id: id("desktop"),
        }),
        ..StageMetadata::default()
    };
    store
        .creation_intent(peer, action, CreationStage::Workspace, metadata.clone(), at)
        .unwrap();
    store
        .creation_result(
            peer,
            action,
            CreationOutcome {
                resources: resources.clone(),
                metadata,
                state: StageState::Confirmed,
                definitive: Some(id("caller matched shell result")),
                at,
            },
        )
        .unwrap();
}
fn prepare_workspace(
    store: &mut Storage,
    peer: &AuthenticatedPeer,
    provider: Provider,
) -> (Request, CreationResources) {
    let request = Request {
        id: Uuid::random(),
        body: RequestBody::SessionCreate(SessionCreateParams {
            directory_ref: None,
            placement: None,
            boot_id: store.boot_id.clone(),
            server_id: Uuid::random(),
            project_id: Uuid::random(),
            provider,
            name: Label::new("same desktop").unwrap(),
        }),
    };
    store.reserve(peer, &request, 0, |_| Ok(())).unwrap();
    store
        .transition(
            peer,
            &request.id,
            ReceiptTransition {
                state: ReceiptState::Dispatching,
                proof: TransitionProof::Intent,
                result: None,
                error: None,
                at: 1,
            },
        )
        .unwrap();
    store
        .creation_intent(
            peer,
            &request.id,
            CreationStage::Workspace,
            StageMetadata::default(),
            2,
        )
        .unwrap();
    let resources = CreationResources {
        checkout: None,
        workspace_id: Some(id("workspace")),
        pane_id: Some(id("pane")),
        terminal_id: Some(id("terminal")),
        native_id: None,
        binding_id: None,
        session_id: None,
        generation: None,
        owner_started: None,
        relay_started: None,
    };
    store
        .creation_result(
            peer,
            &request.id,
            CreationOutcome {
                resources: resources.clone(),
                metadata: StageMetadata::default(),
                state: StageState::Confirmed,
                definitive: Some(id("exact root response")),
                at: 3,
            },
        )
        .unwrap();
    confirmed_shell_probe(store, peer, &request.id, &resources, 4);
    (request, resources)
}
fn prepare(store: &mut Storage, peer: &AuthenticatedPeer) -> (Request, CreationResources) {
    let (request, resources) = prepare_workspace(store, peer, Provider::Omp);
    store
        .creation_intent(
            peer,
            &request.id,
            CreationStage::Launch,
            StageMetadata::default(),
            4,
        )
        .unwrap();
    (request, resources)
}
fn ack(resources: CreationResources) -> CreationOutcome {
    CreationOutcome {
        resources,
        metadata: StageMetadata::default(),
        state: StageState::Dispatching,
        definitive: Some(id("correlated agent.start submission ACK")),
        at: 5,
    }
}
struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        let path = PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(format!(".goatr-launch-{}", Uuid::random().0));
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
#[test]
fn submitted_launch_waits_for_native_identity_outside_fence_and_revoke_preserves_uncertainty() {
    let mut store = Storage::memory([2; 32], 0).unwrap();
    let p = peer(&mut store);
    let (request, mut resources) = prepare(&mut store, &p);
    let ticket = store.dispatch_ticket(&p, &request.id).unwrap();
    store
        .settle_creation_stage(ticket, ack(resources.clone()))
        .unwrap();
    let visible = store.status(&p, &request.id, 6).unwrap().unwrap();
    assert_eq!(visible.state, ReceiptState::Dispatching);
    let progress = visible.creation.unwrap();
    assert_eq!(progress.stage_state, StageState::Dispatching);
    assert_eq!(progress.resources.native_id, None);
    assert!(matches!(
        store.creation_intent(
            &p,
            &request.id,
            CreationStage::Ready,
            StageMetadata::default(),
            6
        ),
        Err(StorageError::Policy(ErrorCode::InvalidRequest))
    ));
    // Only the surviving original job's correlated native proof confirms the already-submitted
    // launch; this is a read-only result, not permission to submit agent.start again.
    resources.native_id = Some(id("actual-native-conversation"));
    store
        .creation_result(
            &p,
            &request.id,
            CreationOutcome {
                resources: resources.clone(),
                metadata: StageMetadata::default(),
                state: StageState::Confirmed,
                definitive: Some(id("same owner/terminal native readiness")),
                at: 7,
            },
        )
        .unwrap();
    store
        .creation_intent(
            &p,
            &request.id,
            CreationStage::Ready,
            StageMetadata::default(),
            8,
        )
        .unwrap();
    for revoke_before_ack in [false, true] {
        let root = Root::new();
        let path = root.0.join("state.sqlite3");
        let mut store = Storage::open(&path, [3; 32], 0, goatr::pairing::uid()).unwrap();
        let p = peer(&mut store);
        let (request, resources) = prepare(&mut store, &p);
        let ticket = store.dispatch_ticket(&p, &request.id).unwrap();
        if revoke_before_ack {
            store.revoke_epoch(p.peer_id(), 5).unwrap();
        }
        store
            .settle_creation_stage(ticket, ack(resources.clone()))
            .unwrap();
        if !revoke_before_ack {
            store.revoke_epoch(p.peer_id(), 6).unwrap();
        }
        assert!(matches!(
            store.status(&p, &request.id, 7),
            Err(StorageError::Policy(ErrorCode::Unauthorized))
        ));
        drop(store);
        let db = rusqlite::Connection::open(&path).unwrap();
        let bytes: String = db
            .query_row(
                "SELECT receipt FROM receipts WHERE action=?1",
                [&request.id.0],
                |r| r.get(0),
            )
            .unwrap();
        let receipt: Receipt = serde_json::from_str(&bytes).unwrap();
        assert_eq!(receipt.state, ReceiptState::Uncertain);
        let progress = receipt.creation.unwrap();
        assert_eq!(progress.stage_state, StageState::Uncertain);
        assert_eq!(progress.resources, resources);
        drop(db);
    }
}
#[test]
fn acknowledged_launch_restart_is_uncertain_without_replay_and_matching_old_action_precedes_validation()
 {
    let root = Root::new();
    let path = root.0.join("state.sqlite3");
    let mut store = Storage::open(&path, [5; 32], 0, goatr::pairing::uid()).unwrap();
    let p = peer(&mut store);
    let (request, resources) = prepare(&mut store, &p);
    let ticket = store.dispatch_ticket(&p, &request.id).unwrap();
    store
        .settle_creation_stage(ticket, ack(resources.clone()))
        .unwrap();
    drop(store);
    let mut reopened = Storage::open(&path, [5; 32], 9, goatr::pairing::uid()).unwrap();
    let Reservation::Existing(receipt) = reopened
        .reserve(&p, &request, 10, |_| {
            panic!("existing old action must never revalidate/replay")
        })
        .unwrap()
    else {
        panic!("old receipt missing")
    };
    assert_eq!(receipt.state, ReceiptState::Uncertain);
    assert_eq!(receipt.creation.unwrap().resources, resources);
    assert!(matches!(
        reopened.creation_intent(
            &p,
            &request.id,
            CreationStage::Launch,
            StageMetadata::default(),
            11
        ),
        Err(StorageError::Policy(ErrorCode::InvalidRequest))
    ));
}

#[test]
fn revoke_between_confirmed_stages_retains_partial_resources_and_preserves_exact_ready_success() {
    for ready in [false, true] {
        let root = Root::new();
        let path = root.0.join("state.sqlite3");
        let mut store = Storage::open(&path, [7; 32], 0, goatr::pairing::uid()).unwrap();
        let p = peer(&mut store);
        let (request, mut resources) = prepare(&mut store, &p);
        resources.native_id = Some(id("same-native"));
        store
            .creation_result(
                &p,
                &request.id,
                CreationOutcome {
                    resources: resources.clone(),
                    metadata: StageMetadata::default(),
                    state: StageState::Confirmed,
                    definitive: Some(id("exact native launch proof")),
                    at: 5,
                },
            )
            .unwrap();
        if ready {
            store
                .creation_intent(
                    &p,
                    &request.id,
                    CreationStage::Ready,
                    StageMetadata::default(),
                    6,
                )
                .unwrap();
            resources.session_id = Some(Uuid::random());
            resources.generation = Some(Uuid::random());
            store
                .creation_result(
                    &p,
                    &request.id,
                    CreationOutcome {
                        resources: resources.clone(),
                        metadata: StageMetadata::default(),
                        state: StageState::Confirmed,
                        definitive: Some(id("exact ready desktop binding")),
                        at: 7,
                    },
                )
                .unwrap();
        }
        let visible = store.status(&p, &request.id, 8).unwrap().unwrap();
        assert_eq!(visible.state, ReceiptState::Dispatching);
        assert_eq!(visible.creation.unwrap().stage_state, StageState::Confirmed);
        store.revoke_epoch(p.peer_id(), 9).unwrap();
        assert!(matches!(
            store.status(&p, &request.id, 10),
            Err(StorageError::Policy(ErrorCode::Unauthorized))
        ));
        drop(store);
        let db = rusqlite::Connection::open(&path).unwrap();
        let text: String = db
            .query_row(
                "SELECT receipt FROM receipts WHERE action=?1",
                [&request.id.0],
                |r| r.get(0),
            )
            .unwrap();
        let receipt: Receipt = serde_json::from_str(&text).unwrap();
        assert_eq!(receipt.creation.as_ref().unwrap().resources, resources);
        if ready {
            assert_eq!(receipt.state, ReceiptState::Succeeded);
            let Some(ActionResult::Created(created)) = receipt.result else {
                panic!("confirmed Ready must retain the exact created result")
            };
            assert_eq!(Some(created.target.session_id), resources.session_id);
            assert_eq!(Some(created.target.generation), resources.generation);
            assert_eq!(Some(created.workspace_id), resources.workspace_id);
            assert_eq!(Some(created.pane_id), resources.pane_id);
            assert_eq!(Some(created.terminal_id), resources.terminal_id);
            assert_eq!(Some(created.native_id), resources.native_id);
            assert_eq!(created.binding_id, resources.binding_id);
        } else {
            assert_eq!(receipt.state, ReceiptState::Failed);
            assert_eq!(receipt.error.unwrap().code, ErrorCode::Unauthorized);
            assert!(receipt.result.is_none());
        }
    }
}

#[test]
fn settled_definitive_launch_rejection_then_revoke_is_failed_with_resources_and_proof() {
    let root = Root::new();
    let path = root.0.join("state.sqlite3");
    let mut store = Storage::open(&path, [9; 32], 0, goatr::pairing::uid()).unwrap();
    let p = peer(&mut store);
    let (request, resources) = prepare(&mut store, &p);
    let ticket = store.dispatch_ticket(&p, &request.id).unwrap();
    let metadata = StageMetadata {
        pid: Some(std::process::id()),
        pid_start_ticks: Some(
            goatr::herdr::process_evidence(std::process::id())
                .unwrap()
                .start_ticks,
        ),
        ..StageMetadata::default()
    };
    store
        .settle_creation_stage(
            ticket,
            CreationOutcome {
                resources: resources.clone(),
                metadata: metadata.clone(),
                state: StageState::Failed,
                definitive: Some(id("agent_pane_busy")),
                at: 5,
            },
        )
        .unwrap();
    let settled = store.creation_local(&p, &request.id).unwrap().unwrap();
    assert_eq!(settled.stages.last().unwrap().state, StageState::Failed);
    assert_eq!(
        settled.stages.last().unwrap().proof,
        Some(id("agent_pane_busy"))
    );
    store.revoke_epoch(p.peer_id(), 6).unwrap();
    assert!(matches!(
        store.status(&p, &request.id, 7),
        Err(StorageError::Policy(ErrorCode::Unauthorized))
    ));
    let db = rusqlite::Connection::open(&path).unwrap();
    let text: String = db
        .query_row(
            "SELECT receipt FROM receipts WHERE action=?1",
            [&request.id.0],
            |r| r.get(0),
        )
        .unwrap();
    let receipt: Receipt = serde_json::from_str(&text).unwrap();
    let stage_text: String = db
        .query_row(
            "SELECT stages FROM creation WHERE action=?1",
            [&request.id.0],
            |r| r.get(0),
        )
        .unwrap();
    let stages: Vec<CreationStageRecord> = serde_json::from_str(&stage_text).unwrap();
    assert_eq!(stages, settled.stages);
    assert_eq!(stages.last().unwrap().result, Some(metadata));
    assert_eq!(receipt.creation.as_ref().unwrap().resources, resources);
    assert_eq!(
        receipt.creation.as_ref().unwrap().stage_state,
        StageState::Failed
    );
    assert_eq!(
        receipt.state,
        ReceiptState::Failed,
        "definitive rejection was stranded dispatching"
    );
    assert_eq!(
        receipt.error.as_ref().unwrap().code,
        ErrorCode::BackendError
    );
    assert!(receipt.result.is_none());
    drop(db);
    drop(store);
    let reopened = Storage::open(&path, [9; 32], 8, goatr::pairing::uid()).unwrap();
    drop(reopened);
    let db = rusqlite::Connection::open(&path).unwrap();
    let text: String = db
        .query_row(
            "SELECT receipt FROM receipts WHERE action=?1",
            [&request.id.0],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(serde_json::from_str::<Receipt>(&text).unwrap(), receipt);
}

#[test]
fn workspace_probe_is_distinct_once_only_and_gates_every_provider_stage() {
    let mut store = Storage::memory([3; 32], 0).unwrap();
    let p = peer(&mut store);
    let request = Request {
        id: Uuid::random(),
        body: RequestBody::SessionCreate(SessionCreateParams {
            directory_ref: None,
            placement: None,
            boot_id: store.boot_id.clone(),
            server_id: Uuid::random(),
            project_id: Uuid::random(),
            provider: Provider::Omp,
            name: Label::new("probe").unwrap(),
        }),
    };
    store.reserve(&p, &request, 0, |_| Ok(())).unwrap();
    store
        .transition(
            &p,
            &request.id,
            ReceiptTransition {
                state: ReceiptState::Dispatching,
                proof: TransitionProof::Intent,
                result: None,
                error: None,
                at: 1,
            },
        )
        .unwrap();
    store
        .creation_intent(
            &p,
            &request.id,
            CreationStage::Workspace,
            StageMetadata::default(),
            2,
        )
        .unwrap();
    let old_ticket = store.dispatch_ticket(&p, &request.id).unwrap();
    let resources = CreationResources {
        checkout: None,
        workspace_id: Some(id("workspace")),
        pane_id: Some(id("pane")),
        terminal_id: Some(id("terminal")),
        native_id: None,
        binding_id: None,
        session_id: None,
        generation: None,
        owner_started: None,
        relay_started: None,
    };
    store
        .creation_result(
            &p,
            &request.id,
            CreationOutcome {
                resources: resources.clone(),
                metadata: StageMetadata::default(),
                state: StageState::Confirmed,
                definitive: Some(id("exact workspace IDs")),
                at: 3,
            },
        )
        .unwrap();
    assert!(matches!(
        store.creation_intent(
            &p,
            &request.id,
            CreationStage::Relay,
            StageMetadata::default(),
            4
        ),
        Err(StorageError::Policy(ErrorCode::NotReady))
    ));
    assert!(
        store
            .creation_intent(
                &p,
                &request.id,
                CreationStage::Workspace,
                StageMetadata::default(),
                4
            )
            .is_err()
    );
    let metadata = StageMetadata {
        workspace_operation: WorkspaceOperation::ShellProbe,
        shell_probe: Some(ShellProbeMetadata {
            attempt_id: Uuid::random(),
            action_id: request.id.clone(),
            workspace_id: id("workspace"),
            pane_id: id("pane"),
            terminal_id: id("terminal"),
            shell: goatr::herdr::process_evidence(std::process::id()).unwrap(),
            executable: std::env::current_exe().unwrap().to_str().unwrap().into(),
            result_path: "/tmp/owned-ledger-probe".into(),
            result_device: 1,
            result_inode: 1,
            parent_device: 1,
            parent_inode: 1,
            focused_workspace_id: id("desktop"),
        }),
        ..StageMetadata::default()
    };
    let mut wrong = metadata.clone();
    wrong.shell_probe.as_mut().unwrap().pane_id = id("other pane");
    assert!(
        store
            .creation_intent(&p, &request.id, CreationStage::Workspace, wrong, 4)
            .is_err()
    );
    store
        .creation_intent(
            &p,
            &request.id,
            CreationStage::Workspace,
            metadata.clone(),
            4,
        )
        .unwrap();
    assert!(
        store
            .settle_creation_stage(
                old_ticket,
                CreationOutcome {
                    resources: resources.clone(),
                    metadata: StageMetadata::default(),
                    state: StageState::Confirmed,
                    definitive: Some(id("late workspace ACK")),
                    at: 5
                }
            )
            .is_err()
    );
    let probe_ticket = store.dispatch_ticket(&p, &request.id).unwrap();
    store
        .settle_creation_stage(
            probe_ticket,
            CreationOutcome {
                resources: resources.clone(),
                metadata: metadata.clone(),
                state: StageState::Dispatching,
                definitive: Some(id("query enqueue ACK only")),
                at: 5,
            },
        )
        .unwrap();
    assert!(store.creation_probe(&p, &request.id).is_err());
    assert!(
        store
            .creation_intent(
                &p,
                &request.id,
                CreationStage::Relay,
                StageMetadata::default(),
                6
            )
            .is_err()
    );
    store
        .creation_result(
            &p,
            &request.id,
            CreationOutcome {
                resources: resources.clone(),
                metadata: metadata.clone(),
                state: StageState::Confirmed,
                definitive: Some(id("matched original-shell result")),
                at: 6,
            },
        )
        .unwrap();
    assert_eq!(
        store.creation_probe(&p, &request.id).unwrap().pane_id,
        id("pane")
    );
    assert!(
        store
            .creation_intent(&p, &request.id, CreationStage::Workspace, metadata, 7)
            .is_err()
    );
    store
        .creation_intent(
            &p,
            &request.id,
            CreationStage::Relay,
            StageMetadata::default(),
            7,
        )
        .unwrap();
    let local = store.creation_local(&p, &request.id).unwrap().unwrap();
    assert_eq!(
        local
            .stages
            .iter()
            .filter(|s| s.intent.workspace_operation == WorkspaceOperation::ShellProbe)
            .count(),
        1
    );
    let wire = serde_json::to_string(&store.status(&p, &request.id, 7).unwrap().unwrap()).unwrap();
    assert!(!wire.contains("shell_probe") && !wire.contains("owned-ledger-probe"));
}

#[test]
fn admitted_probe_revoke_and_restart_preserve_workspace_and_never_reenqueue_or_install_late_result()
{
    let root = Root::new();
    let path = root.0.join("state.sqlite3");
    let mut store = Storage::open(&path, [29; 32], 0, goatr::pairing::uid()).unwrap();
    let p = peer(&mut store);
    let request = Request {
        id: Uuid::random(),
        body: RequestBody::SessionCreate(SessionCreateParams {
            directory_ref: None,
            placement: None,
            boot_id: store.boot_id.clone(),
            server_id: Uuid::random(),
            project_id: Uuid::random(),
            provider: Provider::Omp,
            name: Label::new("probe").unwrap(),
        }),
    };
    store.reserve(&p, &request, 0, |_| Ok(())).unwrap();
    store
        .transition(
            &p,
            &request.id,
            ReceiptTransition {
                state: ReceiptState::Dispatching,
                proof: TransitionProof::Intent,
                result: None,
                error: None,
                at: 1,
            },
        )
        .unwrap();
    store
        .creation_intent(
            &p,
            &request.id,
            CreationStage::Workspace,
            StageMetadata::default(),
            2,
        )
        .unwrap();
    let resources = CreationResources {
        checkout: None,
        workspace_id: Some(id("workspace")),
        pane_id: Some(id("pane")),
        terminal_id: Some(id("terminal")),
        native_id: None,
        binding_id: None,
        session_id: None,
        generation: None,
        owner_started: None,
        relay_started: None,
    };
    store
        .creation_result(
            &p,
            &request.id,
            CreationOutcome {
                resources: resources.clone(),
                metadata: StageMetadata::default(),
                state: StageState::Confirmed,
                definitive: Some(id("workspace confirmed")),
                at: 3,
            },
        )
        .unwrap();
    let metadata = StageMetadata {
        workspace_operation: WorkspaceOperation::ShellProbe,
        shell_probe: Some(ShellProbeMetadata {
            attempt_id: Uuid::random(),
            action_id: request.id.clone(),
            workspace_id: id("workspace"),
            pane_id: id("pane"),
            terminal_id: id("terminal"),
            shell: goatr::herdr::process_evidence(std::process::id()).unwrap(),
            executable: std::env::current_exe().unwrap().to_str().unwrap().into(),
            result_path: "/tmp/owned-ledger-probe".into(),
            result_device: 1,
            result_inode: 1,
            parent_device: 1,
            parent_inode: 1,
            focused_workspace_id: id("desktop"),
        }),
        ..StageMetadata::default()
    };
    store
        .creation_intent(
            &p,
            &request.id,
            CreationStage::Workspace,
            metadata.clone(),
            4,
        )
        .unwrap();
    let ticket = store.dispatch_ticket(&p, &request.id).unwrap();
    store
        .settle_creation_stage(
            ticket,
            CreationOutcome {
                resources: resources.clone(),
                metadata: metadata.clone(),
                state: StageState::Dispatching,
                definitive: Some(id("queue admission only")),
                at: 5,
            },
        )
        .unwrap();
    store.revoke_epoch(p.peer_id(), 6).unwrap();
    assert!(matches!(
        store.creation_result(
            &p,
            &request.id,
            CreationOutcome {
                resources: resources.clone(),
                metadata: metadata.clone(),
                state: StageState::Confirmed,
                definitive: Some(id("late result")),
                at: 7
            }
        ),
        Err(StorageError::Policy(ErrorCode::Unauthorized))
    ));
    drop(store);
    let reopened = Storage::open(&path, [29; 32], 8, goatr::pairing::uid()).unwrap();
    drop(reopened);
    let db = rusqlite::Connection::open(path).unwrap();
    let wire: String = db
        .query_row(
            "SELECT receipt FROM receipts WHERE action=?1",
            [&request.id.0],
            |r| r.get(0),
        )
        .unwrap();
    let retained: Receipt = serde_json::from_str(&wire).unwrap();
    assert_eq!(retained.state, ReceiptState::Uncertain);
    assert_eq!(retained.creation.unwrap().resources, resources);
    let records: String = db
        .query_row(
            "SELECT stages FROM creation WHERE action=?1",
            [&request.id.0],
            |r| r.get(0),
        )
        .unwrap();
    let records: Vec<CreationStageRecord> = serde_json::from_str(&records).unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(
        records[1].intent.workspace_operation,
        WorkspaceOperation::ShellProbe
    );
    assert_eq!(records[1].state, StageState::Uncertain);
}

#[test]
fn intermediate_codex_services_keep_stage_native_identity_and_never_gain_target() {
    // Ledger-consumer fixtures only: no native owner, relay, shell, or TUI proof.
    for (provider, stage, permitted) in [
        (Provider::Codex, CreationStage::Owner, true),
        (Provider::Codex, CreationStage::Relay, true),
        (Provider::Codex, CreationStage::Thread, false),
        (Provider::Omp, CreationStage::Relay, false),
    ] {
        let mut store = Storage::memory([31; 32], 0).unwrap();
        let p = peer(&mut store);
        let (request, mut resources) = prepare_workspace(&mut store, &p, provider);
        if provider == Provider::Codex {
            for prior in [CreationStage::Owner, CreationStage::Thread] {
                if prior == stage {
                    break;
                }
                if prior == CreationStage::Owner {
                    resources.owner_started = Some(false);
                } else {
                    resources.native_id = Some(id("component-only-original-thread"));
                }
                store
                    .creation_intent(&p, &request.id, prior, StageMetadata::default(), 4)
                    .unwrap();
                store
                    .creation_result(
                        &p,
                        &request.id,
                        CreationOutcome {
                            resources: resources.clone(),
                            metadata: StageMetadata::default(),
                            state: StageState::Confirmed,
                            definitive: Some(id("consumer fixture prior stage")),
                            at: 4,
                        },
                    )
                    .unwrap();
            }
        }
        store
            .creation_intent(&p, &request.id, stage, StageMetadata::default(), 4)
            .unwrap();
        let before = store.creation_local(&p, &request.id).unwrap().unwrap();
        let mut missing_root = resources.clone();
        missing_root.workspace_id = None;
        let mut different_native = resources.clone();
        different_native.native_id = Some(id("not-the-original-thread"));
        let mut premature_target = resources.clone();
        premature_target.session_id = Some(Uuid::random());
        premature_target.generation = Some(Uuid::random());
        for invalid in [missing_root, different_native, premature_target] {
            assert!(matches!(
                store.creation_result(&p, &request.id, ack(invalid)),
                Err(StorageError::Policy(ErrorCode::InvalidRequest))
            ));
            assert_eq!(
                store.creation_local(&p, &request.id).unwrap().unwrap(),
                before,
                "rejected intermediate resources changed the original ledger"
            );
        }
        let result = store.creation_result(&p, &request.id, ack(resources.clone()));
        if permitted {
            result.unwrap();
            let settled = store.creation_local(&p, &request.id).unwrap().unwrap();
            assert_eq!(
                settled.stages.last().unwrap().state,
                StageState::Dispatching
            );
        } else {
            assert!(matches!(
                result,
                Err(StorageError::Policy(ErrorCode::InvalidRequest))
            ));
            assert_eq!(
                store.creation_local(&p, &request.id).unwrap().unwrap(),
                before
            );
        }
        let current = store.status(&p, &request.id, 6).unwrap().unwrap();
        assert_eq!(current.state, ReceiptState::Dispatching);
        assert!(current.target.is_none() && current.result.is_none());
        let progress = current.creation.unwrap();
        assert_eq!(progress.stage, stage);
        assert_eq!(progress.stage_state, StageState::Dispatching);
        assert_eq!(progress.resources, resources);
        for next in [
            CreationStage::Thread,
            CreationStage::Relay,
            CreationStage::Launch,
            CreationStage::Ready,
        ] {
            assert!(matches!(
                store.creation_intent(&p, &request.id, next, StageMetadata::default(), 6,),
                Err(StorageError::Policy(ErrorCode::InvalidRequest))
            ));
        }
    }
}

fn codex_service_admission_revoke_component(stage: CreationStage, revoke_before_settlement: bool) {
    use std::io::Write;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    // These are ledger-consumer prerequisite fixtures, not shell/systemd/native/TUI proof.
    let root = Root::new();
    let path = root.0.join("state.sqlite3");
    let mut store = Storage::open(&path, [37; 32], 0, goatr::pairing::uid()).unwrap();
    let p = peer(&mut store);
    let (request, mut resources) = prepare_workspace(&mut store, &p, Provider::Codex);
    if stage == CreationStage::Relay {
        resources.owner_started = Some(false);
        store
            .creation_intent(
                &p,
                &request.id,
                CreationStage::Owner,
                StageMetadata::default(),
                4,
            )
            .unwrap();
        store
            .creation_result(
                &p,
                &request.id,
                CreationOutcome {
                    resources: resources.clone(),
                    metadata: StageMetadata::default(),
                    state: StageState::Confirmed,
                    definitive: Some(id("component-only reused owner prerequisite")),
                    at: 4,
                },
            )
            .unwrap();
        resources.native_id = Some(id("component-only-original-thread"));
        store
            .creation_intent(
                &p,
                &request.id,
                CreationStage::Thread,
                StageMetadata::default(),
                4,
            )
            .unwrap();
        store
            .creation_result(
                &p,
                &request.id,
                CreationOutcome {
                    resources: resources.clone(),
                    metadata: StageMetadata::default(),
                    state: StageState::Confirmed,
                    definitive: Some(id("component-only original thread prerequisite")),
                    at: 4,
                },
            )
            .unwrap();
    }
    store
        .creation_intent(&p, &request.id, stage, StageMetadata::default(), 4)
        .unwrap();
    let ticket = store.dispatch_ticket(&p, &request.id).unwrap();
    let prior = store.creation_local(&p, &request.id).unwrap().unwrap();
    let admitted_path = root.0.join("exclusive-admitted-resource");
    let mut admitted_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&admitted_path)
        .unwrap();
    let mut effects = 0;
    admitted_file.write_all(b"one admitted original").unwrap();
    admitted_file.sync_all().unwrap();
    effects += 1;
    let object = admitted_file.metadata().unwrap();
    if stage == CreationStage::Relay {
        resources.binding_id = Some(Uuid::random());
    }
    assert!(matches!(
        store.reserve(&p, &request, 5, |_| panic!(
            "matching receipt revalidated effect"
        )),
        Ok(Reservation::Existing(_))
    ));
    let admission_proof = id("exclusive owned file admitted; native readiness unconfirmed");
    if revoke_before_settlement {
        assert!(store.revoke_epoch(p.peer_id(), 5).unwrap());
    }
    store
        .settle_creation_stage(
            ticket,
            CreationOutcome {
                resources: resources.clone(),
                metadata: StageMetadata::default(),
                state: StageState::Dispatching,
                definitive: Some(admission_proof.clone()),
                at: 5,
            },
        )
        .unwrap();
    if !revoke_before_settlement {
        assert!(store.revoke_epoch(p.peer_id(), 6).unwrap());
    }
    assert!(matches!(
        store.reserve(&p, &request, 7, |_| panic!(
            "revoked receipt reentered effect"
        )),
        Err(StorageError::Policy(ErrorCode::Unauthorized))
    ));
    assert!(matches!(
        store.creation_intent(
            &p,
            &request.id,
            CreationStage::Ready,
            StageMetadata::default(),
            7
        ),
        Err(StorageError::Policy(ErrorCode::Unauthorized))
    ));
    assert_eq!(effects, 1);
    let retained_object = fs::metadata(&admitted_path).unwrap();
    assert_eq!(
        (retained_object.dev(), retained_object.ino()),
        (object.dev(), object.ino())
    );
    // Read committed SQLite before reopening; startup recovery must not hide wrong revoke state.
    let db = rusqlite::Connection::open(&path).unwrap();
    let receipt_text: String = db
        .query_row(
            "SELECT receipt FROM receipts WHERE action=?1",
            [&request.id.0],
            |r| r.get(0),
        )
        .unwrap();
    let receipt: Receipt = serde_json::from_str(&receipt_text).unwrap();
    let stages_text: String = db
        .query_row(
            "SELECT stages FROM creation WHERE action=?1",
            [&request.id.0],
            |r| r.get(0),
        )
        .unwrap();
    let records: Vec<CreationStageRecord> = serde_json::from_str(&stages_text).unwrap();
    assert!(receipt.target.is_none() && receipt.result.is_none());
    let progress = receipt.creation.as_ref().unwrap();
    assert_eq!(progress.stage, stage);
    assert_eq!(progress.resources, resources);
    assert!(progress.resources.session_id.is_none() && progress.resources.generation.is_none());
    assert!(progress.resources.relay_started.is_none());
    if stage == CreationStage::Owner {
        assert!(progress.resources.owner_started.is_none());
    }
    let last = records.last().unwrap();
    assert_eq!(records.len(), prior.stages.len());
    assert_eq!(
        &records[..records.len() - 1],
        &prior.stages[..prior.stages.len() - 1]
    );
    assert_eq!(last.proof, Some(admission_proof));
    assert_eq!(last.result, Some(StageMetadata::default()));
    assert_eq!(
        (receipt.state, progress.stage_state, last.state),
        (
            ReceiptState::Uncertain,
            StageState::Uncertain,
            StageState::Uncertain
        ),
        "{stage:?}: revoke-before-settlement={revoke_before_settlement}; real admitted resource must stay owned without inventing no-effect/readiness"
    );
}

#[test]
fn codex_owner_revoke_before_admission_settlement_is_durably_uncertain() {
    codex_service_admission_revoke_component(CreationStage::Owner, true);
}
#[test]
fn codex_owner_revoke_after_admission_settlement_is_durably_uncertain() {
    codex_service_admission_revoke_component(CreationStage::Owner, false);
}
#[test]
fn codex_relay_revoke_before_admission_settlement_is_durably_uncertain() {
    codex_service_admission_revoke_component(CreationStage::Relay, true);
}
#[test]
fn codex_relay_revoke_after_admission_settlement_is_durably_uncertain() {
    codex_service_admission_revoke_component(CreationStage::Relay, false);
}
