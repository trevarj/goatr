// SPDX-License-Identifier: GPL-3.0-or-later
//! Deterministic native-contract consumers, not stock-host/live acceptance.
use goatr::{
    backends::codex::{project_item, project_request},
    native_json::{self, NATIVE},
    protocol::*,
    state::{ProviderSnapshot, SessionUpdate, State},
};
use serde_json::{Value, json};
fn fixture() -> (State, Target, Uuid) {
    let state = State::default();
    let target = Target {
        session_id: Uuid::random(),
        generation: Uuid::random(),
    };
    let display = SessionState {
        summary: SessionSummary {
            session_id: target.session_id.clone(),
            generation: target.generation.clone(),
            server_id: Uuid::random(),
            workspace_id: Id("workspace".into()),
            terminal_id: Id("terminal".into()),
            pane_id: Id("pane".into()),
            provider: Provider::Codex,
            native_id: Id("T".into()),
            title: "native contract".into(),
            status: SessionStatus::Idle,
            connectivity: SessionConnectivity::Ready,
            capabilities: Capabilities { controls: false, terminal_create:false,artifacts:false, attachments:false, history: true,
            prompt: true,
            cancel: true,
            respond: true,
            prompt_mode: PromptMode::SubmitOrSteer,
            cancel_mode: CancelMode::NativeTurn,
            terminal_read: false,
            worktrees_read: false,
            changes_read: false,
            workspace_rename: false, agent_rename: false, skills_read: false, },
            skills_generation: None,
            history_coverage: HistoryCoverage {
                scope: HistoryScope::Thread,
                source: HistorySource::NativePages,
                older_available: true,
                source_truncated: false,
                desktop_branch_known: false,
            },
            reason: None,
        },
        native_turn_id: None,
        desktop_selection: DesktopSelection::BoundLaunchThread,
        activity: Activity {
            state: ActivityState::Idle,
            text: None,
            tool_record_id: None,
        },
    };
    let source = state.begin_install(display.clone()).unwrap();
    state
        .finish_install(
            &source,
            ProviderSnapshot {
                state: display,
                pending: vec![],
                history: vec![],
                older_available: true,
                native_older: None,
            },
        )
        .unwrap();
    (state, target, source)
}
fn request(method: &str, params: Value) -> Value {
    let mut frame = json!({"id":-987654321,"method":method,"params":{}});
    frame["params"] = params;
    frame
}
fn command() -> Value {
    json!({"threadId":"T","turnId":"turn","itemId":"item","startedAtMs":-1,"command":"read-only command","cwd":"/work"})
}
fn approval_result(dialog: &goatr::backends::codex::Dialog, index: usize) -> Value {
    native_json::parse(
        &dialog
            .response(&Answer::CodexApproval(AnswerCodexApproval {
                choice_id: Id(format!("choice-{index}")),
            }))
            .unwrap(),
        NATIVE,
    )
    .unwrap()
}
#[test]
fn ordered_authoritative_membership_rejects_unknown_empty_and_unoffered_grants() {
    let (state, target, _) = fixture();
    let mut p = command();
    p["networkApprovalContext"] = json!({"host":"example.test","protocol":"https"});
    p["additionalPermissions"] = json!({"network":{"enabled":true}});
    p["proposedExecpolicyAmendment"] = json!(["never", "offered"]);
    p["availableDecisions"] = json!(["cancel",{"applyNetworkPolicyAmendment":{"network_policy_amendment":{"host":"example.test","action":"deny"}}},"acceptForSession"]);
    let d = project_request(
        &state,
        &target,
        &request("item/commandExecution/requestApproval", p.clone()),
        None,
    )
    .unwrap();
    let Presentation::CodexApproval(card) = &d.pending.presentation else {
        panic!("finite card")
    };
    assert_eq!(card.choices.len(), 3);
    assert_eq!(card.choices[1].scope, ApprovalScope::Policy);
    assert_eq!(card.choices[2].scope, ApprovalScope::Session);
    assert_eq!(approval_result(&d, 0)["result"]["decision"], "cancel");
    assert_eq!(
        approval_result(&d, 1)["result"]["decision"],
        p["availableDecisions"][1]
    );
    assert!(
        d.response(&Answer::CodexApproval(AnswerCodexApproval {
            choice_id: Id("choice-3".into())
        }))
        .is_err()
    );
    for decisions in [
        json!([]),
        json!(["accept", "unknownFutureGrant"]),
        json!([{"acceptWithExecpolicyAmendment":{"execpolicy_amendment":[1]}}]),
        Value::Null,
    ] {
        p["availableDecisions"] = decisions;
        let d = project_request(
            &state,
            &target,
            &request("item/commandExecution/requestApproval", p.clone()),
            None,
        )
        .unwrap();
        assert_eq!(d.pending.kind, PresentationKind::DesktopOnly);
    }
}
#[test]
fn absent_membership_uses_native_scope_precedence_not_blanket_approval_choices() {
    let (state, target, _) = fixture();
    let mut p = command();
    p["proposedExecpolicyAmendment"] = json!(["exact", "arguments"]);
    let d = project_request(
        &state,
        &target,
        &request("item/commandExecution/requestApproval", p.clone()),
        None,
    )
    .unwrap();
    assert_eq!(
        approval_result(&d, 1)["result"]["decision"]["acceptWithExecpolicyAmendment"]["execpolicy_amendment"],
        p["proposedExecpolicyAmendment"]
    );
    assert_eq!(approval_result(&d, 2)["result"]["decision"], "cancel");
    p["additionalPermissions"] = json!({"network":{"enabled":false}});
    let d = project_request(
        &state,
        &target,
        &request("item/commandExecution/requestApproval", p.clone()),
        None,
    )
    .unwrap();
    assert_eq!(approval_result(&d, 1)["result"]["decision"], "cancel");
    assert!(
        d.response(&Answer::CodexApproval(AnswerCodexApproval {
            choice_id: Id("choice-2".into())
        }))
        .is_err()
    );
    p["networkApprovalContext"] = json!({"host":"network.test","protocol":"socks5Tcp"});
    p["proposedNetworkPolicyAmendments"] = json!([{"host":"deny.test","action":"deny"},{"host":"first.test","action":"allow"},{"host":"second.test","action":"allow"}]);
    let d = project_request(
        &state,
        &target,
        &request("item/commandExecution/requestApproval", p),
        None,
    )
    .unwrap();
    assert_eq!(
        approval_result(&d, 1)["result"]["decision"],
        "acceptForSession"
    );
    assert_eq!(
        approval_result(&d, 2)["result"]["decision"]["applyNetworkPolicyAmendment"]["network_policy_amendment"]
            ["host"],
        "first.test"
    );
    assert_eq!(approval_result(&d, 3)["result"]["decision"], "cancel");
}
#[test]
fn complete_permission_profiles_and_file_scope_are_not_dropped_or_broadened() {
    let (state, target, _) = fixture();
    let permissions = json!({"network":{"enabled":true},"fileSystem":{"read":["/exact/read"],"write":["/exact/write"],"globScanMaxDepth":2,"entries":[{"access":"deny","path":{"type":"path","path":"/deny"}},{"access":"read","path":{"type":"glob_pattern","pattern":"/tree/**"}},{"access":"write","path":{"type":"special","value":{"kind":"project_roots","subpath":"subdir"}}},{"access":"read","path":{"type":"special","value":{"kind":"unknown","path":"/root","subpath":"sub"}}}]}});
    let mut p = json!({"threadId":"T","turnId":"turn","itemId":"item","startedAtMs":0,"cwd":"/work","permissions":permissions});
    let d = project_request(
        &state,
        &target,
        &request("item/permissions/requestApproval", p.clone()),
        None,
    )
    .unwrap();
    let result = approval_result(&d, 2);
    assert_eq!(result["result"]["permissions"], p["permissions"]);
    assert_eq!(result["result"]["scope"], "session");
    assert!(result["result"].get("strictAutoReview").is_none());
    assert_eq!(
        approval_result(&d, 0)["result"],
        json!({"permissions":{},"scope":"turn"})
    );
    p["permissions"]["fileSystem"]["entries"][0]["path"]["futureConstraint"] = json!(true);
    assert_eq!(
        project_request(
            &state,
            &target,
            &request("item/permissions/requestApproval", p),
            None
        )
        .unwrap()
        .pending
        .kind,
        PresentationKind::DesktopOnly
    );
    let p = json!({"threadId":"T","turnId":"turn","itemId":"file","startedAtMs":0,"grantRoot":"/suggested"});
    let file = json!({"id":"file","type":"fileChange","changes":[{"path":"/exact/file","diff":"old→new"}]});
    let d = project_request(
        &state,
        &target,
        &request("item/fileChange/requestApproval", p),
        Some(&file),
    )
    .unwrap();
    assert_eq!(
        approval_result(&d, 1)["result"]["decision"],
        "acceptForSession"
    );
    let Presentation::CodexApproval(card) = d.pending.presentation else {
        panic!()
    };
    assert!(
        serde_json::to_string(&card.details)
            .unwrap()
            .contains("old→new")
    );
}
#[test]
fn all_standard_form_types_preserve_numeric_tokens_literal_ids_and_repeated_options() {
    let (state, target, _) = fixture();
    let frame=native_json::parse(br#"{"id":-123456789012345678901234567890,"method":"mcpServer/elicitation/request","params":{"threadId":"T","turnId":null,"serverName":"native","mode":"form","message":"Provide exact values","requestedSchema":{"type":"object","required":["n","i"],"properties":{"n":{"type":"number","minimum":-1.250e2,"maximum":1e3,"default":0.123456789012345678901234567890},"i":{"type":"integer","minimum":-20,"maximum":30,"default":2e1},"s":{"type":"string","minLength":2,"maxLength":8,"default":"hello"},"b":{"type":"boolean","default":false},"one":{"type":"string","oneOf":[{"const":"x","title":"X"},{"const":"y","title":"Y"}],"default":"y"},"old":{"type":"string","enum":["a","b"],"enumNames":["A","B"]},"multi":{"type":"array","minItems":1,"maxItems":3,"items":{"oneOf":[{"const":"v","title":"V"},{"const":"v","title":"V again"}]},"default":["v","v"]},"$serde_json::private::Number":{"type":"string","enum":["literal"]}}}}}"#,NATIVE).unwrap();
    let d = project_request(&state, &target, &frame, None).unwrap();
    let Presentation::CodexForm(form) = &d.pending.presentation else {
        panic!("finite standard form")
    };
    let n = form.fields.iter().find(|f| f.id.0 == "n").unwrap();
    assert_eq!(n.minimum.as_ref().unwrap().0, "-125");
    assert_eq!(n.maximum.as_ref().unwrap().0, "1000");
    assert_eq!(
        n.default,
        Some(FieldValue::String("0.12345678901234567890123456789".into()))
    );
    let multi = form.fields.iter().find(|f| f.id.0 == "multi").unwrap();
    assert_eq!(multi.options.as_ref().unwrap().len(), 2);
    assert_eq!(
        multi.default,
        Some(FieldValue::Multi(vec!["v".into(), "v".into()]))
    );
    let answer = Answer::CodexForm(AnswerCodexForm {
        action: FormAction::Accept,
        values: Some(vec![
            FormAnswerValue {
                id: Id("n".into()),
                value: FieldValue::String("0.12345678901234567890123456789".into()),
            },
            FormAnswerValue {
                id: Id("i".into()),
                value: FieldValue::String("20".into()),
            },
            FormAnswerValue {
                id: Id("$serde_json::private::Number".into()),
                value: FieldValue::String("literal".into()),
            },
            FormAnswerValue {
                id: Id("multi".into()),
                value: FieldValue::Multi(vec!["v".into(), "v".into()]),
            },
        ]),
    });
    let bytes = d.response(&answer).unwrap();
    let result = native_json::parse(&bytes, NATIVE).unwrap();
    assert_eq!(result["id"].to_string(), "-123456789012345678901234567890");
    assert!(result["result"]["content"].is_object());
    assert_eq!(
        result["result"]["content"]["$serde_json::private::Number"],
        "literal"
    );
    assert_eq!(
        result["result"]["content"]["n"].to_string(),
        "0.12345678901234567890123456789"
    );
    assert_eq!(result["result"]["content"]["multi"], json!(["v", "v"]));
    let declined = native_json::parse(
        &d.response(&Answer::CodexForm(AnswerCodexForm {
            action: FormAction::Decline,
            values: None,
        }))
        .unwrap(),
        NATIVE,
    )
    .unwrap();
    assert!(declined["result"].get("content").is_none());
    assert!(declined["result"].get("_meta").is_none());
}
#[test]
fn unknown_constraints_invalid_defaults_and_nonstandard_modes_fail_whole_card() {
    let (state, target, _) = fixture();
    for schema in [
        json!({"type":"object","properties":{"known":{"type":"string"},"unknown":{"type":"object"}}}),
        json!({"type":"object","properties":{"x":{"type":"string","pattern":".*"}}}),
        json!({"type":"object","properties":{"x":{"type":"integer","default":1.25}}}),
        json!({"type":"object","properties":{"x":{"type":"string","enum":["allowed"],"default":"not allowed"}}}),
    ] {
        let p = json!({"threadId":"T","serverName":"native","mode":"form","message":"Unsafe partial form","requestedSchema":schema});
        assert_eq!(
            project_request(
                &state,
                &target,
                &request("mcpServer/elicitation/request", p),
                None
            )
            .unwrap()
            .pending
            .kind,
            PresentationKind::DesktopOnly
        );
    }
    for mode in ["openai/form", "openaiForm", "openai/userVerification"] {
        let p = json!({"threadId":"T","serverName":"native","mode":mode,"message":"Desktop"});
        assert_eq!(
            project_request(
                &state,
                &target,
                &request("mcpServer/elicitation/request", p),
                None
            )
            .unwrap()
            .pending
            .kind,
            PresentationKind::DesktopOnly
        );
    }
    for method in [
        "item/tool/call",
        "attestation/generate",
        "execCommandApproval",
        "applyPatchApproval",
    ] {
        assert_eq!(
            project_request(
                &state,
                &target,
                &request(method, json!({"threadId":"T"})),
                None
            )
            .unwrap()
            .pending
            .kind,
            PresentationKind::DesktopOnly
        );
    }
}
#[test]
fn questions_secret_answers_and_url_navigation_have_no_invented_cancel_or_winner() {
    let (state, target, _) = fixture();
    let p = json!({"threadId":"T","turnId":"turn","itemId":"item","isBlocking":true,"questions":[{"id":"secret","header":"Secret","question":"Enter value","isSecret":true,"options":null},{"id":"choose","header":"Choice","question":"Pick or Other","isOther":true,"options":[{"label":"yes","description":"Offered"}]}]});
    let d = project_request(
        &state,
        &target,
        &request("item/tool/requestUserInput", p),
        None,
    )
    .unwrap();
    let answer = Answer::CodexQuestions(AnswerCodexQuestions {
        answers: vec![
            QuestionAnswer {
                id: Id("secret".into()),
                values: vec!["memory-only-secret".into()],
            },
            QuestionAnswer {
                id: Id("choose".into()),
                values: vec!["different Other value".into()],
            },
        ],
    });
    let response = native_json::parse(&d.response(&answer).unwrap(), NATIVE).unwrap();
    assert_eq!(
        response["result"]["answers"]["secret"]["answers"],
        json!(["memory-only-secret"])
    );
    assert!(
        !serde_json::to_string(&d.pending)
            .unwrap()
            .contains("memory-only-secret")
    );
    assert!(
        d.response(&Answer::CodexUrl(AnswerCodexUrl {
            action: UrlAction::Cancel
        }))
        .is_err()
    );
    let p = json!({"threadId":"T","serverName":"native","mode":"url","message":"Open separately","url":"https://example.test/authorize","elicitationId":"opaque"});
    let d = project_request(
        &state,
        &target,
        &request("mcpServer/elicitation/request", p),
        None,
    )
    .unwrap();
    assert_eq!(d.pending.kind, PresentationKind::CodexUrl);
    let response = native_json::parse(
        &d.response(&Answer::CodexUrl(AnswerCodexUrl {
            action: UrlAction::Cancel,
        }))
        .unwrap(),
        NATIVE,
    )
    .unwrap();
    assert_eq!(response["result"], json!({"action":"cancel"}));
}
#[test]
fn native_history_order_live_precedence_and_source_replacement_reach_reducer() {
    let (state, target, source) = fixture();
    let turn = Id("turn".into());
    let historical = project_item(
        &state,
        &target,
        &turn,
        &json!({"type":"agentMessage","id":"same","text":"old"}),
        8,
        RecordSource::NativePage,
        true,
    )
    .unwrap();
    let live = project_item(
        &state,
        &target,
        &turn,
        &json!({"type":"agentMessage","id":"same","text":"new live"}),
        8,
        RecordSource::Live,
        true,
    )
    .unwrap();
    state
        .history_from_source(&target, &source, vec![historical, live])
        .unwrap();
    let historical = project_item(
        &state,
        &target,
        &turn,
        &json!({"type":"agentMessage","id":"same","text":"stale page"}),
        8,
        RecordSource::NativePage,
        true,
    )
    .unwrap();
    state
        .history_from_source(&target, &source, vec![historical])
        .unwrap();
    let mut origins = goatr::state::HistoryOrigins::default();
    let native = project_item(
        &state,
        &target,
        &turn,
        &json!({"type":"unknownFutureItem","id":"unknown","privateToken":"omitted"}),
        9,
        RecordSource::NativePage,
        true,
    )
    .unwrap();
    assert_eq!(native.record().kind, RecordKind::Unsupported);
    assert!(
        !serde_json::to_string(native.record())
            .unwrap()
            .contains("omitted")
    );
    let current = project_item(
        &state,
        &target,
        &turn,
        &json!({"type":"agentMessage","id":"same","text":"new live"}),
        8,
        RecordSource::Live,
        true,
    )
    .unwrap();
    let old = project_item(
        &state,
        &target,
        &turn,
        &json!({"type":"agentMessage","id":"same","text":"stale page"}),
        8,
        RecordSource::NativePage,
        true,
    )
    .unwrap();
    let merged = goatr::state::merge_history(
        Provider::Codex,
        current.record(),
        old.record().clone(),
        &mut origins,
        true,
    );
    assert_eq!(merged.text, current.record().text);
    let mut display = state.session_state(&target, &source).unwrap();
    display.summary.generation = Uuid::random();
    let replacement = state.begin_install(display).unwrap();
    assert!(
        state
            .update_from_source(&target, &source, SessionUpdate::History(vec![native]))
            .is_err()
    );
    assert_ne!(source, replacement);
}

#[test]
fn native_formats_exact_decimal_ceiling_and_malformed_membership_are_consumer_validated() {
    let (state, target, _) = fixture();
    let properties = json!({
        "email":{"type":"string","format":"email","default":"\"a b\"@example.test"},
        "uri":{"type":"string","format":"uri","default":"urn:example:opaque"},
        "date":{"type":"string","format":"date","default":"2024-02-29"},
        "date-time":{"type":"string","format":"date-time","default":"2024-02-29T23:59:58+01:30"},
        "plain-single":{"type":"string","enum":["first","second"],"default":"second"},
        "plain-multi":{"type":"array","items":{"type":"string","enum":["one","two"]},"minItems":1,"maxItems":2,"default":["one","one"]},
        "titled-multi":{"type":"array","items":{"anyOf":[{"const":"same","title":"First"},{"const":"same","title":"Second"}]},"default":["same","same"]}
    });
    let mut p = json!({"threadId":"T","serverName":"native","mode":"form","message":"Native formats","requestedSchema":{"type":"object","properties":properties}});
    let d = project_request(
        &state,
        &target,
        &request("mcpServer/elicitation/request", p.clone()),
        None,
    )
    .unwrap();
    assert_eq!(d.pending.kind, PresentationKind::CodexForm);
    p["requestedSchema"]["properties"]["date"]["default"] = json!("2023-02-29");
    assert_eq!(
        project_request(
            &state,
            &target,
            &request("mcpServer/elicitation/request", p.clone()),
            None
        )
        .unwrap()
        .pending
        .kind,
        PresentationKind::DesktopOnly
    );
    p["requestedSchema"]["properties"] =
        json!({"bad":{"type":"string","oneOf":["not a const/title object"]}});
    assert_eq!(
        project_request(
            &state,
            &target,
            &request("mcpServer/elicitation/request", p),
            None
        )
        .unwrap()
        .pending
        .kind,
        PresentationKind::DesktopOnly
    );
    for (token, expected) in [
        ("1e127", PresentationKind::CodexForm),
        ("1e128", PresentationKind::DesktopOnly),
    ] {
        let mut params = json!({"threadId":"T","serverName":"native","mode":"form","message":"Bound","requestedSchema":{"type":"object","properties":{"n":{"type":"number"}}}});
        params["requestedSchema"]["properties"]["n"]["maximum"] =
            native_json::parse(token.as_bytes(), NATIVE).unwrap();
        assert_eq!(
            project_request(
                &state,
                &target,
                &request("mcpServer/elicitation/request", params),
                None
            )
            .unwrap()
            .pending
            .kind,
            expected
        );
    }
    let legacy = request(
        "execCommandApproval",
        json!({"conversationId":"T","command":["native"],"cwd":"/work"}),
    );
    assert_eq!(
        project_request(&state, &target, &legacy, None)
            .unwrap()
            .pending
            .kind,
        PresentationKind::DesktopOnly
    );
    let auth = request("account/chatgptAuthTokens/refresh", json!({}));
    assert_eq!(
        project_request(&state, &target, &auth, None)
            .unwrap()
            .pending
            .kind,
        PresentationKind::DesktopOnly
    );
}
