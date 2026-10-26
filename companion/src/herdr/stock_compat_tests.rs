// SPDX-License-Identifier: GPL-3.0-or-later
//! Real process/socket admission fixtures; discovery CLI coverage remains in tests/herdr.rs.
use super::*;
use crate::state::{SafeHistory, SessionUpdate};
use std::{os::unix::fs::PermissionsExt, sync::atomic::AtomicUsize};
use tokio::{net::UnixListener, sync::mpsc};

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        let path = PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(format!(".gh-stock-{}", &Uuid::random().0[..8]));
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }
    fn socket(&self) -> PathBuf {
        self.0.join("h.sock")
    }
    fn config(&self) -> Config {
        let executable = fs::canonicalize(std::env::current_exe().unwrap()).unwrap();
        Config::parse(&format!(
            "version=1\nruntime_dir={:?}\nruntime_dir_agent={:?}\nherdr_sockets=[{:?}]\n[executables]\nherdr={:?}\n",
            self.0, self.0, self.socket(), executable
        )).unwrap()
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

// Drive the existing production follower directly; no discovery script impersonates its peer.
pub(crate) async fn run_source(
    owner: Arc<Topology>,
    stop: watch::Receiver<bool>,
) -> io::Result<()> {
    let path = owner.config.herdr_sockets[0].clone();
    let slot = serde_json::to_string(&(&path, "named")).unwrap();
    let key = slot.clone();
    let server = owner
        .storage
        .call(move |store| store.enroll_server(&key, &Label("named".into())))
        .await
        .map_err(store_io)?;
    owner.inner.lock().unwrap().servers.insert(
        server.clone(),
        ServerRecord {
            slot,
            name: Label("named".into()),
            path: path.clone(),
            server_id: server.clone(),
            source: None,
            reason: Some("fixture awaiting original source".into()),
            coarse_shell: None,
        },
    );
    let (refresh, refreshed) = watch::channel(());
    let following = owner.follow(server, path, stop, refreshed);
    tokio::pin!(following);
    let mut cadence = tokio::time::interval(Duration::from_secs(5));
    loop {
        tokio::select! {
            () = &mut following => return Ok(()),
            _ = cadence.tick() => { refresh.send_replace(()); }
        }
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
async fn healthy_prerequisites(config: &Config) {
    let path = &config.herdr_sockets[0];
    Client::connect(path)
        .await
        .unwrap()
        .qualify_executable(&config.executables.herdr)
        .unwrap()
        .request(
            "events.subscribe",
            json!({"subscriptions":[]}),
            "subscription_started",
        )
        .await
        .unwrap();
    Client::connect(path)
        .await
        .unwrap()
        .qualify_executable(&config.executables.herdr)
        .unwrap()
        .snapshot()
        .await
        .unwrap();
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
    let job = tokio::spawn(run_source(owner.clone(), stop_rx));
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
    let job = tokio::spawn(run_source(owner.clone(), stop_rx));
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
            delivery: None,
            native_turn_id: None,
            images: None,
            skill: None,
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
    let job = tokio::spawn(run_source(owner.clone(), stop_rx));
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
            delivery: None,
            native_turn_id: None,
            images: None,
            skill: None,
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
async fn discovery_failure_publishes_unavailable_and_idle_recovery_resubscribes_original_peer() {
    let root = Root::new();
    let listener = UnixListener::bind(root.socket()).unwrap();
    fs::set_permissions(root.socket(), fs::Permissions::from_mode(0o600)).unwrap();
    let subscribed = Arc::new(AtomicUsize::new(0));
    let native = idle_topology_server(listener, subscribed.clone(), topology("workspace"));
    let mut store = Storage::memory([78; 32], 0).unwrap();
    let paired = peer(&mut store);
    let storage = Arc::new(StorageOwner::new(store).unwrap());
    let state = Arc::new(State::default());
    let connection = state.connect(&paired);
    let owner = Topology::new(
        root.config(),
        storage.clone(),
        state.clone(),
        EndpointId(iroh::SecretKey::from_bytes(&[78; 32]).public().to_string()),
    )
    .await
    .unwrap();
    let (stop, stopped) = watch::channel(false);
    let mut jobs = HashMap::new();
    let slots = || {
        vec![DiscoveredSlot {
            name: "named".into(),
            default: false,
            running: true,
            socket_path: root.socket(),
            session_dir: root.0.clone(),
        }]
    };
    owner
        .reconcile_discovery(&mut jobs, &stopped, Ok(slots()))
        .await
        .unwrap();
    let first = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let visible = inventory(&state, &connection);
            if let Some(server) = visible.servers.first()
                && server.connectivity == ServerConnectivity::Online
                && visible
                    .hierarchy
                    .iter()
                    .any(|graph| graph.freshness == NativeFreshness::Current)
            {
                break owner.source(&server.server_id).unwrap();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        first.executable.process,
        process_evidence(std::process::id()).unwrap()
    );
    let subscriptions_before = subscribed.load(Ordering::SeqCst);
    owner
        .reconcile_discovery(&mut jobs, &stopped, Err(invalid("owned discovery failure")))
        .await
        .unwrap();
    assert!(
        jobs.is_empty(),
        "failed discovery must stop the existing follower"
    );
    assert!(first.recheck().is_err());
    assert!(owner.source(&first.server_id).is_err());
    let unavailable = inventory(&state, &connection);
    assert_eq!(
        unavailable.servers[0].connectivity,
        ServerConnectivity::Offline
    );
    assert!(!unavailable.servers[0].can_create.omp && !unavailable.servers[0].can_create.codex);
    assert_ne!(unavailable.hierarchy[0].freshness, NativeFreshness::Current);
    assert!(
        unavailable
            .hierarchy
            .iter()
            .flat_map(|graph| &graph.panes)
            .all(|pane| pane.target.is_none()
                && pane.shell_target.is_none()
                && pane.unbound_row_id.is_none())
    );
    owner
        .reconcile_discovery(&mut jobs, &stopped, Ok(slots()))
        .await
        .unwrap();
    let recovered = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if subscribed.load(Ordering::SeqCst) > subscriptions_before
                && let Ok(source) = owner.source(&first.server_id)
                && inventory(&state, &connection)
                    .hierarchy
                    .iter()
                    .any(|graph| graph.freshness == NativeFreshness::Current)
            {
                break source;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(first.evidence, recovered.evidence);
    assert_eq!(first.incarnation, recovered.incarnation);
    assert!(Arc::ptr_eq(&first.executable, &recovered.executable));
    assert!(!Arc::ptr_eq(&first.invalidation, &recovered.invalidation));
    assert!(first.recheck().is_err());
    assert_eq!(
        inventory(&state, &connection).servers[0].connectivity,
        ServerConnectivity::Online
    );
    stop.send_replace(true);
    for (_, (job, _)) in jobs {
        tokio::time::timeout(Duration::from_secs(5), job)
            .await
            .unwrap()
            .unwrap();
    }
    native.abort();
    let _ = native.await;
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
    let job = tokio::spawn(run_source(owner.clone(), rx));
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

fn executable(name: &str) -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").expect("fixture PATH"))
        .filter_map(|directory| fs::canonicalize(directory.join(name)).ok())
        .find(|path| path.is_file())
        .unwrap_or_else(|| panic!("owned {name} executable fixture is required"))
}
fn process_server_evidence(pid: u32) -> ServerEvidence {
    let process = process_evidence(pid).unwrap();
    ServerEvidence {
        linux_boot_id: process.linux_boot_id,
        pid,
        pid_start_ticks: process.start_ticks,
        socket_device: 0,
        socket_inode: 0,
    }
}
async fn fixture_ready(stdout: &mut BufReader<tokio::process::ChildStdout>) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while read_line(stdout).await.unwrap().as_slice() != b"GOATR_STOCK_READY" {}
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn configured_executable_accepts_exact_peer_and_rejects_other_object_before_setup_reads() {
    let root = Root::new();
    let listener = UnixListener::bind(root.socket()).unwrap();
    fs::set_permissions(root.socket(), fs::Permissions::from_mode(0o600)).unwrap();
    let current = fs::canonicalize(std::env::current_exe().unwrap()).unwrap();
    let client = Client::connect(&root.socket())
        .await
        .unwrap()
        .qualify_executable(&current)
        .unwrap();
    let (_peer, _) = listener.accept().await.unwrap();
    client.recheck().unwrap();
    let pin = InstalledExecutablePin::capture(&current, &client.evidence).unwrap();
    assert!(
        pin.same_identity(&InstalledExecutablePin::capture(&current, &client.evidence).unwrap())
    );

    let other = root.0.join("different-executable");
    fs::write(&other, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&other, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(InstalledExecutablePin::capture(&other, &client.evidence).is_err());
    let unqualified = Client::connect(&root.socket()).await.unwrap();
    let (mut peer, _) = listener.accept().await.unwrap();
    assert!(unqualified.qualify_executable(&other).is_err());
    let mut byte = [0];
    assert_eq!(
        peer.read(&mut byte).await.unwrap(),
        0,
        "setup qualification must precede snapshot consumption"
    );
}

#[tokio::test]
async fn original_executable_pin_rejects_configured_symlink_and_object_replacement() {
    use std::os::unix::fs::symlink;
    let root = Root::new();
    let current = fs::canonicalize(std::env::current_exe().unwrap()).unwrap();
    let configured = root.0.join("profile-herdr");
    symlink(&current, &configured).unwrap();
    let evidence = process_server_evidence(std::process::id());
    let pin = InstalledExecutablePin::capture(&configured, &evidence).unwrap();
    pin.recheck(&evidence).unwrap();
    let other = executable("bash");
    fs::remove_file(&configured).unwrap();
    symlink(&other, &configured).unwrap();
    assert!(pin.recheck(&evidence).is_err());

    let installed = root.0.join("installed");
    fs::copy(&other, &installed).unwrap();
    fs::set_permissions(&installed, fs::Permissions::from_mode(0o700)).unwrap();
    let mut child = tokio::process::Command::new(&installed)
        .arg("-s")
        .env_remove("BASH_ENV")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"printf 'GOATR_STOCK_READY\\n'\n")
        .await
        .unwrap();
    fixture_ready(&mut stdout).await;
    let pid = child.id().unwrap();
    let evidence = process_server_evidence(pid);
    let pin = InstalledExecutablePin::capture(&installed, &evidence).unwrap();
    let replacement = root.0.join("replacement");
    fs::copy(&other, &replacement).unwrap();
    fs::set_permissions(&replacement, fs::Permissions::from_mode(0o700)).unwrap();
    fs::rename(&replacement, &installed).unwrap();
    assert_eq!(fs::canonicalize(&installed).unwrap(), pin.canonical_path);
    assert!(
        pin.recheck(&evidence).is_err(),
        "equal bytes/path cannot replace the pinned inode"
    );
    drop(child.stdin.take());
    assert!(
        tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .unwrap()
            .unwrap()
            .success()
    );
}

#[tokio::test]
async fn original_executable_pin_rejects_peer_exec_with_unchanged_process_epoch() {
    let root = Root::new();
    let shell = executable("bash");
    let current = fs::canonicalize(std::env::current_exe().unwrap()).unwrap();
    let mut child = tokio::process::Command::new(&shell)
        .arg("-s")
        .env_remove("BASH_ENV")
        .env("GOATR_STOCK_SIBLING_FIXTURE", root.socket())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"printf 'GOATR_STOCK_READY\\n'\n")
        .await
        .unwrap();
    fixture_ready(&mut stdout).await;
    let pid = child.id().unwrap();
    let evidence = process_server_evidence(pid);
    let process = process_evidence(pid).unwrap();
    let pin = InstalledExecutablePin::capture(&shell, &evidence).unwrap();
    let command = format!(
        "exec '{}' --exact herdr::stock_compat_tests::owned_sibling_socket_peer --nocapture\n",
        current.to_str().unwrap().replace('\'', "'\\''")
    );
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(command.as_bytes())
        .await
        .unwrap();
    fixture_ready(&mut stdout).await;
    let image = fs::metadata(format!("/proc/{pid}/exe")).unwrap();
    let expected = fs::metadata(&current).unwrap();
    assert_eq!((image.dev(), image.ino()), (expected.dev(), expected.ino()));
    assert_eq!(
        process_evidence(pid).unwrap(),
        process,
        "exec keeps PID/start/namespace evidence"
    );
    assert!(pin.recheck(&evidence).is_err());
    child.stdin.as_mut().unwrap().write_all(&[1]).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .unwrap()
            .unwrap()
            .success()
    );
}

#[tokio::test]
async fn topology_executable_mismatch_never_publishes_and_refresh_retains_original_pin() {
    use std::os::unix::fs::symlink;
    let root = Root::new();
    let listener = UnixListener::bind(root.socket()).unwrap();
    fs::set_permissions(root.socket(), fs::Permissions::from_mode(0o600)).unwrap();
    let current = fs::canonicalize(std::env::current_exe().unwrap()).unwrap();
    let client = Client::connect(&root.socket())
        .await
        .unwrap()
        .qualify_executable(&current)
        .unwrap();
    let evidence = client.evidence.clone();
    let unrelated_pin = client.executable.as_ref().unwrap().clone();
    let (_original_connection, _) = listener.accept().await.unwrap();
    let wrong = root.0.join("wrong-executable");
    fs::write(&wrong, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&wrong, fs::Permissions::from_mode(0o700)).unwrap();
    let configured = root.0.join("profile-herdr");
    symlink(&wrong, &configured).unwrap();
    let mut config = root.config();
    config.executables.herdr = configured.clone();
    let mut store = Storage::memory([77; 32], 0).unwrap();
    let paired = peer(&mut store);
    let slot = serde_json::to_string(&(&root.socket(), "named")).unwrap();
    let server = store.enroll_server(&slot, &Label("named".into())).unwrap();
    let storage = Arc::new(StorageOwner::new(store).unwrap());
    let state = Arc::new(State::default());
    let connection = state.connect(&paired);
    let owner = Topology::new(
        config,
        storage.clone(),
        state.clone(),
        EndpointId(iroh::SecretKey::from_bytes(&[77; 32]).public().to_string()),
    )
    .await
    .unwrap();
    let mut wire = topology("workspace");
    for key in ["workspaces", "tabs", "panes", "agents"] {
        wire[key].as_array_mut().unwrap().truncate(1);
    }
    let snapshot: Snapshot = serde_json::from_value(wire).unwrap();
    snapshot.validate().unwrap();
    let invalidation = Arc::new(AtomicU64::new(0));
    assert_eq!(
        owner
            .apply_snapshot(
                &server,
                (evidence.clone(), unrelated_pin),
                snapshot.clone(),
                0,
                invalidation.clone(),
                false
            )
            .await,
        Err(ErrorCode::NotReady)
    );
    let (_stop, mut stopped) = watch::channel(false);
    let (_refresh, mut refreshed) = watch::channel(());
    assert!(
        owner
            .recover(&server, &root.socket(), &mut stopped, &mut refreshed)
            .await
            .is_err()
    );
    let (mut refused, _) = listener.accept().await.unwrap();
    let mut byte = [0];
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), refused.read(&mut byte))
            .await
            .unwrap()
            .unwrap(),
        0,
        "the production recovery boundary must reject the wrong executable before any native RPC byte"
    );
    assert!(
        owner.inner.lock().unwrap().servers[&server]
            .source
            .is_none()
    );
    assert!(inventory(&state, &connection).hierarchy.is_empty());

    fs::remove_file(&configured).unwrap();
    symlink(
        fs::canonicalize(std::env::current_exe().unwrap()).unwrap(),
        &configured,
    )
    .unwrap();
    let executable = InstalledExecutablePin::capture(&configured, &evidence).unwrap();
    owner
        .apply_snapshot(
            &server,
            (evidence.clone(), executable.clone()),
            snapshot.clone(),
            0,
            invalidation.clone(),
            false,
        )
        .await
        .unwrap();
    let source = owner.source(&server).unwrap();
    let source_client = Client::connect_source(&source).await.unwrap();
    assert!(Arc::ptr_eq(
        source_client.executable.as_ref().unwrap(),
        &source.executable
    ));
    drop(source_client);
    let published = inventory(&state, &connection);
    assert_eq!(
        published.servers[0].connectivity,
        ServerConnectivity::Online
    );
    assert_eq!(published.hierarchy[0].freshness, NativeFreshness::Current);
    let observed = InstalledExecutablePin::capture(&configured, &evidence).unwrap();
    assert!(!Arc::ptr_eq(&source.executable, &observed));
    assert_eq!(
        owner
            .apply_snapshot(
                &server,
                (evidence.clone(), observed),
                snapshot.clone(),
                0,
                invalidation.clone(),
                false
            )
            .await
            .unwrap(),
        (0, false)
    );
    let refreshed = owner.source(&server).unwrap();
    assert!(source.same_physical_identity(&refreshed));
    assert!(Arc::ptr_eq(&source.executable, &refreshed.executable));
    let alias = root.0.join("other-configured-path");
    symlink(&source.executable.canonical_path, &alias).unwrap();
    let mut other = source.clone();
    other.executable = InstalledExecutablePin::capture(&alias, &evidence).unwrap();
    assert!(!source.same_physical_identity(&other));

    fs::remove_file(&configured).unwrap();
    symlink(&wrong, &configured).unwrap();
    assert!(source.recheck_identity().is_err());
    assert!(
        Client::connect_source(&source).await.is_err(),
        "Source clients must retain, not recapture, their original executable"
    );
    assert!(owner.source(&server).is_err());
    assert_eq!(
        owner
            .apply_snapshot(
                &server,
                (evidence, executable),
                snapshot,
                0,
                invalidation,
                true
            )
            .await,
        Err(ErrorCode::NotReady)
    );
    owner.publish().unwrap();
    let lost = inventory(&state, &connection);
    assert_eq!(lost.servers[0].connectivity, ServerConnectivity::Offline);
    assert_ne!(lost.hierarchy[0].freshness, NativeFreshness::Current);
    assert!(
        lost.hierarchy[0]
            .panes
            .iter()
            .all(|pane| pane.target.is_none()
                && pane.shell_target.is_none()
                && pane.unbound_row_id.is_none())
    );
    drop(listener);
    drop(owner);
    Arc::try_unwrap(storage)
        .ok()
        .unwrap()
        .close()
        .await
        .unwrap();
}

