// SPDX-License-Identifier: GPL-3.0-or-later
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use futures_util::{SinkExt, StreamExt};
use goatr::{
    backends::omp::*,
    config::{Config, Executables},
    herdr::process_evidence,
    native_json::{self, Limits, NATIVE},
    protocol::*,
    state::{ProviderSnapshot, SafeHistory, State},
    storage::{AuthenticatedPeer, PairedDevice, Storage},
};
use serde_json::json;
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::PathBuf,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::watch,
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};
struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("go-{}", &Uuid::random().0[..8]));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        Self(path)
    }
    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .unwrap();
        file.write_all(bytes).unwrap();
        path
    }
    fn config(&self) -> Config {
        Config {
            version: 1,
            runtime_dir: self.0.clone(),
            runtime_dir_agent: self.0.clone(),
            allow_scoped_shell_probe: false,
            allow_attachments: false,
            herdr_sockets: vec![],
            push_allowed_origins: vec![],
            omp_registry_dir: None,
            omp_session_roots: vec![self.0.clone()],
            omp_blob_root: None,
            omp_skills_package_root:None,
            executables: Executables {
                herdr: "/not-executed".into(),
                omp: None,
                codex: None,
            },
            projects: vec![],
            codex_owners: vec![],
            default_codex_owner_id: None,
        }
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn state() -> (State, SessionState, Target, Uuid) {
    let state = State::default();
    let value:SessionState=serde_json::from_value(json!({"summary":{"sessionId":Uuid::random(),"generation":Uuid::random(),"serverId":Uuid::random(),"workspaceId":"w","terminalId":"t","paneId":"p","provider":"omp","nativeId":"session","title":"OMP","status":"idle","connectivity":"ready","capabilities":{"history":true,"prompt":true,"cancel":true,"respond":true,"promptMode":"submit_or_steer","cancelMode":"current_work"},"historyCoverage":{"scope":"all_branches","source":"persisted_and_live","olderAvailable":false,"sourceTruncated":false,"desktopBranchKnown":false}},"desktopSelection":"unknown","activity":{"state":"idle"}})).unwrap();
    let target = Target {
        session_id: value.summary.session_id.clone(),
        generation: value.summary.generation.clone(),
    };
    let epoch = state.begin_install(value.clone()).unwrap();
    (state, value, target, epoch)
}
fn peer() -> AuthenticatedPeer {
    let mut store = Storage::memory([1; 32], 0).unwrap();
    let device = PairedDevice {
        device_id: Uuid::random(),
        endpoint_id: EndpointId(iroh::SecretKey::from_bytes(&[4; 32]).public().to_string()),
        device_name: Label::new("source parity").unwrap(),
        paired_at: Uint(0),
        last_seen_at: None,
    };
    let epoch = store.pair_device(&device).unwrap();
    store.authenticated_hello(&device.device_id, epoch).unwrap()
}
fn finish(state: &State, value: SessionState, epoch: &Uuid, history: Vec<SafeHistory>) {
    state
        .finish_install(
            epoch,
            ProviderSnapshot {
                state: value,
                pending: vec![],
                history,
                older_available: false,
                native_older: None,
            },
        )
        .unwrap();
}
#[test]
fn native_literal_maps_exact_numbers_ids_and_phone_grammar_are_independent() {
    let raw=br#"{"id":-0,"arguments":{"cost":1.2300e-5,"large":-18446744073709551617,"$serde_json::private::Number":"1e9","$serde_json::private::RawValue":{"nested":true}}}"#;
    let parsed = native_json::parse(raw, NATIVE).unwrap();
    assert_eq!(parsed["id"].to_string(), "-0");
    assert_eq!(parsed["arguments"]["cost"].to_string(), "1.2300e-5");
    assert_eq!(
        native_json::decimal(&parsed["arguments"]["cost"])
            .unwrap()
            .0,
        "0.0000123"
    );
    assert_eq!(
        parsed["arguments"]["$serde_json::private::Number"].as_str(),
        Some("1e9")
    );
    assert!(parsed["arguments"]["$serde_json::private::RawValue"].is_object());
    let id = native_json::rpc_id(&parsed["id"]).unwrap();
    assert_eq!(serde_json::to_string(&id).unwrap(), "-0");
    assert!(parse_raw_json(raw).is_err());
    assert_eq!(
        native_json::parse(parsed.to_string().as_bytes(), NATIVE).unwrap(),
        parsed
    );
    for input in [
        r#"{"x":{"a":1,"\u0061":2}}"#,
        r#"{"x":"\ud800"}"#,
        "[1e+]",
        "[-01]",
        "[NaN]",
    ] {
        assert!(
            native_json::parse(input.as_bytes(), NATIVE).is_err(),
            "{input}"
        );
    }
    assert!(
        native_json::parse(
            b"[1,2]",
            Limits {
                bytes: 5,
                depth: 1,
                members: 1
            }
        )
        .is_err()
    );
    assert!(
        native_json::parse(
            b"[[0]]",
            Limits {
                bytes: 8,
                depth: 1,
                members: 8
            }
        )
        .is_err()
    );
    assert!(native_json::parse(&[b'"', 255, b'"'], NATIVE).is_err());
}
#[test]
fn stock_link_normalization_secret_length_and_authenticated_envelope_are_exact() {
    let secret = URL_SAFE_NO_PAD.encode([7; 48]);
    let bare = format!("abcdefghij.{secret}");
    for input in [
        bare.clone(),
        format!("  abcdefghij%23{secret}  "),
        format!("https://browser.invalid/#{bare}"),
        format!("wss://browser.invalid/#{bare}"),
    ] {
        let link = Link::parse(&input).unwrap();
        assert_eq!(link.ws_url, "wss://my.omp.sh/r/abcdefghij");
        assert!(link.writable());
    }
    let local = Link::parse(&format!(
        "http://127.0.0.1:1234/r/abcdefghij.{secret}?ignored=true#wrong"
    ))
    .unwrap();
    assert_eq!(local.ws_url, "ws://127.0.0.1:1234/r/abcdefghij");
    assert!(Link::parse(&format!("ws://not-loopback.invalid/r/abcdefghij.{secret}")).is_err());
    // Buffer's unmatched final sextet is accepted when the decoded length is48.
    assert!(
        Link::parse(&format!("abcdefghij.{secret}A"))
            .unwrap()
            .writable()
    );
    let mut envelope = local.seal(br#"{"t":"abort"}"#).unwrap();
    assert_eq!(&envelope[..4], &[0; 4]);
    assert_eq!(envelope.len(), 45);
    envelope[..4].copy_from_slice(&9u32.to_be_bytes());
    assert_eq!(
        local.open(&mut envelope.clone()).unwrap(),
        br#"{"t":"abort"}"#
    );
    *envelope.last_mut().unwrap() ^= 1;
    assert_eq!(
        local.open(&mut envelope).unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert!(local.open(&mut [0; 31]).is_err());
    // NIST CAVS gcmEncryptExtIV256.rsp vector retained by aes-gcm0.10.3.
    fn unhex(s: &str) -> Vec<u8> {
        s.as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
            .collect()
    }
    let key = unhex("b52c505a37d78eda5dd34f20c22540ea1b58963cf8e5bf8ffa85f9f2492505b4");
    let link = Link::parse(&format!("abcdefghij.{}", URL_SAFE_NO_PAD.encode(key))).unwrap();
    let mut vector = vec![0; 4];
    vector.extend(unhex("516c33929df5a3284ff463d7"));
    vector.extend(unhex("bdc1ac884d332457a1d2664f168c76f0"));
    assert!(link.open(&mut vector).unwrap().is_empty());
}
#[test]
fn final_snapshot_is_required_even_for_zero_or_count_complete_transcripts() {
    let welcome = json!({"t":"welcome","proto":3,"header":{"id":"session"},"entryCount":1});
    let mut boundary = SnapshotBoundary::new(&welcome, "session").unwrap();
    assert!(
        !boundary
            .chunk(&json!({"entries":[{"id":"one"}],"final":false}))
            .unwrap()
    );
    assert!(boundary.chunk(&json!({"entries":[],"final":true})).unwrap());
    assert!(boundary.chunk(&json!({"entries":[],"final":true})).is_err());
    let mut zero = SnapshotBoundary::new(
        &json!({"proto":3,"header":{"id":"session"},"entryCount":0}),
        "session",
    )
    .unwrap();
    assert!(zero.chunk(&json!({"entries":[],"final":true})).unwrap());
    assert!(
        SnapshotBoundary::new(
            &json!({"proto":3,"header":{"id":"session"},"entryCount":0,"readOnly":true}),
            "session"
        )
        .is_err()
    );
    assert!(SnapshotBoundary::new(&welcome, "other-session").is_err());
    let mut missing = SnapshotBoundary::new(&welcome, "session").unwrap();
    assert!(missing.chunk(&json!({"entries":[],"final":true})).is_err());
    for read_only in [json!(null), json!("false"), json!(0)] {
        let mut invalid = welcome.clone();
        invalid["readOnly"] = read_only;
        assert!(SnapshotBoundary::new(&invalid, "session").is_err());
    }
}
#[test]
fn marker_help_controls_duplicates_and_native_dialog_cancellation_are_lossless() {
    let (state, _value, target, _epoch) = state();
    let native = json!({"reqId":9007199254740991u64,"kind":"select","title":"Toggle choices","options":["one","one",{"label":"Done","description":"Advance"}],"selectionMarker":"checkbox","checkedIndices":[1,1,0],"markableCount":2,"initialIndex":1,"helpText":"Enter toggles; Done advances"});
    let form = project_form(&state, &target, &native).unwrap();
    let Presentation::OmpSelect(select) = &form.presentation else {
        panic!("offered checkbox card required")
    };
    assert_eq!(select.selection_marker, SelectionMarker::Checkbox);
    assert_eq!(select.checked, vec![Uint(1), Uint(1), Uint(0)]);
    assert_eq!(select.markable_count, Some(Uint(2)));
    assert_eq!(
        select
            .options
            .iter()
            .map(|o| o.value.as_str())
            .collect::<Vec<_>>(),
        vec!["one", "one", "Done"]
    );
    assert!(select.help_text.is_some());
    validate_answer(
        &form.presentation,
        &Answer::OmpSelect(AnswerOmpSelect {
            action: SubmitAction::Submit,
            value: Some("one".into()),
        }),
    )
    .unwrap();
    assert!(
        validate_answer(
            &form.presentation,
            &Answer::OmpSelect(AnswerOmpSelect {
                action: SubmitAction::Submit,
                value: Some("not offered".into())
            })
        )
        .is_err()
    );
    validate_answer(
        &form.presentation,
        &Answer::OmpSelect(AnswerOmpSelect {
            action: SubmitAction::Cancel,
            value: None,
        }),
    )
    .unwrap();
    let mut oversized = native.clone();
    oversized["markableCount"] = json!(4);
    assert!(matches!(
        project_form(&state, &target, &oversized)
            .unwrap()
            .presentation,
        Presentation::DesktopOnly(_)
    ));
    let mut secret = native;
    secret["isSecret"] = json!(true);
    assert!(matches!(
        project_form(&state, &target, &secret).unwrap().presentation,
        Presentation::DesktopOnly(_)
    ));
    let editor = project_form(
        &state,
        &target,
        &json!({"reqId":4,"kind":"editor","title":"Edit","prefill":"original"}),
    )
    .unwrap();
    validate_answer(
        &editor.presentation,
        &Answer::OmpEditor(AnswerOmpEditor {
            action: SubmitAction::Cancel,
            text: None,
        }),
    )
    .unwrap();
}
#[test]
fn native_history_large_content_unknown_entries_and_private_payloads_remain_safe() {
    let (state, value, target, epoch) = state();
    let text = "x".repeat(CONTENT_INLINE_MAX + 17);
    let entry = json!({"id":"message","parentId":"branch","type":"message","timestamp":"2026-10-07T00:00:00.000Z","message":{"role":"assistant","content":[{"type":"text","text":text},{"type":"redactedThinking","data":"NEVER-SIGNED"},{"type":"toolCall","id":"call","name":"read","arguments":{"n":2,"apiKey":"NEVER-KEY"}}],"signature":"NEVER-SIGNATURE"}});
    let records = project_entry(&state, &target, &entry, RecordSource::Persisted, 0, None).unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].record().parent_id, Some(Id("branch".into())));
    assert_eq!(
        records[1].record().tool.as_ref().unwrap().call_id,
        Some(Id("call".into()))
    );
    assert!(matches!(records[0].record().text, Some(Content::Ref(_))));
    let original = records[0].record().original.clone().unwrap();
    finish(&state, value, &epoch, records);
    let Content::Ref(content) = original else {
        panic!("large original must use real content cache")
    };
    let read = state
        .read_content(
            &peer(),
            &ContentReadParams {
                target: target.clone(),
                handle: content.handle,
                offset_bytes: Uint(0),
                limit_bytes: Uint(65536),
            },
            goatr::pairing::now_ms(),
        )
        .unwrap();
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(read.bytes.0)
        .unwrap();
    let original = String::from_utf8(decoded).unwrap();
    assert!(
        !original.contains("NEVER-KEY")
            && !original.contains("NEVER-SIGNATURE")
            && !original.contains("NEVER-SIGNED")
    );
    assert!(original.contains("toolCall"));
    let unknown=project_entry(&state,&target,&json!({"id":"unknown","parentId":null,"type":"future-entry","data":{"count":2,"password":"NEVER-PASS"},"pendingRequests":[{"prefill":"NEVER-PENDING"}]}),RecordSource::Persisted,2,None).unwrap();
    assert_eq!(unknown[0].record().kind, RecordKind::Unsupported);
    let encoded = serde_json::to_string(unknown[0].record()).unwrap();
    assert!(encoded.contains("future-entry"));
    assert!(!encoded.contains("NEVER-PASS") && !encoded.contains("NEVER-PENDING"));
}
#[test]
fn stock_collab_elision_markers_reach_history_without_losing_content() {
    let (state, value, target, epoch) = state();
    // replication-shrink.ts at 4ef97c8826ee012829a3e756b693a2a16a414f47:
    // the 256-char pass reserves 80 chars, so a 257-char leaf retains 176
    // and elides 81; the final array pass keeps one item from a two-item array.
    // Depth >= 1000 and repeated ancestors emit the exact standalone markers.
    let markers = [
        format!("{}\n…[81 chars elided for collab session]", "x".repeat(176)),
        "…[1 items elided for collab session]".into(),
        "…[deeper levels elided for collab session]".into(),
        "…[cyclic reference elided for collab session]".into(),
    ];
    let control = "Ordinary [collaborators] discuss chars and items elided for collab session.";
    let mut projected = Vec::new();
    let mut expected = Vec::new();
    for (index, (body, truncated)) in markers
        .iter()
        .map(|marker| (marker.as_str(), true))
        .chain(std::iter::once((control, false)))
        .enumerate()
    {
        for (carrier, content) in [
            ("string", json!(body)),
            ("text", json!([{"type":"text","text":body}])),
            ("thinking", json!([{"type":"thinking","thinking":body}])),
            (
                "tool",
                json!([{"type":"toolCall","id":format!("call-{index}"),
                    "name":"read","arguments":{"nested":[{"value":body}],"n":2}}]),
            ),
        ] {
            let id = format!("{carrier}-{index}");
            let entry = json!({"type":"message","id":id,"parentId":"branch",
                "message":{"role":"assistant","content":content}});
            projected.extend(
                project_entry(
                    &state,
                    &target,
                    &entry,
                    RecordSource::Live,
                    projected.len() as u64,
                    None,
                )
                .unwrap(),
            );
            let (id, visible) = if carrier == "tool" {
                (
                    format!("omp-tool:call-{index}"),
                    json!({"nested":[{"value":body}],"n":2}).to_string(),
                )
            } else {
                (id, body.to_owned())
            };
            expected.push((id, visible, truncated, carrier == "tool"));
        }
    }
    // Array clipping and depth/cycle replacement can put a standalone string
    // beside surviving content blocks, not just inside a text/thinking field.
    for (index, marker) in markers.iter().enumerate().skip(1) {
        let id = format!("block-{index}");
        let entry = json!({"type":"message","id":id,"parentId":"branch",
            "message":{"role":"assistant","content":[{"type":"text","text":"kept"},marker]}});
        projected.extend(
            project_entry(
                &state,
                &target,
                &entry,
                RecordSource::Live,
                projected.len() as u64,
                None,
            )
            .unwrap(),
        );
        expected.push((id, format!("kept\n{marker}"), true, false));
    }
    let omitted = "…[message entry omitted for collab session: too large to replicate]";
    let entry = json!({"type":"custom_message","id":"omitted","parentId":"branch",
        "customType":"collab-entry-too-large","display":true,"content":omitted});
    projected.extend(
        project_entry(
            &state,
            &target,
            &entry,
            RecordSource::Live,
            projected.len() as u64,
            None,
        )
        .unwrap(),
    );
    expected.push(("omitted".into(), omitted.into(), true, false));
    finish(&state, value, &epoch, projected);
    let visible = state
        .history(&peer(), &target, None, 100, 0)
        .unwrap()
        .result;
    for (id, body, truncated, tool) in expected {
        let record = visible
            .records
            .iter()
            .find(|record| record.id.0 == id)
            .unwrap();
        let content = if tool {
            record.tool.as_ref().unwrap().input.as_ref().unwrap()
        } else {
            record.text.as_ref().unwrap()
        };
        let Content::Text(content) = content else {
            panic!("small native excerpt must remain visible: {id}")
        };
        assert_eq!(content.text.0, body, "visible native content: {id}");
        assert_eq!(
            record.source_truncated, truncated,
            "native source loss: {id}"
        );
        let parent = if tool {
            format!("tool-{}", id.strip_prefix("omp-tool:call-").unwrap())
        } else {
            "branch".into()
        };
        assert_eq!(record.parent_id, Some(Id(parent)));
    }
    assert_eq!(state.source_epoch(&target).unwrap(), epoch);
}
#[test]
fn title_slot_partial_tail_replacement_and_truncation_are_read_only_source_boundaries() {
    let root = Root::new();
    let mut slot = json!({"type":"title","v":1,"title":"Native title","updatedAt":"2026-10-07T00:00:00.000Z","pad":""});
    let initial = slot.to_string().len() + 1;
    slot["pad"] = json!(" ".repeat(256 - initial));
    let header = json!({"type":"session","id":"session","timestamp":"2026-10-07T00:00:00.000Z","cwd":"/not-authority"});
    let first = json!({"type":"message","id":"a","parentId":null,"message":{"role":"user","content":"hello"}});
    let bytes = format!("{slot}\n{header}\n{first}\n{{\"id\":\"b\"");
    let path = root.write("history.jsonl", bytes.as_bytes());
    let owner = process_evidence(std::process::id()).unwrap();
    let mut file = HistoryFile::open(path.clone(), vec![root.0.clone()], owner.clone()).unwrap();
    assert_eq!(file.len(), 1);
    assert_eq!(file.title.as_deref(), Some("Native title"));
    assert!(!file.refresh().unwrap());
    let mut append = OpenOptions::new().append(true).open(&path).unwrap();
    append
        .write_all(b",\"type\":\"branch_summary\",\"parentId\":\"a\",\"summary\":\"branch\"}\n")
        .unwrap();
    assert!(file.refresh().unwrap());
    assert_eq!(file.len(), 2);
    let (state, _value, target, epoch) = state();
    let records = file
        .page(file.len(), 100, &state, &target, &epoch, None)
        .unwrap();
    assert_eq!(
        records
            .iter()
            .map(|r| r.record().id.0.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "b"]
    );
    let replacement = root.write("replacement", format!("{slot}\n{header}\n").as_bytes());
    fs::rename(replacement, &path).unwrap();
    assert!(file.recheck().is_err());
    let file = HistoryFile::open(path.clone(), vec![root.0.clone()], owner).unwrap();
    OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap()
        .set_len(1)
        .unwrap();
    assert!(file.recheck().is_err());
}

#[test]
fn allocated_history_path_rejects_unsafe_or_wrong_native_materialization() {
    let root = Root::new();
    let path = root.0.join("lazy.jsonl");
    let owner = process_evidence(std::process::id()).unwrap();
    let mut history = HistoryFile::live(
        path.clone(),
        vec![root.0.clone()],
        owner.clone(),
        "session".into(),
    )
    .unwrap();
    assert!(history.header.is_null());
    assert!(history.is_empty());
    assert!(!history.refresh().unwrap());
    let (state, _value, target, epoch) = state();
    assert!(
        history
            .page(0, 100, &state, &target, &epoch, None)
            .unwrap()
            .is_empty()
    );
    assert!(!path.exists());

    std::os::unix::fs::symlink(root.0.join("missing-target"), &path).unwrap();
    assert!(history.recheck().is_err());
    assert!(
        HistoryFile::live(
            path.clone(),
            vec![root.0.clone()],
            owner.clone(),
            "session".into()
        )
        .is_err()
    );
    fs::remove_file(&path).unwrap();
    root.write("lazy.jsonl", b"{\"type\":\"session\",\"id\":\"session\"}\n");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(history.recheck().is_err());
    assert!(
        HistoryFile::live(
            path.clone(),
            vec![root.0.clone()],
            owner.clone(),
            "session".into()
        )
        .is_err()
    );
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let mut file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&path)
        .unwrap();
    writeln!(file, "{}", json!({"type":"session","id":"wrong-native"})).unwrap();
    drop(file);
    assert!(history.recheck().is_err());
    assert!(
        HistoryFile::live(path.clone(), vec![root.0.clone()], owner, "session".into()).is_err()
    );
    let mut file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&path)
        .unwrap();
    writeln!(file, "{}", json!({"type":"session","id":"session"})).unwrap();
    writeln!(
        file,
        "{}",
        json!({"type":"message","id":"actual-entry","parentId":null,
        "message":{"role":"assistant","content":"actual native persistence"}})
    )
    .unwrap();
    drop(file);
    assert!(history.refresh().unwrap());
    assert_eq!(history.header["id"], "session");
    let page = history
        .page(history.len(), 100, &state, &target, &epoch, None)
        .unwrap();
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].record().id.0, "actual-entry");
    fs::remove_file(&path).unwrap();
    assert!(
        history.recheck().is_err(),
        "captured file disappearance cannot become lazy absence again"
    );
}
async fn frame(
    socket: &mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>,
) -> Message {
    tokio::time::timeout(Duration::from_secs(3), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
}
#[tokio::test]
async fn actual_bound_relay_routes_encrypted_peers_and_preserves_live_host_ownership() {
    let root = Root::new();
    let relay = Relay::bind(0).await.unwrap();
    let port = relay.port();
    let path = overlay(&root.config(), port).unwrap();
    let mut yaml = String::new();
    fs::File::open(path)
        .unwrap()
        .read_to_string(&mut yaml)
        .unwrap();
    assert!(yaml.contains(&format!("ws://127.0.0.1:{port}")));
    assert!(Relay::bind(port).await.is_err());
    let (stop, rx) = watch::channel(false);
    let job = tokio::spawn(relay.run(rx));
    let url = format!("ws://127.0.0.1:{port}/r/abcdefghij");
    let (mut host, _) = connect_async(format!("{url}?role=host")).await.unwrap();
    let (mut guest, _) = connect_async(format!("{url}?role=guest")).await.unwrap();
    assert_eq!(
        frame(&mut host).await.to_text().unwrap(),
        "{\"t\":\"peer-joined\",\"peer\":1}"
    );
    let (mut duplicate, _) = connect_async(format!("{url}?role=host")).await.unwrap();
    let Message::Close(Some(close)) = frame(&mut duplicate).await else {
        panic!("duplicate host must close")
    };
    assert_eq!(u16::from(close.code), 4009);
    let link = Link::parse(&format!("abcdefghij.{}", URL_SAFE_NO_PAD.encode([5; 48]))).unwrap();
    let binary = link.seal(br#"{"t":"abort"}"#).unwrap();
    guest.send(Message::Binary(binary.into())).await.unwrap();
    let Message::Binary(routed) = frame(&mut host).await else {
        panic!("native binary required")
    };
    assert_eq!(&routed[..4], &1u32.to_be_bytes());
    assert_eq!(
        link.open(&mut routed.to_vec()).unwrap(),
        br#"{"t":"abort"}"#
    );
    let mut response = link
        .seal(br#"{"t":"entry","entry":{"id":"native"}}"#)
        .unwrap();
    response[..4].copy_from_slice(&1u32.to_be_bytes());
    host.send(Message::Binary(response.clone().into()))
        .await
        .unwrap();
    let Message::Binary(routed) = frame(&mut guest).await else {
        panic!("targeted response required")
    };
    assert_eq!(routed.as_ref(), response.as_slice());
    host.close(None).await.unwrap();
    assert_eq!(
        frame(&mut guest).await.to_text().unwrap(),
        "{\"t\":\"room-closed\"}"
    );
    let Message::Close(Some(close)) = frame(&mut guest).await else {
        panic!("room must close")
    };
    assert_eq!(u16::from(close.code), 4001);
    stop.send(true).unwrap();
    job.await.unwrap().unwrap();
}
#[tokio::test]
async fn relay_http_status_host_origin_role_and_upgrade_are_explicit() {
    let relay = Relay::bind(0).await.unwrap();
    let port = relay.port();
    let (stop, rx) = watch::channel(false);
    let job = tokio::spawn(relay.run(rx));
    for (path, host, extra, status) in [
        ("/wrong?role=host", format!("127.0.0.1:{port}"), "", 404),
        (
            "/r/abcdefghij?role=wrong",
            format!("127.0.0.1:{port}"),
            "",
            404,
        ),
        (
            "/r/abcdefghij?role=host",
            format!("127.0.0.1:{port}"),
            "",
            426,
        ),
        (
            "/r/abcdefghij?role=host",
            "attacker.invalid".into(),
            "",
            403,
        ),
        (
            "/r/abcdefghij?role=host",
            format!("127.0.0.1:{port}"),
            "Origin: https://browser.invalid\r\n",
            403,
        ),
    ] {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        stream
            .write_all(
                format!("GET {path} HTTP/1.1\r\nHost: {host}\r\n{extra}Connection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await
            .unwrap();
        let mut result = Vec::new();
        tokio::time::timeout(Duration::from_secs(3), stream.read_to_end(&mut result))
            .await
            .unwrap()
            .unwrap();
        assert!(
            String::from_utf8(result)
                .unwrap()
                .starts_with(&format!("HTTP/1.1 {status}"))
        );
    }
    let mut request = format!("ws://127.0.0.1:{port}/r/abcdefghij?role=guest")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("Origin", "https://browser.invalid".parse().unwrap());
    assert!(connect_async(request).await.is_err());
    stop.send(true).unwrap();
    job.await.unwrap().unwrap();
}
#[test]
fn registry_rejects_public_metadata_and_does_not_delete_native_publications() {
    let root = Root::new();
    let path = root.write("entry.json", b"{}");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(registry(&root.0).is_err());
    assert!(path.exists());
}
#[test]
fn oversized_native_history_batch_reaches_phone_in_order_and_old_source_cannot_append() {
    let (state, value, target, epoch) = state();
    finish(&state, value.clone(), &epoch, vec![]);
    let connection = state.connect(&peer());
    let subscription = state
        .session_open(&connection, Uuid::random(), &target, 0)
        .unwrap();
    let snapshot = state.pop_output(&connection).unwrap().0.unwrap();
    assert!(matches!(
        decode_payload(&snapshot, Some(Method::SessionOpen)).unwrap(),
        Envelope::Response(_)
    ));
    let text = "b".repeat(8192);
    let mut projected = Vec::new();
    for index in 0..100 {
        let entry = json!({"type":"message","id":format!("batch-{index}"),"message":{"role":"user","content":[{"type":"text","text":text}]}});
        projected.extend(
            project_entry(&state, &target, &entry, RecordSource::Live, index, None).unwrap(),
        );
    }
    state
        .history_from_source(&target, &epoch, projected)
        .unwrap();
    let mut seen = Vec::new();
    let mut frames = 0;
    let mut sequence = goatr::state::ClientSequence::default();
    sequence.snapshot(subscription.clone());
    while let Some(bytes) = state.pop_output(&connection).unwrap().0 {
        assert!(bytes.len() <= FRAME_MAX);
        let Envelope::Event(event) = decode_payload(&bytes, None).unwrap() else {
            panic!("expected native history event")
        };
        assert_eq!(event.subscription, subscription);
        assert_eq!(
            sequence.event(&event),
            goatr::state::SequenceDecision::Apply
        );
        let EventBody::HistoryUpsert(upsert) = event.body else {
            panic!("expected complete history batch")
        };
        seen.extend(upsert.records.into_iter().map(|r| r.id.0));
        frames += 1;
    }
    assert!(frames > 1);
    assert_eq!(
        seen,
        (0..100).map(|i| format!("batch-{i}")).collect::<Vec<_>>()
    );
    let mut replacement_state = value;
    replacement_state.summary.generation = Uuid::random();
    let replacement_target = Target {
        session_id: target.session_id.clone(),
        generation: replacement_state.summary.generation.clone(),
    };
    let stale = json!({"type":"message","id":"stale-must-not-land","message":{"role":"user","content":[{"type":"text","text":"old source"}]}});
    let projected = project_entry(&state, &target, &stale, RecordSource::Live, 100, None).unwrap();
    let replacement = state.begin_install(replacement_state.clone()).unwrap();
    assert_eq!(
        state.source_epoch(&replacement_target).unwrap(),
        replacement
    );
    assert_ne!(replacement_target.generation, target.generation);
    assert!(
        state
            .history_from_source(&target, &epoch, projected)
            .is_err()
    );
    finish(&state, replacement_state, &replacement, vec![]);
    assert_eq!(
        state.history(&peer(), &target, None, 100, 0).err(),
        Some(ErrorCode::StaleSession)
    );
    assert!(
        state
            .history(&peer(), &replacement_target, None, 100, 0)
            .unwrap()
            .result
            .records
            .iter()
            .all(|r| r.id.0 != "stale-must-not-land")
    );
}

#[test]
fn one_native_entry_with_101_records_reaches_consumer_without_disconnect_or_overflow() {
    let (state, value, target, epoch) = state();
    finish(&state, value, &epoch, vec![]);
    let reader = peer();
    let connection = state.connect(&reader);
    let subscription = state
        .session_open(&connection, Uuid::random(), &target, 0)
        .unwrap();
    let (Some(snapshot), closed) = state.pop_output(&connection).unwrap() else {
        panic!("initial session snapshot required")
    };
    assert!(!closed);
    let Envelope::Response(response) =
        decode_payload(&snapshot, Some(Method::SessionOpen)).unwrap()
    else {
        panic!("initial session response required")
    };
    let Ok(ResultBody::SessionOpen(open)) = response.outcome else {
        panic!("initial session history required")
    };
    assert!(open.history.is_empty());
    let entry = json!({
        "type":"message","id":"count-boundary",
        "message":{"role":"assistant","content":(0..100).map(|n|json!({
            "type":"toolCall","id":format!("count-{n}"),"name":"read","arguments":{"n":n}
        })).collect::<Vec<_>>()}
    });
    let projected = project_entry(&state, &target, &entry, RecordSource::Live, 0, None).unwrap();
    assert_eq!(projected.len(), 101);
    state
        .history_from_source(&target, &epoch, projected)
        .expect("101 small native records must not reject the healthy source");
    let expected: Vec<_> = std::iter::once("count-boundary".to_owned())
        .chain((0..100).map(|n| format!("omp-tool:count-{n}")))
        .collect();
    let mut sequence = goatr::state::ClientSequence::default();
    sequence.snapshot(subscription.clone());
    let mut visible = Vec::new();
    loop {
        let (bytes, closed) = state.pop_output(&connection).unwrap();
        assert!(
            !closed,
            "count-boundary publication disconnected the consumer"
        );
        let Some(bytes) = bytes else { break };
        let Envelope::Event(event) = decode_payload(&bytes, None).unwrap() else {
            panic!("ordered history event required")
        };
        assert_eq!(event.subscription, subscription);
        assert_eq!(
            sequence.event(&event),
            goatr::state::SequenceDecision::Apply
        );
        let EventBody::HistoryUpsert(upsert) = event.body else {
            panic!("count-boundary publication must not require overflow recovery")
        };
        assert!(!upsert.records.is_empty() && upsert.records.len() <= 100);
        visible.extend(upsert.records.into_iter().map(|record| record.id.0));
    }
    assert_eq!(visible, expected);
    assert!(sequence.mutations_enabled());
    let latest = state
        .history(&reader, &target, None, 100, 0)
        .unwrap()
        .result;
    let older = state
        .history(&reader, &target, latest.next_cursor.as_ref(), 100, 0)
        .unwrap()
        .result;
    assert_eq!(
        older
            .records
            .into_iter()
            .chain(latest.records)
            .map(|record| record.id.0)
            .collect::<Vec<_>>(),
        expected
    );
}
