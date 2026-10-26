// SPDX-License-Identifier: GPL-3.0-or-later
use goatr::{pairing::*, protocol::*, state::State, storage::*};
use std::{
    fs,
    os::unix::{fs::DirBuilderExt, net::UnixStream as StdStream},
    sync::Arc,
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
async fn authorized() -> (
    iroh::Endpoint,
    iroh::Endpoint,
    Arc<Security>,
    Arc<Authorized>,
) {
    let host = goatr::endpoint_builder(iroh::SecretKey::from_bytes(&[31; 32]))
        .relay_mode(iroh::RelayMode::Disabled)
        .clear_address_lookup()
        .bind()
        .await
        .unwrap();
    let phone = goatr::endpoint_builder(iroh::SecretKey::from_bytes(&[32; 32]))
        .relay_mode(iroh::RelayMode::Disabled)
        .clear_address_lookup()
        .bind()
        .await
        .unwrap();
    let mut storage = Storage::memory([9; 32], now_ms()).unwrap();
    let device = PairedDevice {
        device_id: Uuid::random(),
        endpoint_id: EndpointId(phone.id().to_string()),
        device_name: Label::new("phone").unwrap(),
        paired_at: Uint(now_ms()),
        last_seen_at: None,
    };
    storage.pair_device(&device).unwrap();
    let boot = storage.boot_id.clone();
    let security = Security::new(
        StorageOwner::new(storage).unwrap(),
        Arc::new(State::default()),
        EndpointId(host.id().to_string()),
        boot,
        vec![],
    );
    let endpoint = host.clone();
    let incoming = tokio::spawn(async move { endpoint.accept().await.unwrap().await.unwrap() });
    let connection = phone.connect(host.addr(), goatr::ALPN).await.unwrap();
    let server = incoming.await.unwrap();
    assert_eq!(server.remote_id(), phone.id());
    let auth = Arc::new(
        security
            .hello(
                EndpointId(server.remote_id().to_string()),
                HelloParams {
                    app_version: "test".into(),
                    device_name: Label::new("phone").unwrap(),
                },
                server,
            )
            .await
            .unwrap(),
    );
    drop(connection);
    (host, phone, security, auth)
}
fn prompt(boot: Uuid) -> Request {
    Request {
        id: Uuid::random(),
        body: RequestBody::PromptSubmit(PromptSubmitParams { delivery: None, native_turn_id: None, images: None, skill: None, boot_id: boot,
        target: Target {
            session_id: Uuid::random(),
            generation: Uuid::random(),
        },
        text: PromptText("owned socket boundary".into()), }),
    }
}
#[tokio::test]
async fn queued_reserved_write_is_rejected_after_revoke_without_native_bytes() {
    let (host, phone, security, auth) = authorized().await;
    let request = prompt(security.boot.clone());
    assert!(matches!(
        security
            .reserve(&auth, request.clone(), |_| Ok(()), |_| {})
            .await
            .unwrap(),
        Reservation::Reserved(_)
    ));
    let (sender, receiver) = StdStream::pair().unwrap();
    sender.set_nonblocking(true).unwrap();
    receiver.set_nonblocking(true).unwrap();
    let descriptor = sender.try_clone().unwrap();
    let mut sender = tokio::net::UnixStream::from_std(sender).unwrap();
    let mut receiver = tokio::net::UnixStream::from_std(receiver).unwrap();
    security
        .revoke(auth.proof.peer_id().clone(), None)
        .await
        .unwrap();
    let attempted = security
        .boundary(
            &auth,
            request.id,
            || Ok(()),
            descriptor.into(),
            async move {
                sender.write_all(b"must never write").await.unwrap();
                ReceiptTransition {
                    state: ReceiptState::Dispatched,
                    proof: TransitionProof::WriteCompleted,
                    result: None,
                    error: None,
                    at: now_ms(),
                }
            },
        )
        .await;
    assert_eq!(attempted.unwrap_err(), ErrorCode::Unauthorized);
    let mut bytes = [0; 32];
    assert_eq!(receiver.read(&mut bytes).await.unwrap(), 0);
    assert_eq!(
        security.guard(&auth).await.err().unwrap(),
        ErrorCode::Unauthorized
    );
    goatr::close_endpoint(&host).await.unwrap();
    goatr::close_endpoint(&phone).await.unwrap();
}
#[tokio::test]
async fn entered_owned_unix_write_settles_before_revoke_ack_without_full_turn_wait() {
    let (host, phone, security, auth) = authorized().await;
    let request = prompt(security.boot.clone());
    security
        .reserve(&auth, request.clone(), |_| Ok(()), |_| {})
        .await
        .unwrap();
    let (native, other) = StdStream::pair().unwrap();
    native.set_nonblocking(true).unwrap();
    other.set_nonblocking(true).unwrap();
    let invalidator = native.try_clone().unwrap();
    let mut native = tokio::net::UnixStream::from_std(native).unwrap();
    let mut other = tokio::net::UnixStream::from_std(other).unwrap();
    let (entered, ready) = tokio::sync::oneshot::channel();
    let (release, settle) = tokio::sync::oneshot::channel();
    let s = security.clone();
    let a = auth.clone();
    let boundary = tokio::spawn(async move {
        s.boundary(&a, request.id, || Ok(()), invalidator.into(), async move {
            native.write_all(b"native intent").await.unwrap();
            entered.send(()).unwrap();
            settle.await.unwrap();
            ReceiptTransition {
                state: ReceiptState::Dispatched,
                proof: TransitionProof::WriteCompleted,
                result: None,
                error: None,
                at: now_ms(),
            }
        })
        .await
    });
    ready.await.unwrap();
    let mut bytes = [0; 13];
    other.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"native intent");
    let s = security.clone();
    let peer = auth.proof.peer_id().clone();
    let revoke = tokio::spawn(async move { s.revoke(peer, None).await });
    tokio::time::sleep(Duration::from_millis(25)).await;
    assert!(!revoke.is_finished());
    release.send(()).unwrap();
    boundary.await.unwrap().unwrap();
    assert!(revoke.await.unwrap().unwrap());
    // Revocation closes this peer, not the host endpoint or confirmed native effect.
    assert!(!host.is_closed());
    assert!(!phone.is_closed());
    goatr::close_endpoint(&host).await.unwrap();
    goatr::close_endpoint(&phone).await.unwrap();
}
#[tokio::test]
async fn cancelled_hung_boundary_invalidates_actual_descriptor_before_revoke_ack() {
    let (host, phone, security, auth) = authorized().await;
    let request = prompt(security.boot.clone());
    security
        .reserve(&auth, request.clone(), |_| Ok(()), |_| {})
        .await
        .unwrap();
    let (native, _other) = StdStream::pair().unwrap();
    let mut retained = native.try_clone().unwrap();
    let invalidator = native.try_clone().unwrap();
    native.set_nonblocking(true).unwrap();
    let _native = tokio::net::UnixStream::from_std(native).unwrap();
    let (entered, ready) = tokio::sync::oneshot::channel();
    let s = security.clone();
    let a = auth.clone();
    let boundary = tokio::spawn(async move {
        s.boundary(&a, request.id, || Ok(()), invalidator.into(), async move {
            entered.send(()).unwrap();
            std::future::pending::<ReceiptTransition>().await
        })
        .await
    });
    ready.await.unwrap();
    boundary.abort();
    let _ = boundary.await;
    tokio::time::timeout(
        Duration::from_secs(12),
        security.revoke(auth.proof.peer_id().clone(), None),
    )
    .await
    .unwrap()
    .unwrap();
    // Future cancellation alone left this duplicate FD usable; revoke must shut down the socket.
    assert!(std::io::Write::write_all(&mut retained, b"late native write").is_err());
    goatr::close_endpoint(&host).await.unwrap();
    goatr::close_endpoint(&phone).await.unwrap();
}
#[test]
fn sqlite_revoke_atomically_fails_reserved_but_preserves_entered_receipt_and_resources() {
    let root = std::env::temp_dir().join(format!("goatr-fence-{}", Uuid::random().0));
    fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
    let path = root.join("state.sqlite3");
    let mut storage = Storage::open(&path, [8; 32], 0, uid()).unwrap();
    let device = PairedDevice {
        device_id: Uuid::random(),
        endpoint_id: EndpointId(iroh::SecretKey::from_bytes(&[4; 32]).public().to_string()),
        device_name: Label::new("phone").unwrap(),
        paired_at: Uint(0),
        last_seen_at: None,
    };
    let epoch = storage.pair_device(&device).unwrap();
    let proof = storage
        .authenticated_hello(&device.device_id, epoch)
        .unwrap();
    let reserved = prompt(storage.boot_id.clone());
    let entered = prompt(storage.boot_id.clone());
    storage.reserve(&proof, &reserved, 1, |_| Ok(())).unwrap();
    storage.reserve(&proof, &entered, 1, |_| Ok(())).unwrap();
    storage
        .transition(
            &proof,
            &entered.id,
            ReceiptTransition {
                state: ReceiptState::Dispatching,
                proof: TransitionProof::Intent,
                result: None,
                error: None,
                at: 2,
            },
        )
        .unwrap();
    let ticket = storage.dispatch_ticket(&proof, &entered.id).unwrap();
    assert!(storage.revoke_epoch(proof.peer_id(), 3).unwrap());
    let db = rusqlite::Connection::open(&path).unwrap();
    let reserved_json: String = db
        .query_row(
            "SELECT receipt FROM receipts WHERE action=?1",
            [&reserved.id.0],
            |r| r.get(0),
        )
        .unwrap();
    let record: Receipt = serde_json::from_str(&reserved_json).unwrap();
    assert_eq!(record.state, ReceiptState::Failed);
    assert_eq!(record.error.unwrap().code, ErrorCode::Unauthorized);
    assert_eq!(record.updated_at.0, 3);
    let RequestBody::PromptSubmit(original) = &reserved.body else {
        panic!("prompt fixture")
    };
    assert_eq!(record.target, Some(original.target.clone()));
    let state: String = db
        .query_row(
            "SELECT state FROM receipts WHERE action=?1",
            [&entered.id.0],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "dispatching");
    assert!(storage.status(&proof, &entered.id, 4).is_err());
    storage
        .settle_dispatch(
            ticket,
            ReceiptTransition {
                state: ReceiptState::Uncertain,
                proof: TransitionProof::LostOutcome,
                result: None,
                error: None,
                at: 4,
            },
        )
        .unwrap();
    let paired: i64 = db
        .query_row(
            "SELECT paired FROM peers WHERE peer=?1",
            [&proof.peer_id().0],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(paired, 0);
    drop(db);
    drop(storage);
    fs::remove_dir_all(root).unwrap();
}
#[tokio::test]
async fn synchronous_real_process_start_preserves_work_and_fences_every_later_stage() {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let (host, phone, security, auth) = authorized().await;
    let action = Uuid::random();
    let request = Request {
        id: action.clone(),
        body: RequestBody::SessionCreate(SessionCreateParams { directory_ref: None, placement: None, boot_id: security.boot.clone(),
        server_id: Uuid::random(),
        project_id: Uuid::random(),
        provider: Provider::Codex,
        name: Label::new("owned OS boundary fixture").unwrap(), }),
    };
    security
        .reserve(&auth, request, |_| Ok(()), |_| {})
        .await
        .unwrap();
    let root = std::env::temp_dir().join(format!("goatr-start-{}", Uuid::random().0));
    let directory = root.clone();
    let resources = security.start_boundary(&auth, action.clone(), (CreationStage::Workspace, StageMetadata::default()), || { let checked: Result<(), ErrorCode> = (|| Ok(()))(); checked.map_err(Into::into) }, move || {
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .unwrap();
        let marker = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(directory.join("resource"))
            .unwrap();
        marker.sync_all().unwrap();
        let resources = CreationResources { checkout: None,
            workspace_id: Some(Id(format!(
                "directory-inode-{}",
                fs::metadata(&directory).unwrap().ino()
            ))),
            pane_id: Some(Id(format!(
                "file-inode-{}",
                marker.metadata().unwrap().ino()
            ))),
            terminal_id: Some(Id(format!(
                "descriptor-inode-{}",
                marker.metadata().unwrap().ino()
            ))),
            native_id: None,
            binding_id: None,
            session_id: None,
            generation: None,
            owner_started: None,
            relay_started: None,
        };
        Ok((
            resources.clone(),
            CreationOutcome {
                resources,
                metadata: StageMetadata::default(),
                state: StageState::Confirmed,
                definitive: Some(Id("owned directory/file creation completed".into())),
                at: now_ms(),
            },
        ))
    })
        .await
        .unwrap();
    let proof = auth.proof.clone();
    let id = action.clone();
    let retained = resources.clone();
    security
        .storage
        .call(move |s| {
            let metadata = StageMetadata {
                workspace_operation: WorkspaceOperation::ShellProbe,
                shell_probe: Some(ShellProbeMetadata {
                    attempt_id: Uuid::random(),
                    action_id: id.clone(),
                    workspace_id: retained.workspace_id.clone().unwrap(),
                    pane_id: retained.pane_id.clone().unwrap(),
                    terminal_id: retained.terminal_id.clone().unwrap(),
                    shell: goatr::herdr::process_evidence(std::process::id()).unwrap(),
                    executable: std::env::current_exe().unwrap().to_str().unwrap().into(),
                    result_path: "/tmp/owned-OS-boundary-result".into(),
                    result_device: 1,
                    result_inode: 1,
                    parent_device: 1,
                    parent_inode: 1,
                    focused_workspace_id: Id("desktop".into()),
                }),
                ..StageMetadata::default()
            };
            s.creation_intent(
                &proof,
                &id,
                CreationStage::Workspace,
                metadata.clone(),
                now_ms(),
            )?;
            s.creation_result(
                &proof,
                &id,
                CreationOutcome {
                    resources: retained,
                    metadata,
                    state: StageState::Confirmed,
                    definitive: Some(Id("caller matched shell result before owned start".into())),
                    at: now_ms(),
                },
            )
            .map(|_| ())
        })
        .await
        .unwrap();
    let known = resources.clone();
    let mut child = security.start_boundary(&auth, action.clone(), (CreationStage::Owner, StageMetadata::default()), || { let checked: Result<(), ErrorCode> = (|| Ok(()))(); checked.map_err(Into::into) }, move || {
        let child = std::process::Command::new("sleep")
            .arg("30")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        let stat = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        let ticks = stat[stat.rfind(')').unwrap() + 2..]
            .split_whitespace()
            .nth(19)
            .unwrap()
            .parse::<u64>()
            .unwrap();
        let boot = Uuid::new(
            fs::read_to_string("/proc/sys/kernel/random/boot_id")
                .unwrap()
                .trim(),
        )
        .unwrap();
        let resources = CreationResources { checkout: None,
            owner_started: Some(true),
            ..known
        };
        Ok((
            child,
            CreationOutcome {
                resources,
                metadata: StageMetadata {
                    pid: Some(pid),
                    pid_start_ticks: Some(ticks),
                    linux_boot_id: Some(boot),
                    ..StageMetadata::default()
                },
                state: StageState::Confirmed,
                definitive: Some(Id(format!("started-pid-{pid}-{ticks}"))),
                at: now_ms(),
            },
        ))
    })
        .await
        .unwrap();
    let revoked = security
        .revoke(auth.proof.peer_id().clone(), None)
        .await
        .unwrap();
    assert!(revoked);
    assert!(child.try_wait().unwrap().is_none());
    assert!(root.join("resource").is_file());
    for stage in [
        CreationStage::Workspace,
        CreationStage::Owner,
        CreationStage::Thread,
        CreationStage::Relay,
        CreationStage::Launch,
        CreationStage::Ready,
    ] {
        let forbidden = root.join(format!("forbidden-{stage:?}"));
        let attempt = forbidden.clone();
        let preserved = resources.clone();
        let denied = security.start_boundary(&auth, action.clone(), (stage, StageMetadata::default()), || { let checked: Result<(), ErrorCode> = (|| Ok(()))(); checked.map_err(Into::into) }, move || {
            fs::DirBuilder::new().mode(0o700).create(attempt).unwrap();
            Ok((
                (),
                CreationOutcome {
                    resources: preserved,
                    metadata: StageMetadata::default(),
                    state: StageState::Uncertain,
                    definitive: None,
                    at: now_ms(),
                },
            ))
        })
            .await;
        assert_eq!(denied.unwrap_err(), ErrorCode::Unauthorized);
        assert!(!forbidden.exists());
    }
    // Only this fixture's retained Child handle is stopped, never a desktop/provider process.
    child.kill().unwrap();
    child.wait().unwrap();
    fs::remove_dir_all(root).unwrap();
    goatr::close_endpoint(&host).await.unwrap();
    goatr::close_endpoint(&phone).await.unwrap();
}


#[tokio::test]
async fn accepted_native_job_survives_disconnect_replacement_and_client_timeout_without_replay() {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    use goatr::backends::omp::Link;
    let (host, phone, security, auth) = authorized().await;
    let request = prompt(security.boot.clone());
    security
        .reserve(&auth, request.clone(), |_| Ok(()), |_| {})
        .await
        .unwrap();
    security.disconnect(&auth);
    assert_eq!(
        security.guard(&auth).await.err(),
        Some(ErrorCode::Unauthorized)
    );
    drop(security.native_guard(&auth).await.unwrap());
    let listener = host.clone();
    let incoming = tokio::spawn(async move { listener.accept().await.unwrap().await.unwrap() });
    let client = phone.connect(host.addr(), goatr::ALPN).await.unwrap();
    let server = incoming.await.unwrap();
    let replacement = Arc::new(
        security
            .hello(
                EndpointId(server.remote_id().to_string()),
                HelloParams {
                    app_version: "replacement".into(),
                    device_name: Label::new("phone").unwrap(),
                },
                server,
            )
            .await
            .unwrap(),
    );
    assert_eq!(
        security.guard(&auth).await.err(),
        Some(ErrorCode::Unauthorized)
    );
    drop(security.native_guard(&auth).await.unwrap());
    let (native, other) = StdStream::pair().unwrap();
    native.set_nonblocking(true).unwrap();
    other.set_nonblocking(true).unwrap();
    let fd = native.try_clone().unwrap();
    let mut native = tokio::net::UnixStream::from_std(native).unwrap();
    let mut other = tokio::net::UnixStream::from_std(other).unwrap();
    let link = Link::parse(&format!("abcdefghij.{}", URL_SAFE_NO_PAD.encode([17; 48]))).unwrap();
    let payload = link
        .seal(br#"{"t":"prompt","text":"one admitted operation"}"#)
        .unwrap();
    let expected = payload.clone();
    let (entered, entered_rx) = tokio::sync::oneshot::channel();
    let (release, release_rx) = tokio::sync::oneshot::channel();
    let owner = security.clone();
    let original = auth.clone();
    let action = request.id.clone();
    let mut job = tokio::spawn(async move {
        owner
            .boundary(&original, action, || Ok(()), fd.into(), async move {
                entered.send(()).unwrap();
                release_rx.await.unwrap();
                native.write_all(&payload).await.unwrap();
                ReceiptTransition {
                    state: ReceiptState::Dispatched,
                    proof: TransitionProof::WriteCompleted,
                    result: Some(ActionResult::Prompt(ActionResultPrompt {
                        native_turn_id: None,
                    })),
                    error: None,
                    at: now_ms(),
                }
            })
            .await
    });
    entered_rx.await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut job)
            .await
            .is_err()
    );
    let duplicate = security
        .reserve(
            &replacement,
            request.clone(),
            |_| panic!("matching receipt must bypass new validation"),
            |_| {},
        )
        .await
        .unwrap();
    let Reservation::Existing(running) = duplicate else {
        panic!("original action must own the only dispatch")
    };
    assert_eq!(running.state, ReceiptState::Dispatching);
    release.send(()).unwrap();
    job.await.unwrap().unwrap();
    let mut bytes = vec![0; expected.len()];
    other.read_exact(&mut bytes).await.unwrap();
    assert_eq!(bytes, expected);
    assert_eq!(
        link.open(&mut bytes).unwrap(),
        br#"{"t":"prompt","text":"one admitted operation"}"#
    );
    let Reservation::Existing(done) = security
        .reserve(
            &replacement,
            request.clone(),
            |_| panic!("completed action must not replay"),
            |_| {},
        )
        .await
        .unwrap()
    else {
        panic!("receipt must remain")
    };
    assert_eq!(done.state, ReceiptState::Dispatched);
    assert!(
        !serde_json::to_string(&done)
            .unwrap()
            .contains("one admitted operation")
    );
    let mut extra = [0];
    assert_eq!(other.read(&mut extra).await.unwrap(), 0);
    let queued = prompt(security.boot.clone());
    security
        .reserve(&replacement, queued.clone(), |_| Ok(()), |_| {})
        .await
        .unwrap();
    let (native, other) = StdStream::pair().unwrap();
    native.set_nonblocking(true).unwrap();
    other.set_nonblocking(true).unwrap();
    let fd = native.try_clone().unwrap();
    let mut native = tokio::net::UnixStream::from_std(native).unwrap();
    let mut other = tokio::net::UnixStream::from_std(other).unwrap();
    security
        .revoke(replacement.proof.peer_id().clone(), None)
        .await
        .unwrap();
    assert_eq!(
        security.native_guard(&auth).await.err(),
        Some(ErrorCode::Unauthorized)
    );
    assert_eq!(
        security
            .boundary(&replacement, queued.id, || Ok(()), fd.into(), async move {
                native.write_all(b"forbidden").await.unwrap();
                ReceiptTransition {
                    state: ReceiptState::Dispatched,
                    proof: TransitionProof::WriteCompleted,
                    result: None,
                    error: None,
                    at: now_ms(),
                }
            })
            .await,
        Err(ErrorCode::Unauthorized)
    );
    assert_eq!(other.read(&mut extra).await.unwrap(), 0);
    drop(client);
    goatr::close_endpoint(&host).await.unwrap();
    goatr::close_endpoint(&phone).await.unwrap();
}