/// A subprocess of this same test binary gives a real same-object/different-PID socket peer.
#[test]
fn owned_sibling_socket_peer() {
    let Some(path) = std::env::var_os("GOATR_STOCK_SIBLING_FIXTURE") else {
        return;
    };
    let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    use std::io::Write;
    println!("\nGOATR_STOCK_READY");
    std::io::stdout().flush().unwrap();
    let mut byte = [0];
    std::io::stdin().read_exact(&mut byte).unwrap();
    drop(listener);
}

#[tokio::test]
async fn sibling_socket_requires_same_process_and_retains_own_socket_identity() {
    let (host, phone, security, auth) = super::tests::native_once_authorized().await;
    let fixture = grid::queue_fixture(security.state.clone(), Uuid::random()).await;
    let context = fixture
        .topology()
        .logical_shell_context(fixture.lease.shell().0)
        .unwrap();
    let path = fixture
        .source
        .socket_path
        .with_file_name("native-client.sock");
    let listener = UnixListener::bind(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let session =
        endpoint::Session::connect(&context, Arc::downgrade(&fixture.lease), auth.clone())
            .await
            .unwrap();
    session.recheck().unwrap();
    assert_eq!(session.client.evidence.pid, fixture.source.evidence.pid);
    assert_ne!(
        session.client.evidence.socket_inode,
        fixture.source.evidence.socket_inode
    );
    drop(session);
    drop(listener);
    fs::remove_file(&path).unwrap();

    let mut child = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "herdr::stock_compat_tests::owned_sibling_socket_peer",
            "--nocapture",
        ])
        .env("GOATR_STOCK_SIBLING_FIXTURE", &path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !path.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let candidate = Client::connect(&path).await.unwrap();
    assert_ne!(candidate.evidence.pid, fixture.source.evidence.pid);
    InstalledExecutablePin::capture(
        &fixture.source.executable.configured_path,
        &candidate.evidence,
    )
    .unwrap();
    assert!(
        endpoint::Session::connect(&context, Arc::downgrade(&fixture.lease), auth)
            .await
            .is_err()
    );
    drop(candidate);
    child.stdin.as_mut().unwrap().write_all(&[1]).await.unwrap();
    assert!(child.wait().await.unwrap().success());
    drop(fixture);
    phone.close().await;
    host.close().await;
}

