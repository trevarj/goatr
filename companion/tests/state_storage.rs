// SPDX-License-Identifier: GPL-3.0-or-later
use goatr::{protocol::*, state::*, storage::*};
use std::{
    fs,
    os::unix::fs::{DirBuilderExt, MetadataExt},
    path::PathBuf,
};
fn id(text: &str) -> Id {
    Id::new(text).unwrap()
}
fn peer(store: &mut Storage, seed: u8) -> AuthenticatedPeer {
    let endpoint = EndpointId::new(
        iroh::SecretKey::from_bytes(&[seed; 32])
            .public()
            .to_string(),
    )
    .unwrap();
    let device = PairedDevice {
        device_id: Uuid::random(),
        endpoint_id: endpoint,
        device_name: Label::new("test phone").unwrap(),
        paired_at: Uint(0),
        last_seen_at: None,
    };
    let epoch = store.pair_device(&device).unwrap();
    store.authenticated_hello(&device.device_id, epoch).unwrap()
}
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
fn summary(provider: Provider) -> SessionState {
    SessionState {
        summary: SessionSummary {
            session_id: Uuid::random(),
            generation: Uuid::random(),
            server_id: Uuid::random(),
            workspace_id: id("workspace"),
            terminal_id: id("terminal"),
            pane_id: id("pane"),
            provider,
            native_id: id("native"),
            title: "test".into(),
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
fn install(state: &State, value: &SessionState, history: Vec<SafeHistory>) {
    let source = state.begin_install(value.clone()).unwrap();
    state
        .finish_install(
            &source,
            ProviderSnapshot {
                state: value.clone(),
                pending: vec![],
                history,
                older_available: false,
                native_older: None,
            },
        )
        .unwrap();
}
fn record(n: u64, source: RecordSource, text: &str) -> SafeHistory {
    SafeHistory::from_redacted_display(HistoryRecord {
        id: id(&format!("item-{n}")),
        parent_id: None,
        turn_id: None,
        order: Uint(n),
        timestamp: None,
        role: Role::Assistant,
        kind: RecordKind::Message,
        text: Some(Content::Text(ContentText {
            text: InlineText::new(text).unwrap(),
        })),
        tool: None,
        original: None,
        source,
        complete: true,
        source_truncated: false,
    })
    .unwrap()
}
fn prompt(store: &Storage, target: &Target, text: &str) -> Request {
    Request {
        id: Uuid::random(),
        body: RequestBody::PromptSubmit(PromptSubmitParams {
            delivery: None,
            native_turn_id: None,
            images: None,
            skill: None,
            boot_id: store.boot_id.clone(),
            target: target.clone(),
            text: PromptText::new(text).unwrap(),
        }),
    }
}
fn policy<T: std::fmt::Debug>(result: StoreResult<T>, code: ErrorCode) {
    assert!(matches!(result, Err(StorageError::Policy(c)) if c == code));
}
#[test]
fn native_skill_selector_fingerprints_and_matching_receipts_precede_stale_admission() {
    let mut store = Storage::memory([79; 32], 0).unwrap();
    let owner = peer(&mut store, 80);
    let other = peer(&mut store, 81);
    let target = Target {
        session_id: Uuid::random(),
        generation: Uuid::random(),
    };
    let mut request = prompt(&store, &target, "");
    let RequestBody::PromptSubmit(params) = &mut request.body else {
        unreachable!()
    };
    params.skill = Some(SkillSelector {
        catalogue_generation: Uuid::random(),
        reference: Uuid::random(),
    });
    let Reservation::Reserved(original) = store.reserve(&owner, &request, 1, |_| Ok(())).unwrap()
    else {
        panic!("new native skill intent was not reserved")
    };
    let Reservation::Existing(recovered) = store
        .reserve(&owner, &request, 2, |_| {
            panic!("matching receipt must not re-admit an expired publication")
        })
        .unwrap()
    else {
        panic!("original receipt was not returned")
    };
    assert_eq!(original, recovered);
    let mut conflict = request.clone();
    let RequestBody::PromptSubmit(params) = &mut conflict.body else {
        unreachable!()
    };
    params.skill.as_mut().unwrap().reference = Uuid::random();
    policy(
        store.reserve(&owner, &conflict, 3, |_| {
            panic!("fingerprint conflict must win")
        }),
        ErrorCode::ActionIdConflict,
    );
    let mut stale = request.clone();
    stale.id = Uuid::random();
    policy(
        store.reserve(&owner, &stale, 4, |_| Err(ErrorCode::StaleSession)),
        ErrorCode::StaleSession,
    );
    assert!(store.status(&owner, &stale.id, 5).unwrap().is_none());
    assert!(store.status(&other, &request.id, 5).unwrap().is_none());
    policy(
        store.reserve(&other, &request, 6, |_| Err(ErrorCode::NotReady)),
        ErrorCode::NotReady,
    );
    store.revoke_epoch(owner.peer_id(), 7).unwrap();
    policy(
        store.reserve(&owner, &request, 8, |_| {
            panic!("revoked authorization must never reach native admission")
        }),
        ErrorCode::Unauthorized,
    );
}

struct PrivateRoot(PathBuf);
impl PrivateRoot {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("goatr-state-{}", Uuid::random().0));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        Self(path)
    }
    fn uid(&self) -> u32 {
        fs::metadata(&self.0).unwrap().uid()
    }
}
impl Drop for PrivateRoot {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn review_approval(details: Content) -> PendingRequest {
    PendingRequest {
        request_key: Uuid::random(),
        provider: Provider::Codex,
        kind: PresentationKind::CodexApproval,
        presentation: Presentation::CodexApproval(PresentationCodexApproval {
            approval_type: ApprovalType::Command,
            title: Content::Text(ContentText {
                text: InlineText::new("redacted permission details").unwrap(),
            }),
            details,
            choices: vec![ApprovalChoice {
                choice_id: id("once"),
                label: Label::new("Allow once").unwrap(),
                scope: ApprovalScope::Once,
                details: None,
            }],
        }),
    }
}

fn review_open(state: &State, connection: &Uuid, target: &Target) -> SessionOpenResult {
    let request = Uuid::random();
    state
        .session_open(connection, request.clone(), target, 0)
        .unwrap();
    let bytes = state.pop_output(connection).unwrap().0.unwrap();
    let Envelope::Response(response) = decode_payload(&bytes, Some(Method::SessionOpen)).unwrap()
    else {
        panic!("session.open must enqueue its snapshot first")
    };
    assert_eq!(response.id, request);
    let Ok(ResultBody::SessionOpen(snapshot)) = response.outcome else {
        panic!("expected session.open snapshot")
    };
    snapshot
}

#[test]
fn review_late_install_completion_cannot_replace_new_source() {
    let mut storage = Storage::memory([1; 32], 0).unwrap();
    let peer = peer(&mut storage, 61);
    let state = State::default();
    let session = summary(Provider::Codex);
    let target = target(&session);
    install(&state, &session, vec![]);
    let source_a = state.begin_source_install(&target).unwrap();
    let old_pending = review_approval(Content::Text(ContentText {
        text: InlineText::new("old source only").unwrap(),
    }));
    let snapshot_a = ProviderSnapshot {
        state: session.clone(),
        pending: vec![old_pending],
        history: vec![record(0, RecordSource::NativePage, "old source only")],
        older_available: false,
        native_older: None,
    };
    state.abort_install(&target, &source_a).unwrap();
    let source_b = state.begin_source_install(&target).unwrap();
    assert_ne!(source_a, source_b);
    assert_eq!(
        state.abort_install(&target, &source_a),
        Err(ErrorCode::StaleSession)
    );
    let new_pending = review_approval(Content::Text(ContentText {
        text: InlineText::new("replacement source").unwrap(),
    }));
    state
        .update(
            &target,
            SessionUpdate::History(vec![record(2, RecordSource::Live, "queued replacement")]),
        )
        .unwrap();
    // Deterministic completion order: A was captured before abort; B is now installing.
    assert_eq!(
        state.finish_install(&source_a, snapshot_a),
        Err(ErrorCode::StaleSession),
        "the superseded source completion must not consume B's installation"
    );
    state
        .finish_install(
            &source_b,
            ProviderSnapshot {
                state: session,
                pending: vec![new_pending.clone()],
                history: vec![record(1, RecordSource::NativePage, "replacement source")],
                older_available: false,
                native_older: None,
            },
        )
        .unwrap();
    let connection = state.connect(&peer);
    let snapshot = review_open(&state, &connection, &target);
    assert_eq!(snapshot.pending_requests, vec![new_pending]);
    assert_eq!(
        snapshot
            .history
            .iter()
            .map(|r| r.id.clone())
            .collect::<Vec<_>>(),
        vec![id("item-1"), id("item-2")]
    );
}

#[test]
fn review_shared_large_history_and_pending_refs_survive_phone_revocation() {
    let mut storage = Storage::memory([1; 32], 0).unwrap();
    let state = State::default();
    let session = summary(Provider::Codex);
    let target = target(&session);
    let source = state.begin_install(session.clone()).unwrap();
    let history_text = "h".repeat(CONTENT_INLINE_MAX + 1);
    let pending_text = "p".repeat(CONTENT_INLINE_MAX + 1);
    let history_content = state
        .content(&target, Mime::TextPlain, history_text.clone(), 0)
        .unwrap();
    let pending_content = state
        .content(&target, Mime::TextPlain, pending_text.clone(), 0)
        .unwrap();
    let mut history = record(0, RecordSource::NativePage, "redacted display")
        .record()
        .clone();
    history.text = Some(history_content.clone());
    let pending = review_approval(pending_content.clone());
    state
        .finish_install(
            &source,
            ProviderSnapshot {
                state: session,
                pending: vec![pending.clone()],
                history: vec![SafeHistory::from_redacted_display(history).unwrap()],
                older_available: false,
                native_older: None,
            },
        )
        .unwrap();
    let a = peer(&mut storage, 62);
    let b = peer(&mut storage, 63);
    let ca = state.connect(&a);
    let cb = state.connect(&b);
    let snapshot_a = review_open(&state, &ca, &target);
    let snapshot_b = review_open(&state, &cb, &target);
    let reopened_b = review_open(&state, &cb, &target);
    for snapshot in [&snapshot_a, &snapshot_b, &reopened_b] {
        assert_eq!(snapshot.history[0].text, Some(history_content.clone()));
        assert_eq!(snapshot.pending_requests, vec![pending.clone()]);
    }
    let reads: Vec<_> = [history_content, pending_content]
        .into_iter()
        .map(|content| {
            let Content::Ref(reference) = content else {
                panic!("the inline + 1 boundary must produce a handle")
            };
            ContentReadParams {
                target: target.clone(),
                handle: reference.handle,
                offset_bytes: Uint(0),
                limit_bytes: reference.byte_length,
            }
        })
        .collect();
    let expected = [&history_text, &pending_text];
    for (read, text) in reads.iter().zip(expected) {
        let result = state.read_content(&a, read, 0).unwrap();
        let bytes =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &result.bytes.0)
                .unwrap();
        assert_eq!(bytes, text.as_bytes());
        assert!(result.eof);
    }
    let b_before: Vec<_> = reads
        .iter()
        .map(|read| state.read_content(&b, read, 0))
        .collect();
    storage.revoke_epoch(a.peer_id(), 0).unwrap();
    state.revoke(a.peer_id());
    policy(storage.recheck(&a), ErrorCode::Unauthorized);
    storage.recheck(&b).unwrap();
    assert_eq!(state.pop_output(&ca).err(), Some(ErrorCode::Unauthorized));
    let b_after: Vec<_> = reads
        .iter()
        .map(|read| state.read_content(&b, read, 0))
        .collect();
    for results in [b_before, b_after] {
        for (result, text) in results.into_iter().zip(expected) {
            let result = result.expect("a currently paired peer must read shared display handles");
            let bytes =
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &result.bytes.0)
                    .unwrap();
            assert_eq!(bytes, text.as_bytes());
            assert!(result.eof);
        }
    }
}