#[tokio::test]
async fn codex_runtime_matching_receipt_precedes_stale_target_and_registers_no_original() {
    let (host, phone, security, auth) = authorized().await;
    let root = std::env::temp_dir().join(format!("gc-{}", &Uuid::random().0[..8]));
    fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
    let config = goatr::config::Config::parse(&format!(
        "version=1\nruntime_dir={root:?}\nruntime_dir_agent={root:?}\n[executables]\nherdr={:?}\n",
        std::env::current_exe().unwrap()
    ))
    .unwrap();
    let topology = goatr::herdr::Topology::new(
        config,
        security.storage.clone(),
        security.state.clone(),
        security.host.clone(),
    )
    .await
    .unwrap();
    let runtime = goatr::backends::codex::Runtime::new(topology, security.clone());
    let original = prompt(security.boot.clone());
    let reserved = security
        .reserve(&auth, original.clone(), |_| Ok(()), |_| {})
        .await
        .unwrap();
    let Reservation::Reserved(expected) = reserved else {
        panic!()
    };
    assert_eq!(
        runtime
            .clone()
            .mutate(auth.clone(), original.clone())
            .await
            .unwrap(),
        expected
    );
    let mut conflict = original.clone();
    if let RequestBody::PromptSubmit(p) = &mut conflict.body {
        p.text = PromptText("different original".into());
    }
    assert_eq!(
        runtime
            .clone()
            .mutate(auth.clone(), conflict)
            .await
            .unwrap_err(),
        ErrorCode::ActionIdConflict
    );
    let (_, stop) = tokio::sync::watch::channel(true);
    runtime.run(stop).await.unwrap();
    let proof = auth.proof.clone();
    let action = original.id;
    let current = security
        .storage
        .call(move |s| s.status(&proof, &action, now_ms()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current, expected);
    assert_eq!(current.state, ReceiptState::Reserved);
    security.disconnect(&auth);
    goatr::close_endpoint(&host).await.unwrap();
    goatr::close_endpoint(&phone).await.unwrap();
    fs::remove_dir_all(root).unwrap();
}

// Ledger/security component evidence only: prerequisite records below are explicit
// producer fixtures, not shell, systemd, native owner, thread or TUI readiness proof.
// The tested start boundary admits one real exclusive OS file; no provider/unit runs.
async fn codex_admission_before_readiness_component(stage: CreationStage) {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct OwnedRoot(std::path::PathBuf);
    impl Drop for OwnedRoot {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    let (host, phone, security, auth) = authorized().await;
    let root =
        OwnedRoot(std::env::temp_dir().join(format!("gc-admission-{}", &Uuid::random().0[..8])));
    fs::DirBuilder::new().mode(0o700).create(&root.0).unwrap();
    let result_path = root.0.join("probe-component-result");
    let probe_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&result_path)
        .unwrap();
    let result_object = probe_file.metadata().unwrap();
    let parent_object = fs::metadata(&root.0).unwrap();
    let action = Uuid::random();
    let request = Request {
        id: action.clone(),
        body: RequestBody::SessionCreate(SessionCreateParams { directory_ref: None, placement: None, boot_id: security.boot.clone(),
        server_id: Uuid::random(),
        project_id: Uuid::random(),
        provider: Provider::Codex,
        name: Label::new("admission consumer component").unwrap(), }),
    };
    security
        .reserve(&auth, request.clone(), |_| Ok(()), |_| {})
        .await
        .unwrap();
    let resources = CreationResources { checkout: None,
        workspace_id: Some(Id(format!("component-root-{}", parent_object.ino()))),
        pane_id: Some(Id(format!("component-pane-{}", result_object.ino()))),
        terminal_id: Some(Id(format!("component-terminal-{}", result_object.ino()))),
        native_id: None,
        binding_id: None,
        session_id: None,
        generation: None,
        owner_started: None,
        relay_started: None,
    };
    let proof = auth.proof.clone();
    let id = action.clone();
    let prior = resources.clone();
    let probe = StageMetadata {
        workspace_operation: WorkspaceOperation::ShellProbe,
        shell_probe: Some(ShellProbeMetadata {
            attempt_id: Uuid::random(),
            action_id: action.clone(),
            workspace_id: resources.workspace_id.clone().unwrap(),
            pane_id: resources.pane_id.clone().unwrap(),
            terminal_id: resources.terminal_id.clone().unwrap(),
            shell: goatr::herdr::process_evidence(std::process::id()).unwrap(),
            executable: std::env::current_exe().unwrap().to_str().unwrap().into(),
            result_path: result_path.to_str().unwrap().into(),
            result_device: result_object.dev(),
            result_inode: result_object.ino(),
            parent_device: parent_object.dev(),
            parent_inode: parent_object.ino(),
            focused_workspace_id: Id("component-original-focus".into()),
        }),
        ..StageMetadata::default()
    };
    security
        .storage
        .call(move |s| {
            s.transition(
                &proof,
                &id,
                ReceiptTransition {
                    state: ReceiptState::Dispatching,
                    proof: TransitionProof::Intent,
                    result: None,
                    error: None,
                    at: now_ms(),
                },
            )?;
            s.creation_intent(
                &proof,
                &id,
                CreationStage::Workspace,
                StageMetadata::default(),
                now_ms(),
            )?;
            s.creation_result(
                &proof,
                &id,
                CreationOutcome {
                    resources: prior.clone(),
                    metadata: StageMetadata::default(),
                    state: StageState::Confirmed,
                    definitive: Some(Id("component-only workspace prerequisite".into())),
                    at: now_ms(),
                },
            )?;
            s.creation_intent(
                &proof,
                &id,
                CreationStage::Workspace,
                probe.clone(),
                now_ms(),
            )?;
            s.creation_result(
                &proof,
                &id,
                CreationOutcome {
                    resources: prior.clone(),
                    metadata: probe,
                    state: StageState::Confirmed,
                    definitive: Some(Id("component-only matched probe prerequisite".into())),
                    at: now_ms(),
                },
            )?;
            if stage == CreationStage::Relay {
                let mut owner = prior;
                owner.owner_started = Some(false);
                s.creation_intent(
                    &proof,
                    &id,
                    CreationStage::Owner,
                    StageMetadata::default(),
                    now_ms(),
                )?;
                s.creation_result(
                    &proof,
                    &id,
                    CreationOutcome {
                        resources: owner.clone(),
                        metadata: StageMetadata::default(),
                        state: StageState::Confirmed,
                        definitive: Some(Id("component-only reused owner prerequisite".into())),
                        at: now_ms(),
                    },
                )?;
                owner.native_id = Some(Id("component-only-original-thread".into()));
                s.creation_intent(
                    &proof,
                    &id,
                    CreationStage::Thread,
                    StageMetadata::default(),
                    now_ms(),
                )?;
                s.creation_result(
                    &proof,
                    &id,
                    CreationOutcome {
                        resources: owner,
                        metadata: StageMetadata::default(),
                        state: StageState::Confirmed,
                        definitive: Some(Id("component-only thread prerequisite".into())),
                        at: now_ms(),
                    },
                )?;
            }
            Ok(())
        })
        .await
        .unwrap();
    let proof = auth.proof.clone();
    let id = action.clone();
    let before = security
        .storage
        .call(move |s| s.status(&proof, &id, now_ms()))
        .await
        .unwrap()
        .unwrap();
    let mut admitted = before.creation.unwrap().resources;
    if stage == CreationStage::Relay {
        admitted.binding_id = Some(Uuid::random());
    }
    // Dispatching means only admission. Do not set ownerStarted/relayStarted or
    // session/generation as if actual native readiness had already been established.
    let expected = admitted.clone();
    let effects = Arc::new(AtomicUsize::new(0));
    let effects_started = effects.clone();
    let retained = Arc::new(parking_lot::Mutex::new(None::<fs::File>));
    let retained_start = retained.clone();
    let marker_path = root.0.join("admitted-resource");
    let admission_path = marker_path.clone();
    let admission = tokio::time::timeout(
        Duration::from_secs(3),
        security.start_boundary(&auth, action.clone(), (stage, StageMetadata::default()), || { let checked: Result<(), ErrorCode> = (|| Ok(()))(); checked.map_err(Into::into) }, move || {
            let file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&admission_path)
                .unwrap();
            file.sync_all().unwrap();
            effects_started.fetch_add(1, Ordering::SeqCst);
            *retained_start.lock() = Some(file);
            Ok((
                (),
                CreationOutcome {
                    resources: admitted,
                    metadata: StageMetadata::default(),
                    state: StageState::Dispatching,
                    definitive: Some(Id(
                        "exclusive OS admission completed; native readiness unconfirmed".into(),
                    )),
                    at: now_ms(),
                },
            ))
        }),
    )
    .await;
    let proof = auth.proof.clone();
    let id = action.clone();
    let visible = security
        .storage
        .call(move |s| s.status(&proof, &id, now_ms()))
        .await
        .unwrap()
        .unwrap();
    let existing = security
        .reserve(
            &auth,
            request,
            |_| panic!("matching receipt must precede validation"),
            |_| panic!("matching receipt must not register another original"),
        )
        .await
        .unwrap();
    let proof = auth.proof.clone();
    let id = action.clone();
    let next = if stage == CreationStage::Owner {
        CreationStage::Thread
    } else {
        CreationStage::Launch
    };
    let premature = security
        .storage
        .call(move |s| s.creation_intent(&proof, &id, next, StageMetadata::default(), now_ms()))
        .await;
    // Evaluate both bounded cleanup paths before assertions: the current invalid
    // settlement must fail, not leave this regression hanging on a taken ticket.
    let revoked = tokio::time::timeout(
        Duration::from_secs(3),
        security.revoke(auth.proof.peer_id().clone(), None),
    )
    .await;
    let drained = tokio::time::timeout(Duration::from_secs(3), security.shutdown()).await;
    let retained_object = retained.lock().as_ref().unwrap().metadata().unwrap();
    let path_object = fs::metadata(&marker_path).unwrap();
    let resource_survived =
        (retained_object.dev(), retained_object.ino()) == (path_object.dev(), path_object.ino());
    goatr::close_endpoint(&host).await.unwrap();
    goatr::close_endpoint(&phone).await.unwrap();
    assert_eq!(
        effects.load(Ordering::SeqCst),
        1,
        "receipt lookup/drain must never repeat the admitted OS effect"
    );
    assert!(
        resource_survived,
        "revoke/drain must not stop or remove an admitted independent resource"
    );
    assert!(matches!(existing, Reservation::Existing(_)));
    assert!(
        matches!(
            premature,
            Err(StorageError::Policy(ErrorCode::InvalidRequest))
        ),
        "unconfirmed admission must not allow a later stage"
    );
    assert!(
        matches!(admission, Ok(Ok(()))),
        "{stage:?} admission settlement must succeed without inventing readiness: {admission:?}; revoke={revoked:?}; drain={drained:?}"
    );
    assert_eq!(visible.state, ReceiptState::Dispatching);
    let progress = visible.creation.unwrap();
    assert_eq!(progress.stage, stage);
    assert_eq!(progress.stage_state, StageState::Dispatching);
    assert_eq!(progress.resources, expected);
    assert!(progress.resources.session_id.is_none() && progress.resources.generation.is_none());
    assert!(
        matches!(revoked, Ok(Ok(true))),
        "taken admission ticket must finish before revoke acknowledges: {revoked:?}"
    );
    assert!(
        matches!(drained, Ok(Ok(()))),
        "shutdown must drain the settled original boundary: {drained:?}"
    );
}

#[tokio::test]
async fn codex_owner_admission_dispatching_retains_real_resource_and_finishes_revoke_drain_without_ready()
 {
    codex_admission_before_readiness_component(CreationStage::Owner).await;
}

#[tokio::test]
async fn codex_relay_admission_dispatching_retains_binding_and_finishes_revoke_drain_without_ready()
{
    codex_admission_before_readiness_component(CreationStage::Relay).await;
}