#[tokio::test]
async fn qualified_response_fence_never_turns_source_loss_into_definitive_rejection() {
    use std::os::unix::fs::symlink;
    for mode in [
        "stable_rejection",
        "changed_rejection",
        "changed_success",
        "changed_wrong_id",
        "changed_wrong_type",
    ] {
        let root = Root::new();
        let current = fs::canonicalize(std::env::current_exe().unwrap()).unwrap();
        let configured = root.0.join("profile-herdr");
        symlink(&current, &configured).unwrap();
        let listener = UnixListener::bind(root.socket()).unwrap();
        fs::set_permissions(root.socket(), fs::Permissions::from_mode(0o600)).unwrap();
        let client = Client::connect(&root.socket())
            .await
            .unwrap()
            .qualify_executable(&configured)
            .unwrap();
        let native = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut reader = BufReader::new(stream);
            let request: Value =
                serde_json::from_slice(&read_line(&mut reader).await.unwrap()).unwrap();
            assert_eq!(request["method"], "workspace.create");
            if mode != "stable_rejection" {
                fs::remove_file(&configured).unwrap();
                symlink(executable("bash"), &configured).unwrap();
            }
            let reply = if mode.ends_with("rejection") {
                json!({"id":request["id"],"error":{"code":"invalid_params","message":"no effect"}})
            } else {
                json!({"id":if mode == "changed_wrong_id" { json!("uncorrelated") } else { request["id"].clone() },
                    "result":{"type":if mode == "changed_wrong_type" { "agent_started" } else { "workspace_created" },
                        "workspace":{"workspace_id":"created"}}})
            };
            write(reader.get_mut(), reply).await;
        });
        let response = client
            .request_reported("workspace.create", json!({}), "workspace_created", None)
            .await;
        match (mode, response) {
            ("stable_rejection", Err(ApiError::Rejected(code))) => {
                assert_eq!(code, "invalid_params")
            }
            ("changed_success", Err(ApiError::Changed(value))) => {
                let (facts, _, complete) = workspace_resources(&value, None);
                assert_eq!(facts.workspace_id, Some(Id("created".into())));
                assert!(
                    !complete,
                    "partial success facts are retained, never confirmed"
                );
            }
            (
                "changed_rejection" | "changed_wrong_id" | "changed_wrong_type",
                Err(ApiError::Unavailable(_)),
            ) => {}
            (mode, response) => panic!("{mode} had unsafe classification: {response:?}"),
        }
        native.await.unwrap();
    }
}