#[test]
fn atomic_snapshot_native_handoff_and_client_gap_recovery() {
    let mut storage = Storage::memory([7; 32], 0).unwrap();
    let peer = peer(&mut storage, 1);
    let state = State::default();
    let session = summary(Provider::Codex);
    let target = target(&session);
    let source = state.begin_install(session.clone()).unwrap();
    state
        .update(
            &target,
            SessionUpdate::Activity(Activity {
                state: ActivityState::Working,
                text: None,
                tool_record_id: None,
            }),
        )
        .unwrap();
    let connection = state.connect(&peer);
    assert_eq!(
        state.session_open(&connection, Uuid::random(), &target, 0),
        Err(ErrorCode::NotReady)
    );
    state
        .finish_install(
            &source,
            ProviderSnapshot {
                state: session.clone(),
                pending: vec![],
                history: vec![],
                older_available: false,
                native_older: None,
            },
        )
        .unwrap();
    let subscription = state
        .session_open(&connection, Uuid::random(), &target, 0)
        .unwrap();
    state
        .update(
            &target,
            SessionUpdate::History(vec![record(1, RecordSource::Live, "complete")]),
        )
        .unwrap();
    let first = state.pop_output(&connection).unwrap().0.unwrap();
    let Envelope::Response(response) = decode_payload(&first, Some(Method::SessionOpen)).unwrap()
    else {
        panic!()
    };
    let Ok(ResultBody::SessionOpen(snapshot)) = response.outcome else {
        panic!()
    };
    assert_eq!(snapshot.state.activity.state, ActivityState::Working);
    let second = state.pop_output(&connection).unwrap().0.unwrap();
    let Envelope::Event(event) = decode_payload(&second, None).unwrap() else {
        panic!()
    };
    assert_eq!(event.subscription, subscription);
    assert_eq!(event.seq, Uint(1));
    let mut client = ClientSequence::default();
    client.snapshot(subscription.clone());
    assert_eq!(client.event(&event), SequenceDecision::Apply);
    assert_eq!(client.event(&event), SequenceDecision::Duplicate);
    let gap = Event {
        seq: Uint(3),
        ..event
    };
    assert_eq!(client.event(&gap), SequenceDecision::Resnapshot);
    assert!(!client.mutations_enabled());
    client.snapshot(Uuid::random());
    assert!(client.mutations_enabled());
}
#[test]
fn ordered_output_overflow_does_not_drop_durable_events_and_self_unpair_is_only_final_frame() {
    let mut storage = Storage::memory([1; 32], 0).unwrap();
    let peer = peer(&mut storage, 2);
    let state = State::default();
    let session = summary(Provider::Omp);
    let target = target(&session);
    install(&state, &session, vec![]);
    let connection = state.connect(&peer);
    state
        .session_open(&connection, Uuid::random(), &target, 0)
        .unwrap();
    state.pop_output(&connection).unwrap();
    for _ in 0..257 {
        state
            .update(
                &target,
                SessionUpdate::Close(Uuid::random(), RequestClosedReason::Resolved),
            )
            .unwrap();
    }
    for sequence in 1..=256 {
        let (bytes, closed) = state.pop_output(&connection).unwrap();
        assert!(closed);
        let Envelope::Event(event) = decode_payload(&bytes.unwrap(), None).unwrap() else {
            panic!()
        };
        assert_eq!(event.seq, Uint(sequence));
    }
    assert!(state.pop_output(&connection).unwrap().0.is_none());
    let fresh = state.connect(&peer);
    state.host_snapshot(&fresh, Uuid::random()).unwrap();
    let final_id = Uuid::random();
    state.self_unpair_final(&fresh, final_id.clone()).unwrap();
    let (bytes, closed) = state.pop_output(&fresh).unwrap();
    assert!(closed);
    let Envelope::Response(response) =
        decode_payload(&bytes.unwrap(), Some(Method::DeviceUnpair)).unwrap()
    else {
        panic!()
    };
    assert_eq!(response.id, final_id);
    assert!(state.pop_output(&fresh).unwrap().0.is_none());
}
#[test]
fn identical_state_suppression_keeps_activity_order_and_every_critical_authority_change() {
    let mut storage = Storage::memory([83; 32], 0).unwrap();
    let peer = peer(&mut storage, 84);
    let state = State::default();
    let original = summary(Provider::Codex);
    let target = target(&original);
    install(&state, &original, vec![]);
    let epoch = state.source_epoch(&target).unwrap();
    let connection = state.connect(&peer);
    review_open(&state, &connection, &target);
    state
        .update(&target, SessionUpdate::State(original.clone()))
        .unwrap();
    assert!(state.pop_output(&connection).unwrap().0.is_none());
    state
        .update(
            &target,
            SessionUpdate::Activity(Activity {
                state: ActivityState::Working,
                text: None,
                tool_record_id: None,
            }),
        )
        .unwrap();
    let effective = state.session_state(&target, &epoch).unwrap();
    state
        .update(&target, SessionUpdate::State(effective.clone()))
        .unwrap();
    let Envelope::Event(activity) =
        decode_payload(&state.pop_output(&connection).unwrap().0.unwrap(), None).unwrap()
    else {
        panic!("pending activity must still drain")
    };
    assert_eq!(activity.seq, Uint(1));
    assert!(matches!(activity.body, EventBody::ActivityUpdate(_)));
    assert!(state.pop_output(&connection).unwrap().0.is_none());
    let mut changed = effective;
    for field in 0..7 {
        match field {
            0 => changed.summary.capabilities.prompt = false,
            1 => {
                changed.summary.capabilities.skills_read = true;
                changed.summary.skills_generation = Some(Uuid::random());
            }
            2 => changed.summary.skills_generation = Some(Uuid::random()),
            3 => changed.native_turn_id = Some(id("native-turn")),
            4 => changed.summary.connectivity = SessionConnectivity::Synchronizing,
            5 => changed.summary.connectivity = SessionConnectivity::Ready,
            6 => changed.summary.capabilities.prompt = true,
            _ => unreachable!(),
        }
        state
            .update(&target, SessionUpdate::State(changed.clone()))
            .unwrap();
        let Envelope::Event(event) =
            decode_payload(&state.pop_output(&connection).unwrap().0.unwrap(), None).unwrap()
        else {
            panic!("critical state change must be immediate")
        };
        assert_eq!(event.seq, Uint(field + 2));
        let EventBody::SessionState(value) = event.body else {
            panic!("critical state cannot be replaced by transient activity")
        };
        assert_eq!(value.state, changed);
        assert!(state.pop_output(&connection).unwrap().0.is_none());
        state
            .update(&target, SessionUpdate::State(changed.clone()))
            .unwrap();
        assert!(state.pop_output(&connection).unwrap().0.is_none());
    }
    for _ in 0..2 {
        state.update(&target, SessionUpdate::SourceGap).unwrap();
        let Envelope::Event(event) =
            decode_payload(&state.pop_output(&connection).unwrap().0.unwrap(), None).unwrap()
        else {
            panic!("source gaps are never coalesced")
        };
        assert!(matches!(event.body, EventBody::SyncRequired(_)));
    }
}

