// SPDX-License-Identifier: GPL-3.0-or-later
use goatr::{config::Config, herdr::*, protocol::*, storage::*};
use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};
use tokio::{
    io::{AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
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
    assert!(
        !snapshot.pane_grid_subscribe_bound
            && !snapshot.shell_grid_subscribe_bound
            && !snapshot.shell_read_bound
            && !snapshot.artifact_read_bound
    );
    assert!(
        snapshot
            .panes
            .iter()
            .all(|pane| pane.read_identity.is_none())
    );
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
            delivery: None,
            native_turn_id: None,
            images: None,
            skill: None,
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

#[tokio::test]
async fn discovery_failure_then_healthy_cli_recovers_without_replacing_configuration() {
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
    let first = discover(&config).await.unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].socket_path, root.socket());
    assert!(discover(&config).await.is_err());
    let recovered = discover(&config).await.unwrap();
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].socket_path, first[0].socket_path);
    assert_eq!(config.executables.herdr, executable);
    assert_eq!(fs::read_to_string(count).unwrap().trim(), "3");
}

#[tokio::test]
async fn stock_creation_and_rename_acknowledgments_admit_without_private_fields() {
    let root = Root::new();
    let listener = UnixListener::bind(root.socket()).unwrap();
    fs::set_permissions(root.socket(), fs::Permissions::from_mode(0o600)).unwrap();
    let server = tokio::spawn(async move {
        for expected in [
            "workspace.create",
            "tab.create",
            "agent.rename",
            "workspace.rename",
        ] {
            let (stream, _) = listener.accept().await.unwrap();
            let mut reader = BufReader::new(stream);
            let request: Value =
                serde_json::from_slice(&read_line(&mut reader).await.unwrap()).unwrap();
            assert_eq!(request["method"], expected);
            let result = match expected {
                "workspace.create" => {
                    assert_eq!(
                        request["params"],
                        json!({"cwd":"/approved","label":"Created","focus":false})
                    );
                    json!({"type":"workspace_created",
                        "workspace":{"workspace_id":"created","number":2,"label":"Created","focused":false,"active_tab_id":"new-tab","agent_status":"unknown"},
                        "root_pane":{"pane_id":"new-pane","terminal_id":"new-terminal","workspace_id":"created","tab_id":"new-tab","focused":false,"agent_status":"unknown"},
                        "tab":{"tab_id":"new-tab","workspace_id":"created","number":1,"label":"Shell","focused":false,"agent_status":"unknown"}})
                }
                "tab.create" => {
                    assert_eq!(
                        request["params"],
                        json!({"workspace_id":"created","cwd":"/approved","focus":false})
                    );
                    json!({"type":"tab_created",
                        "root_pane":{"pane_id":"tab-pane","terminal_id":"tab-terminal","workspace_id":"created","tab_id":"another-tab","focused":false,"agent_status":"unknown"},
                        "tab":{"tab_id":"another-tab","workspace_id":"created","number":2,"label":"Shell","focused":false,"agent_status":"unknown"}})
                }
                "agent.rename" => {
                    assert_eq!(
                        request["params"],
                        json!({"target":"new-pane","name":"renamed-agent"})
                    );
                    json!({"type":"agent_info","agent":{"pane_id":"new-pane","terminal_id":"new-terminal",
                        "workspace_id":"created","tab_id":"new-tab","agent":"codex","name":"renamed-agent"}})
                }
                "workspace.rename" => {
                    assert_eq!(
                        request["params"],
                        json!({"workspace_id":"created","label":"Renamed workspace"})
                    );
                    json!({"type":"workspace_info","workspace":{"workspace_id":"created","number":2,
                        "label":"Renamed workspace","focused":false,"active_tab_id":"new-tab","agent_status":"unknown"}})
                }
                _ => unreachable!(),
            };
            write(
                reader.get_mut(),
                json!({"id":request["id"],"result":result}),
            )
            .await;
        }
    });
    let created = Client::connect(&root.socket())
        .await
        .unwrap()
        .request(
            "workspace.create",
            json!({"cwd":"/approved","label":"Created","focus":false}),
            "workspace_created",
        )
        .await
        .unwrap();
    assert_eq!(created["workspace"]["workspace_id"], "created");
    assert_eq!(created["root_pane"]["terminal_id"], "new-terminal");
    let tab = Client::connect(&root.socket())
        .await
        .unwrap()
        .request(
            "tab.create",
            json!({"workspace_id":"created","cwd":"/approved","focus":false}),
            "tab_created",
        )
        .await
        .unwrap();
    assert_eq!(tab["root_pane"]["tab_id"], tab["tab"]["tab_id"]);
    let agent = Client::connect(&root.socket())
        .await
        .unwrap()
        .agent_rename(
            &Id("new-pane".into()),
            &AgentName::new("renamed-agent").unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(agent.name.as_deref(), Some("renamed-agent"));
    assert!(!agent.launch_pending && !agent.interactive_ready);
    let workspace = Client::connect(&root.socket())
        .await
        .unwrap()
        .workspace_rename(
            &Id("created".into()),
            &Label::new("Renamed workspace").unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(workspace.label, "Renamed workspace");
    server.await.unwrap();
}