#[tokio::test]
async fn subscription_frames_recheck_the_original_executable_pin() {
    use std::os::unix::fs::symlink;
    let root = Root::new();
    let configured = root.0.join("profile-herdr");
    symlink(
        fs::canonicalize(std::env::current_exe().unwrap()).unwrap(),
        &configured,
    )
    .unwrap();
    let listener = UnixListener::bind(root.socket()).unwrap();
    fs::set_permissions(root.socket(), fs::Permissions::from_mode(0o600)).unwrap();
    let client = Client::connect(&root.socket())
        .await
        .unwrap()
        .qualify_executable(&configured)
        .unwrap();
    let original = client.executable.as_ref().unwrap().clone();
    let (release, released) = tokio::sync::oneshot::channel();
    let native = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        let request: Value =
            serde_json::from_slice(&read_line(&mut reader).await.unwrap()).unwrap();
        write(
            reader.get_mut(),
            json!({"id":request["id"],"result":{"type":"subscription_started"}}),
        )
        .await;
        released.await.unwrap();
        write(reader.get_mut(), json!({"event":"pane_updated","data":{}})).await;
    });
    let mut subscription = Subscription::connect(client, &[]).await.unwrap();
    assert!(Arc::ptr_eq(
        subscription.client.executable.as_ref().unwrap(),
        &original
    ));
    fs::remove_file(&configured).unwrap();
    symlink(executable("bash"), &configured).unwrap();
    release.send(()).unwrap();
    assert!(subscription.event().await.is_err());
    native.await.unwrap();
}