#[test]
fn transient_coalescing_and_host_peer_subscription_bounds() {
    let mut storage = Storage::memory([1; 32], 0).unwrap();
    let a = peer(&mut storage, 3);
    let b = peer(&mut storage, 4);
    let state = State::default();
    let session = summary(Provider::Omp);
    let target = target(&session);
    install(&state, &session, vec![]);
    let ca = state.connect(&a);
    let cb = state.connect(&b);
    state.host_snapshot(&ca, Uuid::random()).unwrap();
    state.host_snapshot(&cb, Uuid::random()).unwrap();
    state.pop_output(&ca).unwrap();
    state.pop_output(&cb).unwrap();
    let request = prompt(&storage, &target, "ordinary");
    let receipt = match storage.reserve(&a, &request, 1, |_| Ok(())).unwrap() {
        Reservation::Reserved(r) => r,
        _ => panic!(),
    };
    state
        .peer_event(&a, EventBody::ActionStatus(ActionStatusEvent { receipt }))
        .unwrap();
    assert!(state.pop_output(&ca).unwrap().0.is_some());
    assert!(state.pop_output(&cb).unwrap().0.is_none());
    for _ in 0..31 {
        state.session_open(&ca, Uuid::random(), &target, 0).unwrap();
        state.pop_output(&ca).unwrap();
    }
    assert_eq!(
        state.session_open(&ca, Uuid::random(), &target, 0),
        Err(ErrorCode::ResourceLimit)
    );
    state.host_snapshot(&ca, Uuid::random()).unwrap();
    state.pop_output(&ca).unwrap();
    state
        .update(
            &target,
            SessionUpdate::Activity(Activity {
                state: ActivityState::Working,
                text: None,
                tool_record_id: None,
            }),
        )
        .unwrap();
    state
        .update(
            &target,
            SessionUpdate::Activity(Activity {
                state: ActivityState::Idle,
                text: None,
                tool_record_id: None,
            }),
        )
        .unwrap();
    // Multiple subscribers each retain its own ordered sequence; a single subscription check
    // below proves coalescing before sequencing, not durable event replacement.
    let fresh = state.connect(&b);
    state
        .session_open(&fresh, Uuid::random(), &target, 0)
        .unwrap();
    state.pop_output(&fresh).unwrap();
    for work in [ActivityState::Working, ActivityState::Idle] {
        state
            .update(
                &target,
                SessionUpdate::Activity(Activity {
                    state: work,
                    text: None,
                    tool_record_id: None,
                }),
            )
            .unwrap();
    }
    let Envelope::Event(event) =
        decode_payload(&state.pop_output(&fresh).unwrap().0.unwrap(), None).unwrap()
    else {
        panic!()
    };
    assert_eq!(event.seq, Uint(1));
    assert!(matches!(
        event.body,
        EventBody::ActivityUpdate(ActivityUpdateEvent {
            activity: Activity {
                state: ActivityState::Idle,
                ..
            }
        })
    ));
    assert!(state.pop_output(&fresh).unwrap().0.is_none());
}
#[test]
fn content_utf8_offsets_ttl_lru_host_authority_generation_source_and_payload_bounds() {
    let mut storage = Storage::memory([1; 32], 0).unwrap();
    let a = peer(&mut storage, 5);
    let b = peer(&mut storage, 6);
    let state = State::default();
    let session = summary(Provider::Omp);
    let target = target(&session);
    install(&state, &session, vec![]);
    assert!(matches!(
        state
            .content(&target, Mime::TextPlain, "x".repeat(CONTENT_INLINE_MAX), 0)
            .unwrap(),
        Content::Text(_)
    ));
    let text = format!("é{}", "x".repeat(CONTENT_INLINE_MAX));
    let Content::Ref(reference) = state
        .content(&target, Mime::TextPlain, text.clone(), 0)
        .unwrap()
    else {
        panic!()
    };
    let read = ContentReadParams {
        target: target.clone(),
        handle: reference.handle.clone(),
        offset_bytes: Uint(1),
        limit_bytes: Uint(1),
    };
    assert_eq!(state.read_content(&a, &read, 0).unwrap().bytes.0, "qQ==");
    assert_eq!(state.read_content(&b, &read, 0).unwrap().bytes.0, "qQ==");
    let end = ContentReadParams {
        offset_bytes: reference.byte_length,
        ..read.clone()
    };
    assert!(state.read_content(&a, &end, 0).unwrap().eof);
    assert_eq!(state.read_content(&a, &end, 0).unwrap().bytes.0, "");
    assert_eq!(
        state.read_content(&a, &read, CONTENT_TTL_MS).err(),
        Some(ErrorCode::ContentExpired)
    );
    let mut handles = vec![];
    for _ in 0..4 {
        let Content::Ref(r) = state
            .content(
                &target,
                Mime::TextPlain,
                "x".repeat(CONTENT_MAX as usize),
                1,
            )
            .unwrap()
        else {
            panic!()
        };
        handles.push(r.handle);
    }
    let first = ContentReadParams {
        handle: handles[0].clone(),
        offset_bytes: Uint(0),
        ..read.clone()
    };
    state.read_content(&a, &first, 2).unwrap();
    state
        .content(
            &target,
            Mime::TextPlain,
            "x".repeat(CONTENT_INLINE_MAX + 1),
            3,
        )
        .unwrap();
    assert!(state.read_content(&a, &first, 3).is_ok());
    assert_eq!(
        state
            .read_content(
                &a,
                &ContentReadParams {
                    handle: handles[1].clone(),
                    ..first.clone()
                },
                3
            )
            .err(),
        Some(ErrorCode::ContentExpired)
    );
    assert!(matches!(
        state
            .content(
                &target,
                Mime::TextPlain,
                "x".repeat(CONTENT_MAX as usize + 1),
                3
            )
            .unwrap(),
        Content::Unavailable(ContentUnavailable {
            reason: ContentUnavailableReason::TooLarge,
            ..
        })
    ));
    state.update(&target, SessionUpdate::SourceGap).unwrap();
    assert_eq!(
        state.read_content(&a, &first, 4).err(),
        Some(ErrorCode::ContentExpired)
    );
}
#[test]
fn global_history_budget_evicts_only_old_prefixes_and_expires_pinned_cursors() {
    let mut storage = Storage::memory([1; 32], 0).unwrap();
    let peer = peer(&mut storage, 64);
    let state = State::default();
    let text = "x".repeat(CONTENT_INLINE_MAX);
    let mut sessions = vec![];
    for _ in 0..3 {
        let session = summary(Provider::Codex);
        install(
            &state,
            &session,
            (0..HISTORY_RECORDS as u64)
                .map(|n| record(n, RecordSource::NativePage, &text))
                .collect(),
        );
        sessions.push(session);
    }
    let first_target = target(&sessions[0]);
    let pinned = state
        .history(&peer, &first_target, None, 100, 0)
        .unwrap()
        .result
        .next_cursor
        .unwrap();
    for _ in 0..2 {
        let session = summary(Provider::Codex);
        install(
            &state,
            &session,
            (0..HISTORY_RECORDS as u64)
                .map(|n| record(n, RecordSource::NativePage, &text))
                .collect(),
        );
        sessions.push(session);
    }
    assert_eq!(
        state
            .history(&peer, &first_target, Some(&pinned), 100, 0)
            .err(),
        Some(ErrorCode::CursorExpired)
    );
    let connection = state.connect(&peer);
    let mut retained_bytes = 0;
    let mut truncated = false;
    let mut buffer = [0_u8; 32 * 1024];
    let mut encoded = std::io::Cursor::new(&mut buffer[..]);
    for session in sessions {
        let target = target(&session);
        let snapshot = review_open(&state, &connection, &target);
        assert!(snapshot.history.len() <= 100);
        let first = state.history(&peer, &target, None, 100, 0).unwrap();
        let mut records = first.result.records;
        let mut cursor = first.result.next_cursor;
        let mut orders = vec![];
        loop {
            for record in records {
                encoded.set_position(0);
                serde_json::to_writer(&mut encoded, &record).unwrap();
                retained_bytes += encoded.position() as usize;
                orders.push(record.order.0);
            }
            let Some(token) = cursor else { break };
            let page = state.history(&peer, &target, Some(&token), 100, 0).unwrap();
            records = page.result.records;
            cursor = page.result.next_cursor;
        }
        orders.sort_unstable();
        assert!(orders.len() <= HISTORY_RECORDS);
        assert!(
            orders
                .iter()
                .copied()
                .eq((HISTORY_RECORDS - orders.len()) as u64..HISTORY_RECORDS as u64)
        );
        if orders.len() < HISTORY_RECORDS {
            truncated = true;
            assert!(snapshot.state.summary.history_coverage.older_available);
        }
    }
    assert!(truncated);
    assert!(retained_bytes <= SESSION_BYTES);
}

