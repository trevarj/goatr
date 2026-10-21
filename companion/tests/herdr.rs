// SPDX-License-Identifier: GPL-3.0-or-later
use goatr::{
    config::Config,
    herdr::*,
    protocol::*,
    state::{ProviderSnapshot, SafeHistory, SessionUpdate, State},
    storage::*,
};
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
    sync::{mpsc, watch},
};
fn fixture_shell() -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").expect("fixture PATH is required"))
        .filter_map(|directory| fs::canonicalize(directory.join("sh")).ok())
        .find(|path| path.is_file())
        .expect("a real fixture shell interpreter must be available")
}
struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        let path = PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(format!(".gh-{}", &Uuid::random().0[..8]));
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }
    fn socket(&self) -> PathBuf {
        self.0.join("h.sock")
    }
    fn config(&self) -> Config {
        let executable = self.0.join("herdr");
        let slots = json!({"sessions":[{"name":"named","default":false,"running":true,"socket_path":self.socket(),"session_dir":self.0}]});
        fs::write(
            &executable,
            format!(
                "#!{}\nprintf '%s\\n' '{}'\n",
                fixture_shell().display(),
                slots
            ),
        )
        .unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        Config::parse(&format!(
            "version=1\nruntime_dir={:?}\nruntime_dir_agent={:?}\n[executables]\nherdr={:?}\n",
            self.0, self.0, executable
        ))
        .unwrap()
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn topology(workspace: &str) -> Value {
    json!({
        "version":"0.9.1","protocol":22,"focused_workspace_id":workspace,
        "workspaces":[
            {"workspace_id":workspace,"number":7,"label":"Native workspace","focused":true,"active_tab_id":"tab","agent_status":"idle"},
            {"workspace_id":"shell-workspace","number":1,"label":"Shell only","focused":false,"active_tab_id":"shell-tab","agent_status":"unknown"},
            {"workspace_id":"empty-workspace","number":3,"label":"Empty workspace","focused":false,"active_tab_id":"","agent_status":"unknown"},
            {"workspace_id":"unsupported-workspace","number":4,"label":"Unsupported agent","focused":false,"active_tab_id":"unsupported-tab","agent_status":"blocked"}
        ],
        "tabs":[
            {"tab_id":"tab","workspace_id":workspace,"number":2,"label":"Native tab","focused":true,"agent_status":"idle"},
            {"tab_id":"shell-tab","workspace_id":"shell-workspace","number":1,"label":"Development shell","focused":false,"agent_status":"unknown"},
            {"tab_id":"unsupported-tab","workspace_id":"unsupported-workspace","number":5,"label":"Other agent tab","focused":false,"agent_status":"blocked"}
        ],
        "panes":[
            {"pane_id":"pane","terminal_id":"terminal","workspace_id":workspace,"tab_id":"tab","focused":true,"agent_status":"idle","agent":"omp"},
            {"pane_id":"shell-pane","terminal_id":"shell-terminal","workspace_id":"shell-workspace","tab_id":"shell-tab","label":"Local shell","title":"x".repeat(129),"terminal_title_stripped":"Native terminal title","focused":false,"agent_status":"unknown"},
            {"pane_id":"unsupported-pane","terminal_id":"unsupported-terminal","workspace_id":"unsupported-workspace","tab_id":"unsupported-tab","label":"Unsupported pane","agent":"claude","focused":false,"agent_status":"blocked","restore_error":"/private/host/restore/process-report"}
        ],
        "agents":[
            {"pane_id":"pane","terminal_id":"terminal","workspace_id":workspace,"tab_id":"tab","agent":"omp","title":"outside approved projects","agent_session":{"source":"test","agent":"omp","kind":"path","value":"/outside/session.jsonl"}},
            {"pane_id":"unsupported-pane","terminal_id":"unsupported-terminal","workspace_id":"unsupported-workspace","tab_id":"unsupported-tab","agent":"claude","name":"desktop-alias"}
        ]
    })
}
async fn write(stream: &mut UnixStream, value: Value) {
    let mut bytes = serde_json::to_vec(&value).unwrap();
    bytes.push(b'\n');
    stream.write_all(&bytes).await.unwrap();
}
fn peer(storage: &mut Storage) -> AuthenticatedPeer {
    let device = PairedDevice {
        device_id: Uuid::random(),
        endpoint_id: EndpointId(iroh::SecretKey::from_bytes(&[52; 32]).public().to_string()),
        device_name: Label::new("phone").unwrap(),
        paired_at: Uint(0),
        last_seen_at: None,
    };
    let epoch = storage.pair_device(&device).unwrap();
    storage
        .authenticated_hello(&device.device_id, epoch)
        .unwrap()
}
#[tokio::test]
async fn exact_ids_discriminants_duplicates_bounded_reader_namespace_and_socket_replacement() {
    assert!(
        response(
            br#"{"id":"other","result":{"type":"session_snapshot"}}"#,
            "expected",
            "session_snapshot"
        )
        .is_err()
    );
    assert!(
        response(
            br#"{"id":"expected","result":{"type":"agent_started"}}"#,
            "expected",
            "session_snapshot"
        )
        .is_err()
    );
    assert!(
        response(
            br#"{"id":"expected","id":"expected","result":{"type":"session_snapshot"}}"#,
            "expected",
            "session_snapshot"
        )
        .is_err()
    );
    assert!(matches!(
        response(
            br#"{"id":"expected","error":{"code":"invalid_params","message":"no effect"}}"#,
            "expected",
            "session_snapshot"
        ),
        Err(ApiError::Rejected(_))
    ));
    let bytes = vec![b'x'; FRAME_MAX + 1];
    assert!(
        read_line(&mut BufReader::new(bytes.as_slice()))
            .await
            .is_err()
    );
    assert!(read_line(&mut BufReader::new(&b"{}"[..])).await.is_err());
    assert!(process_evidence(0).is_err());
    assert!(process_evidence(u32::MAX).is_err());
    let p = process_evidence(std::process::id()).unwrap();
    assert_eq!(p.pid, std::process::id());
    let fields = format!(
        "{} (a tricky ) name) S {} 73",
        std::process::id(),
        vec!["0"; 18].join(" ")
    );
    assert_eq!(
        process_start_ticks(&fields, std::process::id()).unwrap(),
        73
    );
    assert!(process_start_ticks(&fields, std::process::id() + 1).is_err());
    let status = format!("Name:\tcaller\nNSpid:\t{}\t17\n", std::process::id());
    validate_procfs_view(&fields, &status, std::process::id()).unwrap();
    assert_eq!(
        namespace_pids(&status, std::process::id()).unwrap(),
        vec![std::process::id(), 17]
    );
    assert!(validate_procfs_view(&fields, &status, std::process::id() + 1).is_err());
    assert!(validate_procfs_view(&fields, "NSpid:\t999999\t17\n", std::process::id()).is_err());
    assert!(namespace_pids("NSpid:\t1\t0\n", 1).is_err());
    let root = Root::new();
    let listener = UnixListener::bind(root.socket()).unwrap();
    fs::set_permissions(root.socket(), fs::Permissions::from_mode(0o600)).unwrap();
    let client = Client::connect(&root.socket()).await.unwrap();
    let (_accepted, _) = listener.accept().await.unwrap();
    client.recheck().unwrap();
    fs::remove_file(root.socket()).unwrap();
    let replacement = UnixListener::bind(root.socket()).unwrap();
    fs::set_permissions(root.socket(), fs::Permissions::from_mode(0o600)).unwrap();
    assert!(client.recheck().is_err());
    drop(replacement);
}
#[test]
fn strict_snapshot_routing_empty_focus_and_duplicate_terminal_are_not_proof() {
    let snapshot: Snapshot = serde_json::from_value(topology("workspace")).unwrap();
    snapshot.validate().unwrap();
    snapshot.no_focus().unwrap();
    let mut empty = snapshot.clone();
    empty.focused_workspace_id = None;
    assert!(empty.no_focus().is_err());
    let mut collision = snapshot.clone();
    let mut pane = collision.panes[0].clone();
    pane.pane_id = Id::new("another-pane").unwrap();
    collision.panes.push(pane);
    assert!(collision.validate().is_err());
    let mut wrong = snapshot;
    wrong.agents[0].terminal_id = Id::new("other").unwrap();
    assert!(wrong.validate().is_err());
}
#[tokio::test]
async fn subscribe_before_snapshot_coalesces_dirty_refresh_reconnects_events_lost_and_exposes_unbound()
 {
    let root = Root::new();
    let config = root.config();
    let listener = UnixListener::bind(root.socket()).unwrap();
    fs::set_permissions(root.socket(), fs::Permissions::from_mode(0o600)).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let subscribed = Arc::new(AtomicUsize::new(0));
    let (events, rx) = watch::channel(None::<Value>);
    let (seen, mut seen_rx) = mpsc::channel(32);
    let (release, release_rx) = watch::channel(false);
    let count = calls.clone();
    let active = subscribed.clone();
    let server = tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let mut event_rx = rx.clone();
            let count = count.clone();
            let active = active.clone();
            let seen = seen.clone();
            let mut release = release_rx.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(stream);
                let request: Value =
                    serde_json::from_slice(&read_line(&mut reader).await.unwrap()).unwrap();
                let id = request["id"].clone();
                match request["method"].as_str().unwrap() {
                    "events.subscribe" => {
                        active.fetch_add(1, Ordering::SeqCst);
                        write(
                            reader.get_mut(),
                            json!({"id":id,"result":{"type":"subscription_started"}}),
                        )
                        .await;
                        loop {
                            if event_rx.changed().await.is_err() {
                                break;
                            }
                            let event = event_rx.borrow().clone();
                            if let Some(value) = event
                                && reader
                                    .get_mut()
                                    .write_all(format!("{value}\n").as_bytes())
                                    .await
                                    .is_err()
                            {
                                break;
                            }
                        }
                    }
                    "session.snapshot" => {
                        assert!(active.load(Ordering::SeqCst) > 0);
                        let n = count.fetch_add(1, Ordering::SeqCst) + 1;
                        seen.send(n).await.unwrap();
                        if n == 3 {
                            while !*release.borrow() {
                                release.changed().await.unwrap();
                            }
                        }
                        let ws = if n < 3 { "first" } else { "after-event" };
                        write(reader.get_mut(),json!({"id":id,"result":{"type":"session_snapshot","snapshot":topology(ws)}})).await;
                    }
                    _ => panic!("unexpected public request"),
                }
            });
        }
    });
    let mut storage = Storage::memory([4; 32], 0).unwrap();
    let p = peer(&mut storage);
    let storage = Arc::new(StorageOwner::new(storage).unwrap());
    let state = Arc::new(State::default());
    let connection = state.connect(&p);
    let host = EndpointId(iroh::SecretKey::from_bytes(&[7; 32]).public().to_string());
    let owner = Topology::new(config, storage.clone(), state.clone(), host)
        .await
        .unwrap();
    let (stop, stop_rx) = watch::channel(false);
    let job = tokio::spawn(owner.clone().run(stop_rx));
    tokio::time::timeout(Duration::from_secs(3), async {
        while seen_rx.recv().await.unwrap() < 2 {}
    })
    .await
    .unwrap();
    events
        .send(Some(json!({"event":"pane_updated","data":{}})))
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while seen_rx.recv().await.unwrap() < 3 {}
    })
    .await
    .unwrap();
    events
        .send(Some(json!({"event":"pane_moved","data":{}})))
        .unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    release.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while seen_rx.recv().await.unwrap() < 4 {}
    })
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            state.host_snapshot(&connection, Uuid::random()).unwrap();
            let mut found = false;
            while let (Some(bytes), _) = state.pop_output(&connection).unwrap() {
                if let Envelope::Response(response) =
                    decode_payload(&bytes, Some(Method::HostSnapshot)).unwrap()
                    && let Ok(ResultBody::HostSnapshot(value)) = response.outcome
                    && value
                        .unbound
                        .iter()
                        .any(|r| r.workspace_id.0 == "after-event")
                {
                    assert_eq!(value.unbound.len(), 1);
                    assert!(value.projects.is_empty());
                    assert!(!value.servers[0].can_create.omp);
                    let graph = value
                        .hierarchy
                        .iter()
                        .find(|h| h.server_id == value.servers[0].server_id)
                        .unwrap();
                    assert_eq!(graph.freshness, NativeFreshness::Current);
                    assert_eq!(
                        graph
                            .workspaces
                            .iter()
                            .map(|w| (
                                w.workspace_id.0.as_str(),
                                w.number.0,
                                w.label.as_ref().unwrap().0.as_str()
                            ))
                            .collect::<Vec<_>>(),
                        vec![
                            ("after-event", 7, "Native workspace"),
                            ("shell-workspace", 1, "Shell only"),
                            ("empty-workspace", 3, "Empty workspace"),
                            ("unsupported-workspace", 4, "Unsupported agent")
                        ]
                    );
                    assert_eq!(
                        graph
                            .tabs
                            .iter()
                            .map(|t| (t.tab_id.0.as_str(), t.label.as_ref().unwrap().0.as_str()))
                            .collect::<Vec<_>>(),
                        vec![
                            ("tab", "Native tab"),
                            ("shell-tab", "Development shell"),
                            ("unsupported-tab", "Other agent tab")
                        ]
                    );
                    assert_eq!(
                        graph.panes[0].unbound_row_id,
                        Some(value.unbound[0].row_id.clone())
                    );
                    let shell = graph
                        .panes
                        .iter()
                        .find(|p| p.terminal_id.0 == "shell-terminal")
                        .unwrap();
                    assert!(
                        shell.target.is_none()
                            && shell.unbound_row_id.is_none()
                            && shell.agent.is_none()
                    );
                    assert_eq!(shell.label.as_ref().unwrap().0, "Local shell");
                    assert!(shell.title.is_none());
                    assert_eq!(
                        shell.display_unavailable,
                        Some(NativeDisplayUnavailable::TooLarge)
                    );
                    let unsupported = graph
                        .panes
                        .iter()
                        .find(|p| p.terminal_id.0 == "unsupported-terminal")
                        .unwrap();
                    assert!(unsupported.target.is_none() && unsupported.unbound_row_id.is_none());
                    assert_eq!(unsupported.agent_name.as_ref().unwrap().0, "desktop-alias");
                    assert_eq!(unsupported.agent_status, NativeAgentStatus::Blocked);
                    assert_eq!(
                        unsupported.restore_reason,
                        Some(NativeRestoreReason::RestoreFailed)
                    );
                    assert!(
                        !std::str::from_utf8(&bytes)
                            .unwrap()
                            .contains("/private/host/restore/process-report")
                    );
                    found = true;
                }
            }
            if found {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let before = subscribed.load(Ordering::SeqCst);
    events
        .send(Some(
            json!({"id":"lost","error":{"code":"events_lost","message":"resnapshot"}}),
        ))
        .unwrap();
    tokio::time::timeout(Duration::from_secs(4), async {
        while subscribed.load(Ordering::SeqCst) == before {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    stop.send(true).unwrap();
    job.await.unwrap().unwrap();
    server.abort();
    drop(owner);
    Arc::try_unwrap(storage)
        .ok()
        .unwrap()
        .close()
        .await
        .unwrap();
}

#[tokio::test]
async fn silent_native_report_refresh_is_stable_and_discards_event_races() {
    let root = Root::new();
    let config = root.config();
    let listener = UnixListener::bind(root.socket()).unwrap();
    fs::set_permissions(root.socket(), fs::Permissions::from_mode(0o600)).unwrap();
    let mut pending = topology("workspace");
    pending["agents"][0]["agent_session"] = Value::Null;
    pending["agents"][0]["launch_pending"] = json!(true);
    pending["agents"][0]["interactive_ready"] = json!(false);
    let (current, current_rx) = watch::channel(pending);
    let (events, event_rx) = watch::channel(None::<Value>);
    let (release, release_rx) = watch::channel(false);
    let calls = Arc::new(AtomicUsize::new(0));
    let subscribed = Arc::new(AtomicUsize::new(0));
    let blocked_call = Arc::new(AtomicUsize::new(usize::MAX));
    let (seen, mut seen_rx) = mpsc::channel(32);
    let count = calls.clone();
    let subscriptions = subscribed.clone();
    let blocked = blocked_call.clone();
    let server = tokio::spawn(async move {
        let mut requests = tokio::task::JoinSet::new();
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let mut event_rx = event_rx.clone();
            let current_rx = current_rx.clone();
            let mut release_rx = release_rx.clone();
            let count = count.clone();
            let subscriptions = subscriptions.clone();
            let blocked = blocked.clone();
            let seen = seen.clone();
            requests.spawn(async move {
                let mut reader = BufReader::new(stream);
                let request: Value =
                    serde_json::from_slice(&read_line(&mut reader).await.unwrap()).unwrap();
                match request["method"].as_str().unwrap() {
                    "events.subscribe" => {
                        write(reader.get_mut(), json!({"id":request["id"],"result":{"type":"subscription_started"}})).await;
                        subscriptions.fetch_add(1, Ordering::SeqCst);
                        while event_rx.changed().await.is_ok() {
                            let value = event_rx.borrow_and_update().clone();
                            if let Some(value) = value
                                && reader.get_mut().write_all(format!("{value}\n").as_bytes()).await.is_err()
                            {
                                break;
                            }
                        }
                    }
                    "session.snapshot" => {
                        assert!(subscriptions.load(Ordering::SeqCst) > 0);
                        let snapshot = current_rx.borrow().clone();
                        let n = count.fetch_add(1, Ordering::SeqCst) + 1;
                        seen.send(n).await.unwrap();
                        if n == blocked.load(Ordering::SeqCst) {
                            while !*release_rx.borrow() {
                                release_rx.changed().await.unwrap();
                            }
                        }
                        write(reader.get_mut(), json!({"id":request["id"],"result":{"type":"session_snapshot","snapshot":snapshot}})).await;
                    }
                    _ => panic!("unexpected public request"),
                }
            });
        }
    });
    let mut store = Storage::memory([14; 32], 0).unwrap();
    let paired = peer(&mut store);
    let boot = store.boot_id.clone();
    let storage = Arc::new(StorageOwner::new(store).unwrap());
    let state = Arc::new(State::default());
    let connection = state.connect(&paired);
    let host = EndpointId(iroh::SecretKey::from_bytes(&[14; 32]).public().to_string());
    let owner = Topology::new(config, storage.clone(), state.clone(), host.clone())
        .await
        .unwrap();
    let (stop, stop_rx) = watch::channel(false);
    let job = tokio::spawn(owner.clone().run(stop_rx));
    let first = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(server) = inventory(&state, &connection).servers.first()
                && let Ok(source) = owner.source(&server.server_id)
            {
                break source;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(first.snapshot.agents[0].agent_session.is_none());
    assert!(first.snapshot.agents[0].launch_pending);

    // Native/readiness-only changes deliberately send no Herdr event.
    let mut matured = topology("workspace");
    matured["agents"][0]["launch_pending"] = json!(false);
    matured["agents"][0]["interactive_ready"] = json!(true);
    current.send(matured.clone()).unwrap();
    let mature_source = tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            if let Ok(source) = owner.source(&first.server_id)
                && source.snapshot.agents[0].agent_session.is_some()
                && source.snapshot.agents[0].interactive_ready
            {
                break source;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("existing discovery cadence must refresh a silent native report");
    assert!(!mature_source.snapshot.agents[0].launch_pending);
    assert!(first.recheck().is_err());
    assert_eq!(first.incarnation, mature_source.incarnation);
    assert_eq!(first.evidence, mature_source.evidence);
    let subscriptions_before = subscribed.load(Ordering::SeqCst);
    let native_owner = process_evidence(std::process::id()).unwrap();
    let key = StableSessionKey {
        host_id: host,
        server_id: mature_source.server_id.clone(),
        server_incarnation: mature_source.incarnation.clone(),
        terminal_id: Id::new("terminal").unwrap(),
        provider_owner_identity: Id::new(format!(
            "{}:{}",
            native_owner.pid, native_owner.start_ticks
        ))
        .unwrap(),
        native_conversation_id: Id::new("/outside/session.jsonl").unwrap(),
    };
    let display: SessionState = serde_json::from_value(json!({
        "summary":{"sessionId":Uuid::random(),"generation":Uuid::random(),"serverId":mature_source.server_id,
            "workspaceId":"workspace","terminalId":"terminal","paneId":"pane","provider":"omp",
            "nativeId":"/outside/session.jsonl","title":"retained fixture","status":"idle","connectivity":"ready",
            "capabilities":{"history":true,"prompt":true,"cancel":true,"respond":true,"promptMode":"submit_or_steer","cancelMode":"current_work"},
            "historyCoverage":{"scope":"all_branches","source":"persisted_and_live","olderAvailable":false,"sourceTruncated":false,"desktopBranchKnown":false}},
        "desktopSelection":"unknown","activity":{"state":"idle"}
    })).unwrap();
    let (target, epoch) = owner
        .begin_native(key.clone(), native_owner.clone(), display.clone(), false)
        .await
        .unwrap();
    let mut installed = display;
    installed.summary.session_id = target.session_id.clone();
    installed.summary.generation = target.generation.clone();
    owner.finish_native(&key, &native_owner, &epoch, ProviderSnapshot {
        state: installed,
        history: vec![SafeHistory::from_redacted_display(serde_json::from_value(json!({
            "id":"retained","order":0,"role":"assistant","kind":"message",
            "text":{"kind":"text","text":"retained fixture history"},"source":"live","complete":true,"sourceTruncated":false
        })).unwrap()).unwrap()],
        pending: vec![],
        older_available: false,
        native_older: None,
    }).await.unwrap();
    let prompt = Request {
        id: Uuid::random(),
        body: RequestBody::PromptSubmit(PromptSubmitParams {
            boot_id: boot,
            target: target.clone(),
            text: PromptText::new("fixture admission").unwrap(),
        }),
    };
    state.validate_session_mutation(&prompt).unwrap();
    let retained_history = state
        .history(&paired, &target, None, 100, 0)
        .unwrap()
        .result
        .records;

    let before = calls.load(Ordering::SeqCst);
    tokio::time::timeout(Duration::from_secs(7), async {
        while seen_rx.recv().await.unwrap() <= before {}
    })
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let unchanged = owner.source(&first.server_id).unwrap();
    assert!(Arc::ptr_eq(&mature_source.snapshot, &unchanged.snapshot));
    assert!(mature_source.recheck().is_ok());
    assert_eq!(subscribed.load(Ordering::SeqCst), subscriptions_before);
    state.validate_session_mutation(&prompt).unwrap();
    state.validate_source(&target, &epoch).unwrap();

    // Equal data after an event is still dirty and must be revalidated.
    events
        .send(Some(json!({"event":"pane_updated","data":{}})))
        .unwrap();
    let revalidated = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if unchanged.recheck().is_err()
                && let Ok(source) = owner.source(&first.server_id)
                && !Arc::ptr_eq(&unchanged.snapshot, &source.snapshot)
            {
                break source;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(revalidated.snapshot.as_ref(), unchanged.snapshot.as_ref());
    assert_eq!(revalidated.incarnation, unchanged.incarnation);
    assert_eq!(subscribed.load(Ordering::SeqCst), subscriptions_before);
    state.validate_session_mutation(&prompt).unwrap();
    state.validate_source(&target, &epoch).unwrap();
    assert_eq!(
        state
            .session_state(&target, &epoch)
            .unwrap()
            .summary
            .connectivity,
        SessionConnectivity::Ready
    );
    assert_eq!(
        state
            .history(&paired, &target, None, 100, 0)
            .unwrap()
            .result
            .records,
        retained_history
    );

    let mut raced = matured.clone();
    raced["agents"][0]["title"] = json!("race must not install");
    current.send(raced).unwrap();
    let blocked_number = calls.load(Ordering::SeqCst) + 1;
    blocked_call.store(blocked_number, Ordering::SeqCst);
    tokio::time::timeout(Duration::from_secs(7), async {
        while seen_rx.recv().await.unwrap() < blocked_number {}
    })
    .await
    .unwrap();
    let mut final_snapshot = matured;
    final_snapshot["agents"][0]["title"] = json!("after racing event");
    current.send(final_snapshot).unwrap();
    events
        .send(Some(json!({"event":"pane_updated","data":{}})))
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while revalidated.recheck().is_ok() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    release.send(true).unwrap();
    let recovered = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Ok(source) = owner.source(&first.server_id)
                && source.snapshot.agents[0].title.as_deref() == Some("after racing event")
            {
                break source;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("raced cadence snapshot must be discarded and refetched");
    assert_eq!(recovered.incarnation, revalidated.incarnation);
    assert_eq!(recovered.evidence, revalidated.evidence);
    state.validate_session_mutation(&prompt).unwrap();
    state.validate_source(&target, &epoch).unwrap();
    assert_eq!(
        state
            .history(&paired, &target, None, 100, 0)
            .unwrap()
            .result
            .records,
        retained_history
    );
    stop.send(true).unwrap();
    job.await.unwrap().unwrap();
    server.abort();
    let _ = server.await;
    drop(owner);
    Arc::try_unwrap(storage)
        .ok()
        .unwrap()
        .close()
        .await
        .unwrap();
}

#[test]
fn grouping_is_not_proof_mapping_display_restores_offline_and_retirement_preserves_receipts() {
    let root = Root::new();
    let path = root.0.join("state.sqlite3");
    let mut store = Storage::open(&path, [8; 32], 0, goatr::pairing::uid()).unwrap();
    let server = store
        .enroll_server("logical-slot", &Label::new("named").unwrap())
        .unwrap();
    let grouping = store.persisted_servers().unwrap().pop().unwrap();
    assert_eq!(grouping.server_id, server);
    assert!(grouping.evidence.is_none());
    assert!(grouping.incarnation.is_none());
    let evidence = ServerEvidence {
        linux_boot_id: Uuid::random(),
        pid: 123,
        pid_start_ticks: 72,
        socket_device: 1,
        socket_inode: 2,
    };
    let proven = store.server_identity("logical-slot", &evidence).unwrap();
    assert_eq!(proven.server_id, server);
    let key = StableSessionKey {
        host_id: EndpointId(iroh::SecretKey::from_bytes(&[9; 32]).public().to_string()),
        server_id: server.clone(),
        server_incarnation: proven.incarnation.clone(),
        terminal_id: Id::new("terminal").unwrap(),
        provider_owner_identity: Id::new("boot/process/start/socket").unwrap(),
        native_conversation_id: Id::new("native").unwrap(),
    };
    let target = store.session_identity(&key, false).unwrap();
    let display:SessionState=serde_json::from_value(json!({"summary":{"sessionId":target.session_id,"generation":target.generation,"serverId":server,"workspaceId":"workspace","terminalId":"terminal","paneId":"pane","provider":"omp","nativeId":"native","title":"retained display","status":"idle","connectivity":"ready","capabilities":{"history":true,"prompt":true,"cancel":true,"respond":true,"promptMode":"submit_or_steer","cancelMode":"current_work"},"historyCoverage":{"scope":"all_branches","source":"persisted_and_live","olderAvailable":false,"sourceTruncated":false,"desktopBranchKnown":false}},"desktopSelection":"unknown","activity":{"state":"idle","text":{"kind":"text","text":"must not persist this activity"}}})).unwrap();
    store.remember_session_display(&key, &display).unwrap();
    let p = peer(&mut store);
    let request = Request {
        id: Uuid::random(),
        body: RequestBody::PromptSubmit(PromptSubmitParams {
            boot_id: store.boot_id.clone(),
            target: target.clone(),
            text: PromptText::new("not retained").unwrap(),
        }),
    };
    store.reserve(&p, &request, 0, |_| Ok(())).unwrap();
    assert!(
        !store
            .retire_session_mapping(&target.session_id, false, 1)
            .unwrap()
    );
    drop(store);
    let mut restored = Storage::open(&path, [8; 32], 2, goatr::pairing::uid()).unwrap();
    let persisted = restored.persisted_sessions().unwrap().pop().unwrap();
    assert_eq!(persisted.target.session_id, target.session_id);
    assert_ne!(persisted.target.generation, target.generation);
    assert_eq!(
        persisted.display.as_ref().unwrap().summary.connectivity,
        SessionConnectivity::Offline
    );
    assert!(persisted.display.as_ref().unwrap().activity.text.is_none());
    assert!(
        !restored
            .retire_session_mapping(&target.session_id, true, RETENTION_MS + 3)
            .unwrap()
    );
    assert!(
        restored
            .retire_session_mapping(&target.session_id, false, RETENTION_MS + 3)
            .unwrap()
    );
    assert!(
        restored
            .retire_server_mapping("logical-slot", false, RETENTION_MS + 3)
            .unwrap()
    );
}
#[test]
fn persisted_mapping_capacity_rejects_without_dropping_needed_rows() {
    let mut store = Storage::memory([6; 32], 0).unwrap();
    for n in 0..64 {
        store
            .enroll_server(
                &format!("slot-{n}"),
                &Label::new(format!("server {n}")).unwrap(),
            )
            .unwrap();
    }
    assert!(matches!(
        store.enroll_server("overflow", &Label::new("overflow").unwrap()),
        Err(StorageError::Policy(ErrorCode::ResourceLimit))
    ));
    assert_eq!(store.persisted_servers().unwrap().len(), 64);
    assert!(store.retire_server_mapping("slot-0", false, 0).unwrap());
    store
        .enroll_server("replacement", &Label::new("replacement").unwrap())
        .unwrap();
    let proof = store
        .server_identity(
            "slot-1",
            &ServerEvidence {
                linux_boot_id: Uuid::random(),
                pid: 1,
                pid_start_ticks: 1,
                socket_device: 1,
                socket_inode: 1,
            },
        )
        .unwrap();
    let mut key = StableSessionKey {
        host_id: EndpointId(iroh::SecretKey::from_bytes(&[6; 32]).public().to_string()),
        server_id: proof.server_id,
        server_incarnation: proof.incarnation,
        terminal_id: Id::new("terminal").unwrap(),
        provider_owner_identity: Id::new("owner").unwrap(),
        native_conversation_id: Id::new("native").unwrap(),
    };
    for n in 0..512 {
        key.native_conversation_id = Id::new(format!("native-{n}")).unwrap();
        store.session_identity(&key, false).unwrap();
    }
    key.native_conversation_id = Id::new("overflow").unwrap();
    assert!(matches!(
        store.session_identity(&key, false),
        Err(StorageError::Policy(ErrorCode::ResourceLimit))
    ));
    assert_eq!(store.persisted_sessions().unwrap().len(), 512);
}

#[tokio::test]
async fn response_socket_replacement_preserves_known_creation_ids_as_changed_not_success() {
    let root = Root::new();
    let path = root.socket();
    let listener = UnixListener::bind(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let client = Client::connect(&path).await.unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        let request: Value =
            serde_json::from_slice(&read_line(&mut reader).await.unwrap()).unwrap();
        assert_eq!(request["method"], "workspace.create");
        fs::remove_file(&path).unwrap();
        let replacement = UnixListener::bind(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        write(reader.get_mut(),json!({"id":request["id"],"result":{"type":"workspace_created","workspace":{"workspace_id":"known-workspace"},"root_pane":{"pane_id":"known-pane","terminal_id":"known-terminal","workspace_id":"known-workspace","tab_id":"known-tab"},"tab":{"tab_id":"known-tab","workspace_id":"known-workspace"}}})).await;
        replacement
    });
    let outcome = client
        .request(
            "workspace.create",
            json!({"focus":false}),
            "workspace_created",
        )
        .await;
    let Err(ApiError::Changed(value)) = outcome else {
        panic!("changed source must not discard its known response or report success")
    };
    assert_eq!(
        value.pointer("/workspace/workspace_id").unwrap(),
        "known-workspace"
    );
    assert_eq!(
        value.pointer("/root_pane/terminal_id").unwrap(),
        "known-terminal"
    );
    drop(server.await.unwrap());
}

fn inventory(state: &State, connection: &Uuid) -> HostSnapshotResult {
    let request = Uuid::random();
    state.host_snapshot(connection, request.clone()).unwrap();
    let mut snapshot = None;
    while let (Some(bytes), _) = state.pop_output(connection).unwrap() {
        if let Envelope::Response(response) =
            decode_payload(&bytes, Some(Method::HostSnapshot)).unwrap()
            && response.id == request
        {
            let Ok(ResultBody::HostSnapshot(value)) = response.outcome else {
                panic!("host snapshot response required")
            };
            snapshot = Some(value);
        }
    }
    snapshot.unwrap()
}

fn idle_topology_server(
    listener: UnixListener,
    subscribed: Arc<AtomicUsize>,
    snapshot: Value,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut requests = tokio::task::JoinSet::new();
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let subscribed = subscribed.clone();
            let snapshot = snapshot.clone();
            requests.spawn(async move {
                let mut reader = BufReader::new(stream);
                let request: Value =
                    serde_json::from_slice(&read_line(&mut reader).await.unwrap()).unwrap();
                match request["method"].as_str().unwrap() {
                    "events.subscribe" => {
                        write(
                            reader.get_mut(),
                            json!({"id":request["id"],"result":{"type":"subscription_started"}}),
                        )
                        .await;
                        subscribed.fetch_add(1, Ordering::SeqCst);
                        // Keep the real subscription idle: no topology event wakes recovery.
                        let _ = read_line(&mut reader).await;
                    }
                    "session.snapshot" => {
                        assert!(subscribed.load(Ordering::SeqCst) > 0);
                        write(
                            reader.get_mut(),
                            json!({"id":request["id"],"result":{"type":"session_snapshot","snapshot":snapshot}}),
                        )
                        .await;
                    }
                    _ => panic!("unexpected public request"),
                }
            });
        }
    })
}

async fn healthy_prerequisites(config: &Config) {
    let slots = discover(config)
        .await
        .expect("initial real discovery process must succeed");
    assert_eq!(slots.len(), 1);
    let path = &slots[0].socket_path;
    Client::connect(path)
        .await
        .expect("initial private socket, credentials and procfs evidence must succeed")
        .request(
            "events.subscribe",
            json!({"subscriptions":[]}),
            "subscription_started",
        )
        .await
        .expect("initial subscription_started must be observable");
    Client::connect(path)
        .await
        .expect("initial snapshot socket evidence must succeed")
        .snapshot()
        .await
        .expect("initial authoritative public snapshot must validate");
}

#[tokio::test]
async fn replacement_owner_with_unchanged_herdr_report_offlines_old_authority() {
    let root = Root::new();
    let config = root.config();
    let listener = UnixListener::bind(root.socket()).unwrap();
    fs::set_permissions(root.socket(), fs::Permissions::from_mode(0o600)).unwrap();
    // Retain the old socket object through replacement so the kernel cannot reuse its inode.
    let _socket_inode = std::os::fd::AsFd::as_fd(&listener)
        .try_clone_to_owned()
        .unwrap();
    let mut server = idle_topology_server(
        listener,
        Arc::new(AtomicUsize::new(0)),
        topology("workspace"),
    );
    healthy_prerequisites(&config).await;
    let mut store = Storage::memory([11; 32], 0).unwrap();
    let p = peer(&mut store);
    let boot = store.boot_id.clone();
    let storage = Arc::new(StorageOwner::new(store).unwrap());
    let state = Arc::new(State::default());
    let connection = state.connect(&p);
    let host = EndpointId(iroh::SecretKey::from_bytes(&[11; 32]).public().to_string());
    let owner = Topology::new(config, storage.clone(), state.clone(), host.clone())
        .await
        .unwrap();
    let (stop, stop_rx) = watch::channel(false);
    let job = tokio::spawn(owner.clone().run(stop_rx));
    let source = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let snapshot = inventory(&state, &connection);
            if let Some(server) = snapshot.servers.first()
                && let Ok(source) = owner.source(&server.server_id)
            {
                break source;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|error| {
        panic!(
            "initial source bootstrap failed: {error}; inventory={:?}",
            inventory(&state, &connection)
        )
    });
    let mut old_process = tokio::process::Command::new("sleep")
        .arg("30")
        .kill_on_drop(true)
        .spawn()
        .expect("owned native-owner identity fixture must start");
    let old_evidence = process_evidence(old_process.id().unwrap()).unwrap();
    let key = StableSessionKey {
        host_id: host,
        server_id: source.server_id.clone(),
        server_incarnation: source.incarnation.clone(),
        terminal_id: Id::new("terminal").unwrap(),
        provider_owner_identity: Id::new(format!(
            "{}:{}",
            old_evidence.pid, old_evidence.start_ticks
        ))
        .unwrap(),
        native_conversation_id: Id::new("/outside/session.jsonl").unwrap(),
    };
    let display: SessionState = serde_json::from_value(json!({
        "summary":{
            "sessionId":Uuid::random(),"generation":Uuid::random(),"serverId":source.server_id,
            "workspaceId":"workspace","terminalId":"terminal","paneId":"pane","provider":"omp",
            "nativeId":"/outside/session.jsonl","title":"native binding","status":"idle",
            "connectivity":"ready",
            "capabilities":{"history":true,"prompt":true,"cancel":true,"respond":true,
                "promptMode":"submit_or_steer","cancelMode":"current_work"},
            "historyCoverage":{"scope":"all_branches","source":"persisted_and_live",
                "olderAvailable":false,"sourceTruncated":false,"desktopBranchKnown":false}
        },
        "desktopSelection":"unknown","activity":{"state":"idle"}
    }))
    .unwrap();
    let history = SafeHistory::from_redacted_display(
        serde_json::from_value(json!({
            "id":"retained","order":0,"role":"assistant","kind":"message",
            "text":{"kind":"text","text":"old history"},"source":"live",
            "complete":true,"sourceTruncated":false
        }))
        .unwrap(),
    )
    .unwrap();
    let snapshot = ProviderSnapshot {
        state: display,
        history: vec![history],
        pending: vec![],
        older_available: false,
        native_older: None,
    };
    let (old, epoch) = owner
        .begin_native(
            key.clone(),
            old_evidence.clone(),
            snapshot.state.clone(),
            false,
        )
        .await
        .unwrap();
    assert_eq!(
        inventory(&state, &connection)
            .sessions
            .iter()
            .find(|s| s.session_id == old.session_id)
            .unwrap()
            .connectivity,
        SessionConnectivity::Synchronizing
    );
    let large = state
        .content(&old, Mime::TextPlain, "large".repeat(CONTENT_INLINE_MAX), 0)
        .unwrap();
    let mut old_snapshot = snapshot.clone();
    old_snapshot.state.summary.session_id = old.session_id.clone();
    old_snapshot.state.summary.generation = old.generation.clone();
    old_snapshot.history.push(
        SafeHistory::from_redacted_display(HistoryRecord {
            id: Id("large-original".into()),
            parent_id: None,
            turn_id: None,
            order: Uint(1),
            timestamp: None,
            role: Role::Assistant,
            kind: RecordKind::Message,
            text: Some(large.clone()),
            tool: None,
            original: None,
            source: RecordSource::Persisted,
            complete: true,
            source_truncated: false,
        })
        .unwrap(),
    );
    let stale_snapshot = old_snapshot.clone();
    owner
        .finish_native(&key, &old_evidence, &epoch, old_snapshot)
        .await
        .unwrap();
    let retained_history = state
        .history(&p, &old, None, 100, 0)
        .unwrap()
        .result
        .records;
    let prompt = Request {
        id: Uuid::random(),
        body: RequestBody::PromptSubmit(PromptSubmitParams {
            boot_id: boot,
            target: old.clone(),
            text: PromptText::new("old owner").unwrap(),
        }),
    };
    state.validate_session_mutation(&prompt).unwrap();
    let proof = p.clone();
    let request = prompt.clone();
    let retained = storage
        .call(move |s| {
            s.reserve(&proof, &request, 0, |_| Ok(()))?;
            s.status(&proof, &request.id, 0)
        })
        .await
        .unwrap()
        .unwrap();
    let Content::Ref(content) = state
        .content(&old, Mime::TextPlain, "x".repeat(CONTENT_INLINE_MAX + 1), 0)
        .unwrap()
    else {
        panic!("content handle required")
    };
    let read = ContentReadParams {
        target: old.clone(),
        handle: content.handle,
        offset_bytes: Uint(0),
        limit_bytes: Uint(1),
    };
    state.read_content(&p, &read, 0).unwrap();
    old_process.kill().await.unwrap();
    old_process.wait().await.unwrap();
    let mut replacement_process = tokio::process::Command::new("sleep")
        .arg("30")
        .kill_on_drop(true)
        .spawn()
        .expect("replacement native-owner identity fixture must start");
    let replacement_evidence = process_evidence(replacement_process.id().unwrap()).unwrap();
    let changed = StableSessionKey {
        provider_owner_identity: Id::new(format!(
            "{}:{}",
            replacement_evidence.pid, replacement_evidence.start_ticks
        ))
        .unwrap(),
        ..key.clone()
    };
    let (new, new_epoch) = owner
        .begin_native(
            changed.clone(),
            replacement_evidence.clone(),
            snapshot.state.clone(),
            false,
        )
        .await
        .unwrap();
    assert!(
        owner
            .finish_native(&key, &old_evidence, &epoch, stale_snapshot)
            .await
            .is_err()
    );
    assert!(state.abort_install(&old, &epoch).is_err());
    assert_eq!(
        inventory(&state, &connection)
            .sessions
            .iter()
            .find(|s| s.session_id == new.session_id)
            .unwrap()
            .connectivity,
        SessionConnectivity::Synchronizing
    );
    let mut new_snapshot = snapshot;
    new_snapshot.state.summary.session_id = new.session_id.clone();
    new_snapshot.state.summary.generation = new.generation.clone();
    owner
        .finish_native(&changed, &replacement_evidence, &new_epoch, new_snapshot)
        .await
        .unwrap();
    assert_ne!(old.session_id, new.session_id);
    assert_eq!(
        source.snapshot.agents[0].agent_session,
        owner.source(&source.server_id).unwrap().snapshot.agents[0].agent_session,
    );
    let visible = inventory(&state, &connection);
    let graph = visible
        .hierarchy
        .iter()
        .find(|h| h.server_id == source.server_id)
        .unwrap();
    assert_eq!(graph.server_incarnation, source.incarnation);
    assert_eq!(
        graph
            .panes
            .iter()
            .find(|p| p.terminal_id.0 == "terminal")
            .unwrap()
            .target,
        Some(new.clone())
    );
    assert!(graph.panes.iter().all(|p| p.target.as_ref() != Some(&old)));
    let old_row = visible
        .sessions
        .iter()
        .find(|s| s.session_id == old.session_id)
        .unwrap();
    assert_eq!(old_row.connectivity, SessionConnectivity::Offline);
    assert_eq!(
        visible
            .sessions
            .iter()
            .find(|s| s.session_id == new.session_id)
            .unwrap()
            .connectivity,
        SessionConnectivity::Ready,
    );
    assert_eq!(
        state.validate_session_mutation(&prompt),
        Err(ErrorCode::NotReady)
    );
    assert_eq!(
        state.update_from_source(
            &old,
            &epoch,
            SessionUpdate::Activity(Activity {
                state: ActivityState::Working,
                text: None,
                tool_record_id: None,
            }),
        ),
        Err(ErrorCode::CursorExpired),
    );
    assert_eq!(
        state.read_content(&p, &read, 0).unwrap_err(),
        ErrorCode::ContentExpired
    );
    state.validate_source(&new, &new_epoch).unwrap();
    assert_eq!(
        state
            .history(&p, &old, None, 100, 0)
            .unwrap()
            .result
            .records,
        retained_history,
        "owner replacement must preserve completed history unchanged"
    );
    assert_eq!(
        storage
            .call(|s| s.persisted_sessions())
            .await
            .unwrap()
            .len(),
        2
    );
    let proof = p.clone();
    let action = prompt.id.clone();
    assert_eq!(
        storage
            .call(move |s| s.status(&proof, &action, 0))
            .await
            .unwrap(),
        Some(retained)
    );
    server.abort();
    let _ = server.await;
    fs::remove_file(root.socket()).unwrap();
    let listener = UnixListener::bind(root.socket()).unwrap();
    fs::set_permissions(root.socket(), fs::Permissions::from_mode(0o600)).unwrap();
    server = idle_topology_server(
        listener,
        Arc::new(AtomicUsize::new(0)),
        topology("workspace"),
    );
    let replaced = tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let inventory = inventory(&state, &connection);
            if inventory.hierarchy.iter().any(|h| {
                h.server_id == source.server_id
                    && h.server_incarnation != source.incarnation
                    && h.freshness == NativeFreshness::Current
            }) {
                break inventory;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        replaced
            .sessions
            .iter()
            .filter(|s| s.session_id == old.session_id || s.session_id == new.session_id)
            .all(|s| s.connectivity == SessionConnectivity::Offline)
    );
    let graph = replaced
        .hierarchy
        .iter()
        .find(|h| h.server_id == source.server_id)
        .unwrap();
    let pane = graph
        .panes
        .iter()
        .find(|p| p.terminal_id.0 == "terminal")
        .unwrap();
    assert!(
        pane.target.is_none(),
        "matching native route strings cannot join an old incarnation Target"
    );
    assert_eq!(
        pane.unbound_row_id,
        Some(
            replaced
                .unbound
                .iter()
                .find(|r| r.terminal_id.0 == "terminal")
                .unwrap()
                .row_id
                .clone()
        )
    );
    replacement_process.kill().await.unwrap();
    replacement_process.wait().await.unwrap();
    stop.send(true).unwrap();
    job.await.unwrap().unwrap();
    server.abort();
    let _ = server.await;
    drop(owner);
    Arc::try_unwrap(storage)
        .ok()
        .unwrap()
        .close()
        .await
        .unwrap();
}

#[tokio::test]
async fn discovery_failure_then_idle_healthy_subscription_recovers_without_event() {
    let root = Root::new();
    let config = root.config();
    let executable = config.executables.herdr.clone();
    let count = root.0.join("discovery-count");
    let slots = json!({"sessions":[{"name":"named","default":false,"running":true,
        "socket_path":root.socket(),"session_dir":root.0}]});
    fs::write(
        &executable,
        format!(
            "#!{shell}\n[ \"$*\" = 'session list --json' ] || exit 2\nn=0\n[ ! -f '{count}' ] || read -r n < '{count}'\nn=$((n+1))\nprintf '%s\\n' \"$n\" > '{count}'\n[ \"$n\" -ne 2 ] || exit 1\nprintf '%s\\n' '{slots}'\n",
            count = count.display(),
            shell = fixture_shell().display(),
        ),
    )
    .unwrap();
    let listener = UnixListener::bind(root.socket()).unwrap();
    fs::set_permissions(root.socket(), fs::Permissions::from_mode(0o600)).unwrap();
    let subscribed = Arc::new(AtomicUsize::new(0));
    let server = idle_topology_server(listener, subscribed.clone(), topology("workspace"));
    healthy_prerequisites(&config).await;
    fs::remove_file(&count).unwrap();
    let mut store = Storage::memory([12; 32], 0).unwrap();
    let p = peer(&mut store);
    let storage = Arc::new(StorageOwner::new(store).unwrap());
    let state = Arc::new(State::default());
    let connection = state.connect(&p);
    let host = EndpointId(iroh::SecretKey::from_bytes(&[12; 32]).public().to_string());
    let owner = Topology::new(config, storage.clone(), state.clone(), host)
        .await
        .unwrap();
    let (stop, stop_rx) = watch::channel(false);
    let job = tokio::spawn(owner.clone().run(stop_rx));
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if inventory(&state, &connection)
                .servers
                .first()
                .is_some_and(|s| s.connectivity == ServerConnectivity::Online)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|error| {
        panic!(
            "initial source bootstrap failed: {error}; inventory={:?}",
            inventory(&state, &connection)
        )
    });
    let before = subscribed.load(Ordering::SeqCst);
    tokio::time::timeout(Duration::from_secs(7), async {
        loop {
            if inventory(&state, &connection)
                .servers
                .first()
                .is_some_and(|s| {
                    s.reason.as_deref() == Some("Configured Herdr discovery is unavailable")
                })
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(7), async {
        loop {
            if inventory(&state, &connection)
                .servers
                .first()
                .is_some_and(|s| s.connectivity == ServerConnectivity::Online)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("healthy discovery must recover idle inventory without a topology event");
    assert!(subscribed.load(Ordering::SeqCst) > before);
    assert!(
        fs::read_to_string(count)
            .unwrap()
            .trim()
            .parse::<usize>()
            .unwrap()
            >= 3
    );
    stop.send(true).unwrap();
    job.await.unwrap().unwrap();
    server.abort();
    let _ = server.await;
    drop(owner);
    Arc::try_unwrap(storage)
        .ok()
        .unwrap()
        .close()
        .await
        .unwrap();
}

#[tokio::test]
async fn hierarchy_node_overflow_is_explicit_without_hiding_supported_inventory() {
    let root = Root::new();
    let config = root.config();
    let mut snapshot = topology("workspace");
    for n in 3..510 {
        snapshot["panes"].as_array_mut().unwrap().push(json!({
            "pane_id":format!("shell-pane-{n}"),"terminal_id":format!("shell-terminal-{n}"),
            "workspace_id":"shell-workspace","tab_id":"shell-tab","focused":false,"agent_status":"unknown"
        }));
    }
    let listener = UnixListener::bind(root.socket()).unwrap();
    fs::set_permissions(root.socket(), fs::Permissions::from_mode(0o600)).unwrap();
    let server = idle_topology_server(listener, Arc::new(AtomicUsize::new(0)), snapshot);
    let mut store = Storage::memory([13; 32], 0).unwrap();
    let peer = peer(&mut store);
    let storage = Arc::new(StorageOwner::new(store).unwrap());
    let state = Arc::new(State::default());
    let connection = state.connect(&peer);
    let host = EndpointId(iroh::SecretKey::from_bytes(&[13; 32]).public().to_string());
    let owner = Topology::new(config, storage.clone(), state.clone(), host)
        .await
        .unwrap();
    let (stop, rx) = watch::channel(false);
    let job = tokio::spawn(owner.clone().run(rx));
    let inventory = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let inventory = inventory(&state, &connection);
            if !inventory.hierarchy.is_empty() && !inventory.unbound.is_empty() {
                break inventory;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let graph = &inventory.hierarchy[0];
    assert_eq!(graph.freshness, NativeFreshness::Unavailable);
    assert_eq!(graph.reason, Some(NativeUnavailableReason::TooLarge));
    assert!(graph.workspaces.is_empty() && graph.tabs.is_empty() && graph.panes.is_empty());
    assert_eq!(inventory.unbound[0].terminal_id.0, "terminal");
    assert!(inventory.sessions.is_empty() && inventory.projects.is_empty());
    assert_eq!(
        owner.source(&graph.server_id).unwrap().snapshot.panes.len(),
        510
    );
    stop.send(true).unwrap();
    job.await.unwrap().unwrap();
    server.abort();
    let _ = server.await;
    drop(owner);
    Arc::try_unwrap(storage)
        .ok()
        .unwrap()
        .close()
        .await
        .unwrap();
}