#[test]
fn owned_rejection_after_exec_peer() {
    use std::{
        io::{BufRead, Write},
        os::unix::process::CommandExt,
    };
    let Some(path) = std::env::var_os("GOATR_STOCK_EXEC_REPLY_FIXTURE") else {
        return;
    };
    let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    println!("\nGOATR_STOCK_READY");
    std::io::stdout().flush().unwrap();
    let (stream, _) = listener.accept().unwrap();
    let mut reader = std::io::BufReader::new(stream);
    let mut request = String::new();
    reader.read_line(&mut request).unwrap();
    let request: Value = serde_json::from_str(&request).unwrap();
    assert_eq!(request["method"], "workspace.create");
    println!("\nGOATR_STOCK_READY");
    std::io::stdout().flush().unwrap();
    let mut byte = [0];
    std::io::stdin().read_exact(&mut byte).unwrap();
    let response =
        json!({"id":request["id"],"error":{"code":"invalid_params","message":"no effect"}});
    let script = format!(
        "printf '%s\\n' '{}'\nread -r _\n",
        response.to_string().replace('\'', "'\\''")
    );
    let socket: OwnedFd = reader.into_inner().into();
    let error = std::process::Command::new(executable("bash"))
        .args(["-c", &script])
        .env_remove("BASH_ENV")
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::from(socket))
        .exec();
    panic!("owned exec fixture failed: {error}");
}