#[test]
fn history_retention_paging_stable_retries_and_source_precedence() {
    let mut storage = Storage::memory([1; 32], 0).unwrap();
    let a = peer(&mut storage, 7);
    let b = peer(&mut storage, 8);
    let state = State::default();
    let session = summary(Provider::Codex);
    let target = target(&session);
    install(
        &state,
        &session,
        (0..1001)
            .map(|n| record(n, RecordSource::NativePage, "historical"))
            .collect(),
    );
    let first = state.history(&a, &target, None, 100, 0).unwrap();
    assert_eq!(first.result.records.len(), 100);
    let cursor = first.result.next_cursor.clone().unwrap();
    let second = state.history(&a, &target, Some(&cursor), 100, 0).unwrap();
    let retry = state.history(&a, &target, Some(&cursor), 100, 1).unwrap();
    assert_eq!(second.result, retry.result);
    assert_eq!(
        state.history(&b, &target, Some(&cursor), 100, 0).err(),
        Some(ErrorCode::CursorExpired)
    );
    state
        .update(
            &target,
            SessionUpdate::History(vec![record(1000, RecordSource::Live, "new live")]),
        )
        .unwrap();
    state
        .update(
            &target,
            SessionUpdate::History(vec![record(1000, RecordSource::NativePage, "old page")]),
        )
        .unwrap();
    let latest = state.history(&a, &target, None, 1, 0).unwrap();
    assert_eq!(
        latest.result.records[0].text,
        record(1000, RecordSource::Live, "new live").record().text
    );
    state
        .update(
            &target,
            SessionUpdate::History(
                (1001..=1010)
                    .map(|n| record(n, RecordSource::Live, "new append"))
                    .collect(),
            ),
        )
        .unwrap();
    let mut retained = state.history(&a, &target, None, 100, 0).unwrap();
    let mut retained_count = retained.result.records.len();
    while let Some(cursor) = retained.result.next_cursor {
        retained = state.history(&a, &target, Some(&cursor), 100, 0).unwrap();
        retained_count += retained.result.records.len();
    }
    assert_eq!(retained_count, HISTORY_RECORDS);
    assert_eq!(retained.result.records[0].order, Uint(11));
    assert!(retained.refetch_older.is_some());
    let mut page = first;
    while let Some(cursor) = page.result.next_cursor {
        page = state.history(&a, &target, Some(&cursor), 100, 0).unwrap();
    }
    assert!(page.refetch_older.is_some());
    assert_eq!(page.result.records[0].order, Uint(1));
    state.update(&target, SessionUpdate::SourceGap).unwrap();
    assert_eq!(
        state.history(&a, &target, Some(&cursor), 100, 0).err(),
        Some(ErrorCode::CursorExpired)
    );
    let persisted = record(0, RecordSource::Persisted, "rich full");
    let live = record(0, RecordSource::Live, "placeholder");
    assert_eq!(
        merge_history(
            Provider::Omp,
            persisted.record(),
            live.record().clone(),
            &mut HistoryOrigins::from_record(persisted.record()),
            false
        )
        .text,
        persisted.record().text
    );
}
#[test]
fn crash_windows_key_loss_matching_lookup_before_stale_and_no_replay() {
    let root = PrivateRoot::new();
    let path = root.0.join("state.sqlite");
    let mut storage = Storage::open(&path, [9; 32], 0, root.uid()).unwrap();
    let a = peer(&mut storage, 9);
    let session = summary(Provider::Omp);
    let request = prompt(&storage, &target(&session), "ordinary recovery text");
    storage.reserve(&a, &request, 1, |_| Ok(())).unwrap();
    storage
        .transition(
            &a,
            &request.id,
            ReceiptTransition {
                state: ReceiptState::Dispatching,
                proof: TransitionProof::Intent,
                result: None,
                error: None,
                at: 2,
            },
        )
        .unwrap();
    drop(storage);
    let mut reopened = Storage::open(&path, [9; 32], 3, root.uid()).unwrap();
    assert_eq!(
        reopened.status(&a, &request.id, 3).unwrap().unwrap().state,
        ReceiptState::Uncertain
    );
    match reopened
        .reserve(&a, &request, 4, |_| {
            panic!("must not validate stale target or replay")
        })
        .unwrap()
    {
        Reservation::Existing(r) => assert_eq!(r.state, ReceiptState::Uncertain),
        _ => panic!(),
    }
    let mut conflict = request.clone();
    if let RequestBody::PromptSubmit(p) = &mut conflict.body {
        p.text = PromptText::new("different").unwrap();
    }
    policy(
        reopened.reserve(&a, &conflict, 4, |_| Ok(())),
        ErrorCode::ActionIdConflict,
    );
    policy(
        reopened.transition(
            &a,
            &request.id,
            ReceiptTransition {
                state: ReceiptState::Dispatching,
                proof: TransitionProof::Intent,
                result: None,
                error: None,
                at: 4,
            },
        ),
        ErrorCode::InvalidRequest,
    );
    let other = peer(&mut reopened, 10);
    assert!(reopened.status(&other, &request.id, 4).unwrap().is_none());
    reopened.revoke_epoch(a.peer_id(), 4).unwrap();
    policy(reopened.status(&a, &request.id, 4), ErrorCode::Unauthorized);
    drop(reopened);
    assert!(Storage::open(&path, [8; 32], 5, root.uid()).is_err());
    let bytes = fs::read(&path).unwrap();
    assert!(
        !bytes
            .windows("ordinary recovery text".len())
            .any(|w| w == b"ordinary recovery text")
    );
}
#[test]
fn sqlite_integer_boundaries_reject_corruption_without_wrapping() {
    let root = PrivateRoot::new();
    let path = root.0.join("state.sqlite");
    let mut storage = Storage::open(&path, [9; 32], 0, root.uid()).unwrap();
    let a = peer(&mut storage, 24);
    let device = storage.devices().unwrap().remove(0);
    let private = rusqlite::Connection::open(&path).unwrap();
    private
        .execute("UPDATE peers SET epoch=-1 WHERE peer=?1", [&a.peer_id().0])
        .unwrap();
    assert!(matches!(
        storage.current_epoch(&device.endpoint_id),
        Err(StorageError::Corrupt)
    ));
    assert!(matches!(storage.recheck(&a), Err(StorageError::Corrupt)));
    assert!(matches!(
        storage.revoke_epoch(a.peer_id(), 0),
        Err(StorageError::Corrupt)
    ));
    private
        .execute(
            "UPDATE peers SET epoch=?1 WHERE peer=?2",
            rusqlite::params![i64::MAX, a.peer_id().0],
        )
        .unwrap();
    assert_eq!(
        storage.current_epoch(&device.endpoint_id).unwrap(),
        Some((a.peer_id().clone(), u64::try_from(i64::MAX).unwrap()))
    );
    assert!(matches!(
        storage.revoke_epoch(a.peer_id(), 0),
        Err(StorageError::Corrupt)
    ));
    let unchanged: i64 = private
        .query_row(
            "SELECT epoch FROM peers WHERE peer=?1",
            [&a.peer_id().0],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(unchanged, i64::MAX);
    private
        .execute(
            "UPDATE peers SET epoch=9223372036854775808.0 WHERE peer=?1",
            [&a.peer_id().0],
        )
        .unwrap();
    assert!(storage.current_epoch(&device.endpoint_id).is_err());
    private
        .execute(
            "UPDATE peers SET epoch=?1 WHERE peer=?2",
            rusqlite::params![i64::try_from(a.epoch()).unwrap(), a.peer_id().0],
        )
        .unwrap();
    policy(
        storage.authenticated_hello(a.peer_id(), u64::MAX),
        ErrorCode::Unauthorized,
    );
    let change = AttentionTransition {
        target: target(&summary(Provider::Omp)),
        correlation: id("integer boundary"),
        previous: SessionStatus::Idle,
        current: SessionStatus::Attention,
        snapshot: false,
        new_actionable: true,
    };
    for invalid in [-1, i64::try_from(UINT_MAX + 1).unwrap()] {
        private
            .execute(
                "UPDATE peers SET highwater=?1 WHERE peer=?2",
                rusqlite::params![invalid, a.peer_id().0],
            )
            .unwrap();
        assert!(matches!(
            storage.notifications_sync(&a, 0, 100, 1),
            Err(StorageError::Corrupt)
        ));
        assert!(matches!(
            storage.attention_transition(&change, 1),
            Err(StorageError::Corrupt)
        ));
    }
    private
        .execute(
            "UPDATE peers SET highwater=0 WHERE peer=?1",
            [&a.peer_id().0],
        )
        .unwrap();
    let event = storage
        .attention_transition(&change, 1)
        .unwrap()
        .remove(0)
        .1;
    assert_eq!(event.sequence, Uint(1));
    storage
        .notifications_received(&a, std::slice::from_ref(&event))
        .unwrap();
    storage
        .notifications_read(&a, &event.session_id, 1, true)
        .unwrap();
    for invalid in [-1, i64::try_from(UINT_MAX + 1).unwrap()] {
        private
            .execute(
                "UPDATE readwater SET through=?1 WHERE peer=?2",
                rusqlite::params![invalid, a.peer_id().0],
            )
            .unwrap();
        assert!(matches!(
            storage.notifications_sync(&a, 0, 100, 1),
            Err(StorageError::Corrupt)
        ));
        assert!(matches!(
            storage.notifications_read(&a, &event.session_id, 1, true),
            Err(StorageError::Corrupt)
        ));
    }
}
#[test]
fn definitive_transitions_ordered_partial_creation_and_reconciliation() {
    let mut storage = Storage::memory([1; 32], 0).unwrap();
    let a = peer(&mut storage, 11);
    let create = Request {
        id: Uuid::random(),
        body: RequestBody::SessionCreate(SessionCreateParams {
            directory_ref: None,
            placement: None,
            boot_id: storage.boot_id.clone(),
            server_id: Uuid::random(),
            project_id: Uuid::random(),
            provider: Provider::Codex,
            name: Label::new("new session").unwrap(),
        }),
    };
    storage.reserve(&a, &create, 0, |_| Ok(())).unwrap();
    storage
        .transition(
            &a,
            &create.id,
            ReceiptTransition {
                state: ReceiptState::Dispatching,
                proof: TransitionProof::Intent,
                result: None,
                error: None,
                at: 1,
            },
        )
        .unwrap();
    policy(
        storage.creation_intent(
            &a,
            &create.id,
            CreationStage::Thread,
            StageMetadata::default(),
            2,
        ),
        ErrorCode::InvalidRequest,
    );
    storage
        .creation_intent(
            &a,
            &create.id,
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
    storage
        .creation_result(
            &a,
            &create.id,
            CreationOutcome {
                resources: resources.clone(),
                metadata: StageMetadata::default(),
                state: StageState::Confirmed,
                definitive: Some(id("workspace response")),
                at: 3,
            },
        )
        .unwrap();
    confirmed_shell_probe(&mut storage, &a, &create.id, &resources, 4);
    storage
        .creation_intent(
            &a,
            &create.id,
            CreationStage::Owner,
            StageMetadata {
                unit_name: Some("owned-owner.service".into()),
                ..Default::default()
            },
            4,
        )
        .unwrap();
    let uncertain = storage
        .creation_result(
            &a,
            &create.id,
            CreationOutcome {
                resources: resources.clone(),
                metadata: StageMetadata::default(),
                state: StageState::Uncertain,
                definitive: None,
                at: 5,
            },
        )
        .unwrap();
    assert_eq!(uncertain.state, ReceiptState::Uncertain);
    assert_eq!(
        uncertain.creation.as_ref().unwrap().resources.workspace_id,
        resources.workspace_id
    );
    policy(
        storage.creation_intent(
            &a,
            &create.id,
            CreationStage::Owner,
            StageMetadata::default(),
            6,
        ),
        ErrorCode::InvalidRequest,
    );
    storage
        .creation_reconcile(
            &a,
            &create.id,
            CreationReconciliation {
                resources: resources.clone(),
                proof: CreationReconciliationProof::NoEffect(id("owner proved absent")),
                metadata: StageMetadata::default(),
                at: 6,
            },
        )
        .unwrap();
    let failed = storage
        .transition(
            &a,
            &create.id,
            ReceiptTransition {
                state: ReceiptState::Failed,
                proof: TransitionProof::ReconciledNoEffect(id("operator proof")),
                result: None,
                error: Some(WireError {
                    code: ErrorCode::BackendError,
                    message: SafeMessage::new("Native stage was not started").unwrap(),
                }),
                at: 7,
            },
        )
        .unwrap();
    assert_eq!(failed.creation.unwrap().resources, resources);
    let local = storage.creation_local(&a, &create.id).unwrap().unwrap();
    assert_eq!(
        local.stages[0].intent.workspace_operation,
        WorkspaceOperation::Create
    );
    assert_eq!(
        local.stages[1].intent.workspace_operation,
        WorkspaceOperation::ShellProbe
    );
    assert_eq!(local.stages[1].state, StageState::Confirmed);
    let owner = local
        .stages
        .iter()
        .find(|stage| stage.stage == CreationStage::Owner)
        .unwrap();
    assert_eq!(
        owner.intent.unit_name.as_deref(),
        Some("owned-owner.service")
    );
    assert!(local.alias.starts_with("goatr-codex-"));
}
#[test]
fn proven_six_tuple_identity_boot_replacement_and_routing_independence() {
    let root = PrivateRoot::new();
    let path = root.0.join("identity.sqlite");
    let mut storage = Storage::open(&path, [1; 32], 0, root.uid()).unwrap();
    let evidence = ServerEvidence {
        linux_boot_id: Uuid::random(),
        pid: 123,
        pid_start_ticks: 44,
        socket_device: 1,
        socket_inode: 2,
    };
    let server = storage
        .server_identity("/canonical/runtime/herdr.sock:name", &evidence)
        .unwrap();
    let key = StableSessionKey {
        host_id: EndpointId::new(iroh::SecretKey::from_bytes(&[1; 32]).public().to_string())
            .unwrap(),
        server_id: server.server_id.clone(),
        server_incarnation: server.incarnation.clone(),
        terminal_id: id("terminal"),
        provider_owner_identity: id("boot/pid/start/socket owner"),
        native_conversation_id: id("native"),
    };
    let original = storage.session_identity(&key, false).unwrap();
    assert_eq!(original, storage.session_identity(&key, false).unwrap());
    let reconnect = storage.session_identity(&key, true).unwrap();
    assert_eq!(original.session_id, reconnect.session_id);
    assert_ne!(original.generation, reconnect.generation);
    let replaced = storage
        .session_identity(
            &StableSessionKey {
                provider_owner_identity: id("different owner"),
                ..key.clone()
            },
            false,
        )
        .unwrap();
    assert_ne!(original.session_id, replaced.session_id);
    drop(storage);
    let mut reopened = Storage::open(&path, [1; 32], 1, root.uid()).unwrap();
    let restored = reopened.session_identity(&key, false).unwrap();
    assert_eq!(restored.session_id, original.session_id);
    assert_ne!(restored.generation, reconnect.generation);
    let changed = reopened
        .server_identity(
            "/canonical/runtime/herdr.sock:name",
            &ServerEvidence {
                pid_start_ticks: 45,
                ..evidence
            },
        )
        .unwrap();
    assert_eq!(changed.server_id, server.server_id);
    assert_ne!(changed.incarnation, server.incarnation);
}
#[test]
fn durable_attention_no_snapshot_completion_received_view_guard_retention_and_revocation() {
    let root = PrivateRoot::new();
    let path = root.0.join("attention.sqlite");
    let mut storage = Storage::open(&path, [1; 32], 0, root.uid()).unwrap();
    let a = peer(&mut storage, 12);
    let b = peer(&mut storage, 13);
    let target = target(&summary(Provider::Codex));
    let snapshot = AttentionTransition {
        target: target.clone(),
        correlation: id("initial"),
        previous: SessionStatus::Working,
        current: SessionStatus::Idle,
        snapshot: true,
        new_actionable: false,
    };
    assert!(
        storage
            .attention_transition(&snapshot, 0)
            .unwrap()
            .is_empty()
    );
    storage
        .attention_transition(
            &AttentionTransition {
                current: SessionStatus::Working,
                previous: SessionStatus::Idle,
                snapshot: false,
                correlation: id("working"),
                ..snapshot.clone()
            },
            1,
        )
        .unwrap();
    let completed = AttentionTransition {
        snapshot: false,
        correlation: id("native completion"),
        ..snapshot.clone()
    };
    let events = storage.attention_transition(&completed, 2).unwrap();
    assert_eq!(events.len(), 2);
    let event = events
        .iter()
        .find(|(p, _)| p == a.peer_id())
        .unwrap()
        .1
        .clone();
    assert!(
        storage
            .attention_transition(&completed, 3)
            .unwrap()
            .is_empty()
    );
    policy(
        storage.notifications_read(&a, &target.session_id, event.sequence.0, true),
        ErrorCode::InvalidRequest,
    );
    storage
        .notifications_received(&a, std::slice::from_ref(&event))
        .unwrap();
    policy(
        storage.notifications_read(&a, &target.session_id, event.sequence.0, false),
        ErrorCode::InvalidRequest,
    );
    assert_eq!(
        storage
            .notifications_read(&a, &target.session_id, event.sequence.0, true)
            .unwrap()
            .through_sequence,
        event.sequence
    );
    let wrong = Uuid::random();
    policy(
        storage.notifications_read(&a, &wrong, event.sequence.0, true),
        ErrorCode::InvalidRequest,
    );
    drop(storage);
    let mut reopened = Storage::open(&path, [1; 32], 4, root.uid()).unwrap();
    assert!(
        reopened
            .attention_transition(&completed, 4)
            .unwrap()
            .is_empty()
    );
    let synced = reopened.notifications_sync(&a, 0, 100, 4).unwrap();
    assert_eq!(synced.events[0].event_id, event.event_id);
    assert_eq!(synced.latest_sequence, event.sequence);
    let expired = reopened
        .notifications_sync(&a, 0, 100, RETENTION_MS + 3)
        .unwrap();
    assert!(expired.reset_required);
    assert!(expired.events.is_empty());
    assert_eq!(expired.latest_sequence, event.sequence);
    reopened
        .revoke_epoch(a.peer_id(), RETENTION_MS + 3)
        .unwrap();
    policy(
        reopened.notifications_sync(&a, 0, 100, RETENTION_MS + 3),
        ErrorCode::Unauthorized,
    );
    assert!(reopened.notifications_sync(&b, 0, 100, 4).is_ok());
}
#[test]
fn receipt_capacity_expired_only_and_retired_generations_survive_without_plaintext_registration() {
    let mut storage = Storage::memory([1; 32], 0).unwrap();
    let a = peer(&mut storage, 14);
    let target = target(&summary(Provider::Omp));
    for _ in 0..PEER_CAPACITY {
        let request = prompt(&storage, &target, "not persisted");
        storage.reserve(&a, &request, 0, |_| Ok(())).unwrap();
        storage
            .transition(
                &a,
                &request.id,
                ReceiptTransition {
                    state: ReceiptState::Failed,
                    proof: TransitionProof::DefinitiveNoEffect(id("never entered write")),
                    result: None,
                    error: None,
                    at: 0,
                },
            )
            .unwrap();
    }
    let request = prompt(&storage, &target, "capacity");
    policy(
        storage.reserve(&a, &request, RETENTION_MS - 1, |_| Ok(())),
        ErrorCode::ActionCapacity,
    );
    assert!(matches!(
        storage
            .reserve(&a, &request, RETENTION_MS, |_| Ok(()))
            .unwrap(),
        Reservation::Reserved(_)
    ));
    let generation = Uuid::random();
    storage
        .encrypted_registration(
            &a,
            &generation,
            b"test cipher supplied by encryption owner",
            0,
        )
        .unwrap();
    storage.retire_push_generation(&a, &generation, 0).unwrap();
    assert!(storage.push_generation_retired(&a, &generation).unwrap());
    policy(
        storage.encrypted_registration(&a, &generation, b"cipher", 0),
        ErrorCode::StaleRegistration,
    );
}

#[test]
fn byte_overflow_preserves_order_and_queues_sync_only_when_it_fits() {
    let mut storage = Storage::memory([1; 32], 0).unwrap();
    let peer = peer(&mut storage, 15);
    let state = State::default();
    let session = summary(Provider::Codex);
    let target = target(&session);
    install(&state, &session, vec![]);
    let connection = state.connect(&peer);
    state
        .session_open(&connection, Uuid::random(), &target, 0)
        .unwrap();
    state.pop_output(&connection).unwrap();
    let mut large = session.clone();
    large.summary.title = "x".repeat(900_000);
    for digit in '0'..='4' {
        large.summary.title.pop().unwrap();
        large.summary.title.push(digit);
        state
            .update(&target, SessionUpdate::State(large.clone()))
            .unwrap();
    }
    for n in 1..=4 {
        let (bytes, closed) = state.pop_output(&connection).unwrap();
        assert!(closed);
        let Envelope::Event(event) = decode_payload(&bytes.unwrap(), None).unwrap() else {
            panic!()
        };
        assert_eq!(event.seq, Uint(n));
        assert!(matches!(event.body, EventBody::SessionState(_)));
    }
    let Envelope::Event(sync) =
        decode_payload(&state.pop_output(&connection).unwrap().0.unwrap(), None).unwrap()
    else {
        panic!()
    };
    assert_eq!(sync.seq, Uint(5));
    assert!(matches!(
        sync.body,
        EventBody::SyncRequired(SyncRequiredEvent {
            reason: SyncReason::Overflow
        })
    ));
}

#[test]
fn pending_capacity_native_snapshot_overflow_and_generation_context_fail_closed() {
    let mut storage = Storage::memory([1; 32], 0).unwrap();
    let peer = peer(&mut storage, 16);
    let state = State::default();
    let session = summary(Provider::Omp);
    let target = target(&session);
    install(&state, &session, vec![]);
    for _ in 0..64 {
        state
            .update(
                &target,
                SessionUpdate::Open(PendingRequest {
                    request_key: Uuid::random(),
                    provider: Provider::Omp,
                    kind: PresentationKind::OmpEditor,
                    presentation: Presentation::OmpEditor(PresentationOmpEditor {
                        title: Content::Text(ContentText {
                            text: InlineText::new("editor").unwrap(),
                        }),
                        prefill: Content::Text(ContentText {
                            text: InlineText::new("").unwrap(),
                        }),
                    }),
                }),
            )
            .unwrap();
    }
    let excess = PendingRequest {
        request_key: Uuid::random(),
        provider: Provider::Omp,
        kind: PresentationKind::DesktopOnly,
        presentation: Presentation::DesktopOnly(PresentationDesktopOnly {
            title: Content::Text(ContentText {
                text: InlineText::new("desktop").unwrap(),
            }),
            reason: "capacity".into(),
        }),
    };
    assert_eq!(
        state.update(&target, SessionUpdate::Open(excess)),
        Err(ErrorCode::ResourceLimit)
    );
    assert_eq!(
        state.begin_install(session.clone()),
        Err(ErrorCode::StaleSession)
    );
    let source = state.begin_source_install(&target).unwrap();
    for _ in 0..256 {
        state
            .update(
                &target,
                SessionUpdate::Close(Uuid::random(), RequestClosedReason::Resolved),
            )
            .unwrap();
    }
    assert_eq!(
        state.update(
            &target,
            SessionUpdate::Close(Uuid::random(), RequestClosedReason::Resolved)
        ),
        Err(ErrorCode::ResourceLimit)
    );
    assert_eq!(
        state.finish_install(
            &source,
            ProviderSnapshot {
                state: session.clone(),
                pending: vec![],
                history: vec![],
                older_available: false,
                native_older: None
            }
        ),
        Err(ErrorCode::ResourceLimit)
    );
    state.abort_install(&target, &source).unwrap();
    let mut replacement = session.clone();
    replacement.summary.generation = Uuid::random();
    install(&state, &replacement, vec![]);
    assert_eq!(
        state.abort_install(&target, &source),
        Err(ErrorCode::StaleSession)
    );
    assert_eq!(
        state.finish_install(
            &source,
            ProviderSnapshot {
                state: session.clone(),
                pending: vec![],
                history: vec![],
                older_available: false,
                native_older: None,
            }
        ),
        Err(ErrorCode::StaleSession)
    );
    assert_eq!(
        state.validate_session_mutation(&prompt(&storage, &target, "old context")),
        Err(ErrorCode::StaleSession)
    );
    let replacement_target = Target {
        session_id: replacement.summary.session_id.clone(),
        generation: replacement.summary.generation.clone(),
    };
    let connection = state.connect(&peer);
    state
        .session_open(&connection, Uuid::random(), &replacement_target, 0)
        .unwrap();
}

#[test]
fn native_history_cursor_retries_are_scoped_stable_and_invalidated() {
    let mut storage = Storage::memory([1; 32], 0).unwrap();
    let a = peer(&mut storage, 17);
    let b = peer(&mut storage, 18);
    let state = State::default();
    let session = summary(Provider::Codex);
    let target = target(&session);
    install(&state, &session, vec![]);
    let source = state.source_epoch(&target).unwrap();
    assert_eq!(
        state.native_history_cursor(
            &a,
            &target,
            &Uuid::random(),
            id("actual native pagination continuation"),
            None,
            0,
        ),
        Err(ErrorCode::CursorExpired)
    );
    let token = state
        .native_history_cursor(
            &a,
            &target,
            &source,
            id("actual native pagination continuation"),
            None,
            0,
        )
        .unwrap();
    assert!(matches!(
        state.native_history_page(&b, &target, &token, 100, 0),
        Err(ErrorCode::CursorExpired)
    ));
    let NativePageLookup::Fetch(ticket) = state
        .native_history_page(&a, &target, &token, 100, 0)
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(
        ticket.native_cursor(),
        &id("actual native pagination continuation")
    );
    let page = state
        .finish_native_history_page(
            ticket,
            vec![record(1, RecordSource::NativePage, "old native")],
            Some(id("next native")),
            0,
        )
        .unwrap();
    let NativePageLookup::Cached(retry) = state
        .native_history_page(&a, &target, &token, 100, 1)
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(retry, page);
    assert_eq!(
        state
            .native_history_cursor(
                &a,
                &target,
                &source,
                id("actual native pagination continuation"),
                None,
                1,
            )
            .unwrap(),
        token
    );
    state.update(&target, SessionUpdate::SourceGap).unwrap();
    assert!(matches!(
        state.native_history_page(&a, &target, &token, 100, 1),
        Err(ErrorCode::CursorExpired)
    ));
}

#[tokio::test]
async fn bounded_sqlite_owner_serializes_commits_off_the_async_reader() {
    let owner = StorageOwner::new(Storage::memory([1; 32], 0).unwrap()).unwrap();
    let a = owner.call(|store| Ok(peer(store, 19))).await.unwrap();
    let copy = a.clone();
    owner
        .call(move |store| {
            store.recheck(&copy)?;
            store.revoke_epoch(copy.peer_id(), 0)
        })
        .await
        .unwrap();
    assert!(matches!(
        owner.call(move |store| store.recheck(&a)).await,
        Err(StorageError::Policy(ErrorCode::Unauthorized))
    ));
    owner.close().await.unwrap();
}

#[test]
fn entered_boundary_settles_after_revoke_without_revoked_receipt_access() {
    let root = PrivateRoot::new();
    let path = root.0.join("fence.sqlite");
    let mut storage = Storage::open(&path, [1; 32], 0, root.uid()).unwrap();
    let a = peer(&mut storage, 20);
    let request = prompt(&storage, &target(&summary(Provider::Omp)), "never stored");
    storage.reserve(&a, &request, 0, |_| Ok(())).unwrap();
    storage
        .transition(
            &a,
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
    let ticket = storage.dispatch_ticket(&a, &request.id).unwrap();
    storage.revoke_epoch(a.peer_id(), 2).unwrap();
    assert!(matches!(
        storage.dispatch_ticket(&a, &request.id),
        Err(StorageError::Policy(ErrorCode::Unauthorized))
    ));
    storage
        .settle_dispatch(
            ticket,
            ReceiptTransition {
                state: ReceiptState::Uncertain,
                proof: TransitionProof::LostOutcome,
                result: None,
                error: None,
                at: 2,
            },
        )
        .unwrap();
    policy(storage.status(&a, &request.id, 2), ErrorCode::Unauthorized);
    let private = rusqlite::Connection::open(&path).unwrap();
    let state: String = private
        .query_row(
            "SELECT state FROM receipts WHERE action=?1",
            [&request.id.0],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "uncertain");
}

#[test]
fn confirmed_entered_creation_resources_survive_revocation_and_no_next_stage() {
    let root = PrivateRoot::new();
    let path = root.0.join("partial.sqlite");
    let mut storage = Storage::open(&path, [1; 32], 0, root.uid()).unwrap();
    let a = peer(&mut storage, 21);
    let request = Request {
        id: Uuid::random(),
        body: RequestBody::SessionCreate(SessionCreateParams {
            directory_ref: None,
            placement: None,
            boot_id: storage.boot_id.clone(),
            server_id: Uuid::random(),
            project_id: Uuid::random(),
            provider: Provider::Codex,
            name: Label::new("created").unwrap(),
        }),
    };
    storage.reserve(&a, &request, 0, |_| Ok(())).unwrap();
    storage
        .transition(
            &a,
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
    storage
        .creation_intent(
            &a,
            &request.id,
            CreationStage::Workspace,
            StageMetadata::default(),
            2,
        )
        .unwrap();
    let ticket = storage.dispatch_ticket(&a, &request.id).unwrap();
    storage.revoke_epoch(a.peer_id(), 3).unwrap();
    let resources = CreationResources {
        checkout: None,
        workspace_id: Some(id("confirmed workspace")),
        pane_id: Some(id("confirmed pane")),
        terminal_id: Some(id("confirmed terminal")),
        native_id: None,
        binding_id: None,
        session_id: None,
        generation: None,
        owner_started: None,
        relay_started: None,
    };
    storage
        .settle_creation_stage(
            ticket,
            CreationOutcome {
                resources: resources.clone(),
                metadata: StageMetadata::default(),
                state: StageState::Confirmed,
                definitive: Some(id("native workspace response")),
                at: 3,
            },
        )
        .unwrap();
    policy(
        storage.creation_intent(
            &a,
            &request.id,
            CreationStage::Owner,
            StageMetadata::default(),
            4,
        ),
        ErrorCode::Unauthorized,
    );
    let private = rusqlite::Connection::open(&path).unwrap();
    let data: String = private
        .query_row(
            "SELECT receipt FROM receipts WHERE action=?1",
            [&request.id.0],
            |r| r.get(0),
        )
        .unwrap();
    let receipt: Receipt = serde_json::from_str(&data).unwrap();
    assert_eq!(receipt.state, ReceiptState::Failed);
    assert_eq!(receipt.creation.unwrap().resources, resources);
}

#[test]
fn notification_read_capacity_does_not_suppress_unviewed_attention_and_pair_cap_is_atomic() {
    let mut storage = Storage::memory([1; 32], 0).unwrap();
    let a = peer(&mut storage, 22);
    for n in 0..513 {
        let change = AttentionTransition {
            target: Target {
                session_id: Uuid::random(),
                generation: Uuid::random(),
            },
            correlation: id(&format!("attention-{n}")),
            previous: SessionStatus::Idle,
            current: SessionStatus::Attention,
            snapshot: false,
            new_actionable: true,
        };
        let event = storage
            .attention_transition(&change, 0)
            .unwrap()
            .pop()
            .unwrap()
            .1;
        storage
            .notifications_received(&a, std::slice::from_ref(&event))
            .unwrap();
        let read = storage.notifications_read(&a, &event.session_id, event.sequence.0, true);
        if n < 512 {
            assert_eq!(read.unwrap().through_sequence, event.sequence);
        } else {
            policy(read, ErrorCode::ResourceLimit);
        }
    }
    assert_eq!(
        storage
            .notifications_sync(&a, 0, 100, 0)
            .unwrap()
            .latest_sequence,
        Uint(513)
    );
    for seed in 23..54 {
        peer(&mut storage, seed);
    }
    let device = PairedDevice {
        device_id: Uuid::random(),
        endpoint_id: EndpointId::new(iroh::SecretKey::from_bytes(&[54; 32]).public().to_string())
            .unwrap(),
        device_name: Label::new("capacity").unwrap(),
        paired_at: Uint(0),
        last_seen_at: None,
    };
    policy(storage.pair_device(&device), ErrorCode::ResourceLimit);
    assert_eq!(storage.devices().unwrap().len(), 32);
}

#[test]
fn native_projected_page_is_split_without_clipping_and_old_source_cannot_append() {
    let mut storage = Storage::memory([1; 32], 0).unwrap();
    let a = peer(&mut storage, 55);
    let state = State::default();
    let session = summary(Provider::Codex);
    let target = target(&session);
    install(&state, &session, vec![]);
    let source = state.source_epoch(&target).unwrap();
    let token = state
        .native_history_cursor(&a, &target, &source, id("actual native cursor"), None, 0)
        .unwrap();
    let NativePageLookup::Fetch(ticket) = state
        .native_history_page(&a, &target, &token, 100, 0)
        .unwrap()
    else {
        panic!()
    };
    let page = state
        .finish_native_history_page(
            ticket,
            (0..150)
                .map(|n| record(n, RecordSource::NativePage, "native item"))
                .collect(),
            None,
            0,
        )
        .unwrap();
    assert_eq!(page.records.len(), 100);
    assert_eq!(page.records[0].order, Uint(50));
    let older = state
        .history(&a, &target, page.next_cursor.as_ref(), 100, 0)
        .unwrap();
    assert_eq!(older.result.records.len(), 50);
    assert!(older.result.next_cursor.is_none());
    assert_eq!(older.result.records[0].order, Uint(0));
    state.update(&target, SessionUpdate::SourceGap).unwrap();
    assert_eq!(
        state.update_from_source(
            &target,
            &source,
            SessionUpdate::History(vec![record(151, RecordSource::NativePage, "old source")])
        ),
        Err(ErrorCode::CursorExpired)
    );
}

#[test]
fn global_content_lru_is_bounded_without_evicting_the_recently_copied_payload() {
    let mut storage = Storage::memory([1; 32], 0).unwrap();
    let a = peer(&mut storage, 56);
    let state = State::default();
    let mut handles = vec![];
    let mut targets = vec![];
    for _ in 0..4 {
        let session = summary(Provider::Omp);
        let t = target(&session);
        install(&state, &session, vec![]);
        targets.push(t.clone());
        for _ in 0..4 {
            let Content::Ref(r) = state
                .content(&t, Mime::TextPlain, "x".repeat(CONTENT_MAX as usize), 0)
                .unwrap()
            else {
                panic!()
            };
            handles.push(r.handle);
        }
    }
    let first = ContentReadParams {
        target: targets[0].clone(),
        handle: handles[0].clone(),
        offset_bytes: Uint(0),
        limit_bytes: Uint(1),
    };
    state.read_content(&a, &first, 1).unwrap();
    let session = summary(Provider::Omp);
    let t = target(&session);
    install(&state, &session, vec![]);
    state
        .content(&t, Mime::TextPlain, "x".repeat(CONTENT_MAX as usize), 2)
        .unwrap();
    assert!(state.read_content(&a, &first, 2).is_ok());
    assert_eq!(
        state
            .read_content(
                &a,
                &ContentReadParams {
                    handle: handles[1].clone(),
                    ..first
                },
                2
            )
            .err(),
        Some(ErrorCode::ContentExpired)
    );
}

#[test]
fn complete_creation_requires_every_ordered_confirmation_and_exact_resulting_binding() {
    complete_creation(Provider::Omp, true, false);
    complete_creation(Provider::Codex, true, false);
}

#[test]
fn codex_existing_ready_owner_is_reused_without_claiming_it_was_started() {
    complete_creation(Provider::Codex, false, true);
}

fn complete_creation(provider: Provider, owner_started: bool, reconcile: bool) {
    let mut storage = Storage::memory([1; 32], 0).unwrap();
    let a = peer(&mut storage, 57);
    let request = Request {
        id: Uuid::random(),
        body: RequestBody::SessionCreate(SessionCreateParams {
            directory_ref: None,
            placement: None,
            boot_id: storage.boot_id.clone(),
            server_id: Uuid::random(),
            project_id: Uuid::random(),
            provider,
            name: Label::new("complete").unwrap(),
        }),
    };
    storage.reserve(&a, &request, 0, |_| Ok(())).unwrap();
    storage
        .transition(
            &a,
            &request.id,
            ReceiptTransition {
                state: ReceiptState::Dispatching,
                proof: TransitionProof::Intent,
                result: None,
                error: None,
                at: 0,
            },
        )
        .unwrap();
    let mut resources = CreationResources {
        checkout: None,
        workspace_id: None,
        pane_id: None,
        terminal_id: None,
        native_id: None,
        binding_id: None,
        session_id: None,
        generation: None,
        owner_started: None,
        relay_started: None,
    };
    let stages = if provider == Provider::Codex {
        vec![
            CreationStage::Workspace,
            CreationStage::Owner,
            CreationStage::Thread,
            CreationStage::Relay,
            CreationStage::Launch,
            CreationStage::Ready,
        ]
    } else {
        vec![
            CreationStage::Workspace,
            CreationStage::Relay,
            CreationStage::Launch,
            CreationStage::Ready,
        ]
    };
    for stage in stages {
        storage
            .creation_intent(&a, &request.id, stage, StageMetadata::default(), 1)
            .unwrap();
        match stage {
            CreationStage::Workspace => {
                resources.workspace_id = Some(id("workspace"));
                resources.pane_id = Some(id("pane"));
                resources.terminal_id = Some(id("terminal"));
            }
            CreationStage::Owner => resources.owner_started = Some(owner_started),
            CreationStage::Thread | CreationStage::Launch => {
                resources.native_id = Some(id("native"))
            }
            CreationStage::Relay => {
                resources.relay_started = Some(true);
                if provider == Provider::Codex {
                    resources.binding_id = Some(Uuid::random());
                }
            }
            CreationStage::Ready => {
                resources.session_id = Some(Uuid::random());
                resources.generation = Some(Uuid::random());
            }
        }
        storage
            .creation_result(
                &a,
                &request.id,
                CreationOutcome {
                    resources: resources.clone(),
                    metadata: StageMetadata::default(),
                    state: StageState::Confirmed,
                    definitive: Some(id("correlated native confirmation")),
                    at: 1,
                },
            )
            .unwrap();
        if stage == CreationStage::Workspace {
            confirmed_shell_probe(&mut storage, &a, &request.id, &resources, 1);
        }
    }
    if reconcile {
        storage
            .transition(
                &a,
                &request.id,
                ReceiptTransition {
                    state: ReceiptState::Uncertain,
                    proof: TransitionProof::LostOutcome,
                    result: None,
                    error: None,
                    at: 2,
                },
            )
            .unwrap();
        storage
            .creation_reconcile(
                &a,
                &request.id,
                CreationReconciliation {
                    resources: resources.clone(),
                    proof: CreationReconciliationProof::Ready(id("exact reused owner binding")),
                    metadata: StageMetadata::default(),
                    at: 3,
                },
            )
            .unwrap();
    }
    let completed_at = if reconcile { 4 } else { 2 };
    let created = ActionResultCreated {
        target: Target {
            session_id: resources.session_id.clone().unwrap(),
            generation: resources.generation.clone().unwrap(),
        },
        workspace_id: resources.workspace_id.clone().unwrap(),
        terminal_id: resources.terminal_id.clone().unwrap(),
        pane_id: resources.pane_id.clone().unwrap(),
        native_id: resources.native_id.clone().unwrap(),
        binding_id: resources.binding_id.clone(),
    };
    let mut wrong = created.clone();
    wrong.target.session_id = Uuid::random();
    policy(
        storage.transition(
            &a,
            &request.id,
            ReceiptTransition {
                state: ReceiptState::Succeeded,
                proof: TransitionProof::CorrelatedAcknowledgement(id("ready binding")),
                result: Some(ActionResult::Created(Box::new(wrong))),
                error: None,
                at: completed_at,
            },
        ),
        ErrorCode::InvalidRequest,
    );
    let succeeded = storage
        .transition(
            &a,
            &request.id,
            ReceiptTransition {
                state: ReceiptState::Succeeded,
                proof: if reconcile {
                    TransitionProof::ReconciledSuccess(id("ready reused owner binding"))
                } else {
                    TransitionProof::CorrelatedAcknowledgement(id("ready binding"))
                },
                result: Some(ActionResult::Created(Box::new(created))),
                error: None,
                at: completed_at,
            },
        )
        .unwrap();
    assert_eq!(succeeded.state, ReceiptState::Succeeded);
    assert_eq!(
        succeeded.creation.as_ref().unwrap().resources.owner_started,
        (provider == Provider::Codex).then_some(owner_started)
    );
}