#[tokio::test]
async fn same_pid_exec_before_rejection_is_unavailable_not_definitive_no_effect() {
    let root = Root::new();
    let current = fs::canonicalize(std::env::current_exe().unwrap()).unwrap();
    let mut child = tokio::process::Command::new(&current)
        .args([
            "--exact",
            "herdr::stock_compat_tests::owned_rejection_after_exec_peer",
            "--nocapture",
        ])
        .env("GOATR_STOCK_EXEC_REPLY_FIXTURE", root.socket())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    fixture_ready(&mut stdout).await;
    let client = Client::connect(&root.socket())
        .await
        .unwrap()
        .qualify_executable(&current)
        .unwrap();
    let evidence = client.evidence.clone();
    let original = client.executable.as_ref().unwrap().clone();
    let process = process_evidence(child.id().unwrap()).unwrap();
    let request = tokio::spawn(client.request("workspace.create", json!({}), "workspace_created"));
    fixture_ready(&mut stdout).await;
    child.stdin.as_mut().unwrap().write_all(&[1]).await.unwrap();
    assert!(matches!(
        request.await.unwrap(),
        Err(ApiError::Unavailable(_))
    ));
    assert_eq!(
        process_evidence(child.id().unwrap()).unwrap(),
        process,
        "the rejection followed exec, not a PID/start/namespace replacement"
    );
    assert!(original.recheck(&evidence).is_err());
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"release\n")
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .unwrap()
            .unwrap()
            .success()
    );
}

#[tokio::test]
async fn recovery_rejects_executable_changes_in_subscription_and_snapshot_responses_before_publication()
 {
    use std::os::unix::fs::symlink;
    for phase in ["subscription", "snapshot"] {
        let root = Root::new();
        let configured = root.0.join("profile-herdr");
        symlink(
            fs::canonicalize(std::env::current_exe().unwrap()).unwrap(),
            &configured,
        )
        .unwrap();
        let mut config = root.config();
        config.executables.herdr = configured.clone();
        let listener = UnixListener::bind(root.socket()).unwrap();
        fs::set_permissions(root.socket(), fs::Permissions::from_mode(0o600)).unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let (release, released) = tokio::sync::oneshot::channel();
        let native = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut subscription = BufReader::new(stream);
            let request: Value =
                serde_json::from_slice(&read_line(&mut subscription).await.unwrap()).unwrap();
            assert_eq!(request["method"], "events.subscribe");
            count.fetch_add(1, Ordering::SeqCst);
            if phase == "subscription" {
                fs::remove_file(&configured).unwrap();
                symlink(executable("bash"), &configured).unwrap();
            }
            write(
                subscription.get_mut(),
                json!({"id":request["id"],"result":{"type":"subscription_started"}}),
            )
            .await;
            if phase == "snapshot" {
                let (stream, _) = listener.accept().await.unwrap();
                let mut snapshot = BufReader::new(stream);
                let request: Value =
                    serde_json::from_slice(&read_line(&mut snapshot).await.unwrap()).unwrap();
                assert_eq!(request["method"], "session.snapshot");
                count.fetch_add(1, Ordering::SeqCst);
                fs::remove_file(&configured).unwrap();
                symlink(executable("bash"), &configured).unwrap();
                write(snapshot.get_mut(), json!({"id":request["id"],"result":{"type":"session_snapshot",
                    "snapshot":{"version":"0.9.1","protocol":22,"workspaces":[],"tabs":[],"panes":[],"agents":[]}}})).await;
            }
            released.await.unwrap();
        });
        let mut store = Storage::memory([79; 32], 0).unwrap();
        let paired = peer(&mut store);
        let slot = serde_json::to_string(&(&root.socket(), "named")).unwrap();
        let server = store.enroll_server(&slot, &Label("named".into())).unwrap();
        let storage = Arc::new(StorageOwner::new(store).unwrap());
        let state = Arc::new(State::default());
        let connection = state.connect(&paired);
        let owner = Topology::new(
            config,
            storage.clone(),
            state.clone(),
            EndpointId(iroh::SecretKey::from_bytes(&[79; 32]).public().to_string()),
        )
        .await
        .unwrap();
        let (_stop, mut stopped) = watch::channel(false);
        let (_refresh, mut refreshed) = watch::channel(());
        assert!(
            tokio::time::timeout(
                Duration::from_secs(5),
                owner.recover(&server, &root.socket(), &mut stopped, &mut refreshed)
            )
            .await
            .unwrap()
            .is_err()
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            if phase == "subscription" { 1 } else { 2 }
        );
        assert!(
            owner.inner.lock().unwrap().servers[&server]
                .source
                .is_none()
        );
        assert!(
            inventory(&state, &connection).hierarchy.is_empty(),
            "an unfenced observation cannot expose native routes"
        );
        release.send(()).unwrap();
        native.await.unwrap();
        drop(owner);
        Arc::try_unwrap(storage)
            .ok()
            .unwrap()
            .close()
            .await
            .unwrap();
    }
}
