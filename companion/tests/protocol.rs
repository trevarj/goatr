// SPDX-License-Identifier: GPL-3.0-or-later
use base64::{Engine, engine::general_purpose::STANDARD};
use goatr::protocol::*;
use serde_json::{Value, json};
use std::io::{Cursor, Read};

fn corpus() -> Value {
    serde_json::from_str(include_str!("../../protocol/fixtures/v2.json")).unwrap()
}
fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap()
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn method(v: &Value) -> Option<Method> {
    v.get("responseMethod")
        .map(|v| serde_json::from_value(v.clone()).unwrap())
}
fn input(v: &Value) -> Vec<u8> {
    if let Some(s) = v.get("base64") {
        STANDARD.decode(s.as_str().unwrap()).unwrap()
    } else {
        text(v, "wire").as_bytes().to_vec()
    }
}

#[test]
fn shared_envelopes_canonical_hashes_and_roundtrips() {
    let corpus = corpus();
    let key = [11; 32];
    for case in corpus["frames"].as_array().unwrap() {
        let decoded = decode_payload(&input(case), method(case));
        if text(case, "expected") == "accept" {
            let decoded = decoded.unwrap_or_else(|e| panic!("{}: {e}", text(case, "name")));
            if let Envelope::Request(request) = &decoded
                && case.get("canonical").is_some()
            {
                let canonical = canonical_request(request).unwrap();
                assert_eq!(
                    canonical,
                    text(case, "canonical").as_bytes(),
                    "{}",
                    text(case, "name")
                );
                assert_eq!(hex(&sha256(&canonical)), text(case, "sha256"));
                assert_eq!(hex(&hmac_sha256(&key, &canonical)), text(case, "hmac"));
                assert!(verify_hmac_sha256(
                    &key,
                    &canonical,
                    &hmac_sha256(&key, &canonical)
                ));
                assert!(!verify_hmac_sha256(
                    &[12; 32],
                    &canonical,
                    &hmac_sha256(&key, &canonical)
                ));
            }
            if let Envelope::Response(response)=&decoded
                && let Ok(ResultBody::SkillsList(result))=&response.outcome {
                let wire=parse_raw_json(&input(case)).unwrap();
                let rows=wire["result"]["skills"].as_array().unwrap();
                let expected:Vec<_>=rows.iter().map(|row|row.get("invocationRef").and_then(Value::as_str)).collect();
                let actual:Vec<_>=result.skills.iter().map(|row|row.invocation_ref.as_ref().map(|id|id.0.as_str())).collect();
                assert_eq!(actual,expected,"{} must retain opaque native invocation references",text(case,"name"));
            }
            let encoded = encode_payload(&decoded).unwrap();
            let roundtrip = decode_payload(&encoded, method(case)).unwrap();
            assert_eq!(decoded, roundtrip, "{}", text(case, "name"));
        } else {
            let error = decoded.expect_err(text(case, "name"));
            assert_eq!(
                format!("{:?}", error.kind),
                text(case, "expected"),
                "{}",
                text(case, "name")
            );
            assert_eq!(error.close, error.kind != FailureKind::UnsupportedMethod);
            if let Some(expected) = case.get("requestId") {
                assert_eq!(
                    error.request_id.as_ref().map(|id| id.0.as_str()),
                    expected.as_str(),
                    "{}",
                    text(case, "name")
                );
            }
        }
    }
    for case in corpus["canonical"].as_array().unwrap() {
        let canonical = canonical_json(&parse_raw_json(&input(case)).unwrap()).unwrap();
        assert_eq!(canonical, text(case, "canonical").as_bytes());
        assert_eq!(
            canonical_json_size(&parse_raw_json(&input(case)).unwrap()).unwrap(),
            canonical.len()
        );
        assert_eq!(hex(&sha256(&canonical)), text(case, "sha256"));
        assert_eq!(hex(&hmac_sha256(&key, &canonical)), text(case, "hmac"));
    }
    for case in corpus["hmacVectors"].as_array().unwrap() {
        let key: Vec<u8> = text(case, "keyHex")
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|s| u8::from_str_radix(std::str::from_utf8(s).unwrap(), 16).unwrap())
            .collect();
        assert_eq!(
            hex(&hmac_sha256(&key, text(case, "bytes").as_bytes())),
            text(case, "hmac")
        );
    }
}

#[test]
fn generated_finite_envelopes_obey_aggregate_json_bounds() {
    let id = Uuid("ce8ae66d-3345-4810-9052-0363cb356172".into());
    for (count, accepted) in [(JSON_MEMBERS_MAX / 2, true), (JSON_MEMBERS_MAX, false)] {
        let envelope = Envelope::Event(Event {
            subscription: id.clone(),
            seq: Uint(1),
            body: EventBody::RequestOpened(RequestOpenedEvent {
                request: PendingRequest {
                    request_key: id.clone(),
                    provider: Provider::Omp,
                    kind: PresentationKind::OmpSelect,
                    presentation: Presentation::OmpSelect(PresentationOmpSelect {
                        title: Content::Text(ContentText {
                            text: InlineText("Choose".into()),
                        }),
                        options: vec![SelectOption {
                            value: "a".into(),
                            label: Label("A".into()),
                            description: None,
                        }],
                        selected: None,
                        selection_marker: SelectionMarker::Radio,
                        help_text: None,
                        markable_count: None,
                        checked: vec![Uint(0); count],
                    }),
                },
            }),
        });
        let encoded = encode_payload(&envelope);
        assert_eq!(encoded.is_ok(), accepted);
        if let Ok(bytes) = encoded {
            assert_eq!(decode_payload(&bytes, None).unwrap(), envelope);
        }
    }
}

#[test]
fn shared_raw_structural_and_resource_boundaries() {
    let corpus = corpus();
    for case in corpus["raw"].as_array().unwrap() {
        assert_eq!(
            parse_raw_json(&input(case)).is_ok(),
            case["accept"].as_bool().unwrap(),
            "{}",
            text(case, "name")
        );
        if let Some(expected) = case.get("canonical") {
            let parsed = parse_raw_json(&input(case)).unwrap();
            assert!(parsed.is_object(), "{}", text(case, "name"));
            assert_eq!(
                canonical_json(&parsed).unwrap(),
                expected.as_str().unwrap().as_bytes()
            );
        }
    }
    for case in corpus["rawBounds"].as_array().unwrap() {
        let n = case["count"].as_u64().unwrap() as usize;
        let wire = match text(case, "kind") {
            "depth" => format!("{}0{}", "[".repeat(n), "]".repeat(n)),
            "members" => format!("{{\"x\":[{}]}}", vec!["0"; n].join(",")),
            "frame" => format!("{{\"x\":\"{}\"}}", "x".repeat(n - 8)),
            _ => panic!("unknown corpus bound"),
        };
        assert_eq!(
            parse_raw_json(wire.as_bytes()).is_ok(),
            case["accept"].as_bool().unwrap(),
            "{case}"
        );
    }
    for case in corpus["wireBounds"].as_array().unwrap() {
        let mut wire: Value = serde_json::from_str(text(case, "baseWire")).unwrap();
        let mut slot = &mut wire;
        for key in case["path"].as_array().unwrap() {
            slot = if let Some(key) = key.as_str() {
                &mut slot[key]
            } else {
                &mut slot[key.as_u64().unwrap() as usize]
            };
        }
        let n = case["count"].as_u64().unwrap() as usize;
        *slot = if case["generator"] == "text" {
            Value::String(text(case, "unit").repeat(n))
        } else {
            Value::Array(
                (0..n)
                    .map(|index| {
                        let mut unit = case["unit"].clone();
                        if let Some(field) = case["identityField"].as_str() {
                            unit[field] =
                                Value::String(format!("11111111-1111-4111-8111-{index:012x}"));
                        }
                        unit
                    })
                    .collect(),
            )
        };
        assert_eq!(
            decode_payload(&serde_json::to_vec(&wire).unwrap(), method(case)).is_ok(),
            case["accept"].as_bool().unwrap(),
            "{}",
            text(case, "name")
        );
    }
}

#[test]
fn shared_finite_answers_defaults_formats_and_offered_membership() {
    let corpus = corpus();
    for case in corpus["answers"].as_array().unwrap() {
        let accepted = decode_presentation(text(case, "presentation").as_bytes()).and_then(|p| {
            decode_answer(text(case, "answer").as_bytes()).and_then(|a| validate_answer(&p, &a))
        });
        assert_eq!(
            accepted.is_ok(),
            case["accept"].as_bool().unwrap(),
            "{}: {accepted:?}",
            text(case, "name")
        );
    }
    for case in corpus["formats"].as_array().unwrap() {
        let format = serde_json::from_value(case["format"].clone()).unwrap();
        assert_eq!(
            valid_format(format, text(case, "input")),
            case["accept"].as_bool().unwrap(),
            "{case}"
        );
    }
}

#[test]
fn shared_exact_decimals_native_emission_and_lossless_ids() {
    let corpus = corpus();
    for (key, native) in [("decimals", false), ("nativeDecimals", true)] {
        for case in corpus[key].as_array().unwrap() {
            let result = if native {
                Decimal::from_native_token(text(case, "input"))
            } else {
                Decimal::new(text(case, "input"))
            };
            if let Some(expected) = case["normalized"].as_str() {
                let result = result.unwrap();
                assert_eq!(result.0, expected);
                assert_eq!(result.native_number().unwrap().get(), expected);
            } else {
                assert!(result.is_err(), "{case}");
            }
        }
    }
    for case in corpus["comparisons"].as_array().unwrap() {
        let a = Decimal::new(text(case, "a")).unwrap();
        let b = Decimal::new(text(case, "b")).unwrap();
        let ordering = match a.cmp(&b) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        };
        assert_eq!(ordering, case["order"].as_i64().unwrap());
    }
    for raw in corpus["nativeIds"].as_array().unwrap() {
        let raw = raw.as_str().unwrap();
        let id = NativeRpcId::from_raw(raw).unwrap();
        assert_eq!(serde_json::to_string(&id).unwrap(), raw);
    }
    for raw in ["true", "1.0", "1e0", "{}", "null"] {
        assert!(NativeRpcId::from_raw(raw).is_err());
    }
}

struct Fragmented<R>(R);
impl<R: Read> Read for Fragmented<R> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let n = out.len().min(1);
        self.0.read(&mut out[..n])
    }
}
#[test]
fn real_framing_handles_partial_reads_and_rejects_lengths_before_payload() {
    let bytes = serde_json::to_vec(&json!({"unicode":"😀 é"})).unwrap();
    let mut encoded = Vec::new();
    write_frame(&mut encoded, &bytes).unwrap();
    assert_eq!(
        read_frame(&mut Fragmented(Cursor::new(&encoded))).unwrap(),
        bytes
    );
    assert!(read_frame(&mut Cursor::new(&encoded[..encoded.len() - 1])).is_err());
    for length in [0u32, FRAME_MAX as u32 + 1, u32::MAX] {
        assert!(read_frame(&mut Cursor::new(length.to_be_bytes())).is_err());
    }
    assert_eq!(
        frame_length((FRAME_MAX as u32).to_be_bytes()).unwrap(),
        FRAME_MAX
    );
}

#[test]
fn shared_generation_bound_cancel_and_content_constraints() {
    let corpus = corpus();
    let id = Uuid::new("11111111-1111-4111-8111-111111111111").unwrap();
    let target = Target {
        session_id: id.clone(),
        generation: id.clone(),
    };
    for case in corpus["contextChecks"]["cancel"].as_array().unwrap() {
        let params = TurnCancelParams {
            boot_id: id.clone(),
            target: target.clone(),
            native_turn_id: case["nativeTurnId"].as_str().map(|s| Id::new(s).unwrap()),
        };
        let provider = serde_json::from_value(case["provider"].clone()).unwrap();
        assert_eq!(
            validate_turn_cancel(provider, &params).is_ok(),
            case["accept"].as_bool().unwrap()
        );
    }
    for case in corpus["contextChecks"]["contentRead"].as_array().unwrap() {
        let params = ContentReadParams {
            target: target.clone(),
            handle: id.clone(),
            offset_bytes: Uint(case["offset"].as_u64().unwrap()),
            limit_bytes: Uint(1),
        };
        assert_eq!(
            validate_content_read(&params, Uint(case["length"].as_u64().unwrap())).is_ok(),
            case["accept"].as_bool().unwrap()
        );
    }
    assert_eq!(
        uuid::Uuid::parse_str(&Uuid::random().0)
            .unwrap()
            .get_version(),
        Some(uuid::Version::Random)
    );
}

#[test]
fn canonical_expansion_does_not_shrink_valid_decoded_editor_or_prompt_limit() {
    let id = Uuid::random();
    let target = Target {
        session_id: Uuid::random(),
        generation: Uuid::random(),
    };
    for body in [
        RequestBody::PromptSubmit(PromptSubmitParams { delivery: None, native_turn_id: None, images: None, skill: None, boot_id: id.clone(),
        target: target.clone(),
        text: PromptText::new("\n".repeat(ANSWER_MAX)).unwrap(), }),
        RequestBody::RequestRespond(RequestRespondParams {
            boot_id: id.clone(),
            target,
            request_key: Uuid::random(),
            answer: Answer::OmpEditor(AnswerOmpEditor {
                action: SubmitAction::Submit,
                text: Some(PromptText::new("\n".repeat(ANSWER_MAX)).unwrap()),
            }),
        }),
    ] {
        let request = Request {
            id: id.clone(),
            body,
        };
        let envelope = Envelope::Request(request.clone());
        let wire = encode_payload(&envelope).unwrap();
        assert!(wire.len() <= FRAME_MAX);
        assert_eq!(decode_payload(&wire, None).unwrap(), envelope);
        assert!(canonical_request(&request).unwrap().len() > FRAME_MAX);
    }
}

#[test]
fn observer_hello_defaults_and_subscription_builders_preserve_old_bytes() {
    let corpus=corpus();
    let old=corpus["frames"].as_array().unwrap().iter().find(|case|case["name"]=="result hello").unwrap();
    let Envelope::Response(response)=decode_payload(&input(old),Some(Method::Hello)).unwrap() else{panic!()};
    let Ok(ResultBody::Hello(hello))=response.outcome else{panic!()};
    assert!(!hello.features.agent_ansi);
    let id=Uuid::random();let target=Target{session_id:id.clone(),generation:id.clone()};
    let request=Request{id:id.clone(),body:RequestBody::TerminalGridSubscribe(TerminalGridSubscribeParams{target:target.clone(),ansi_observer:false})};
    let wire=encode_payload(&Envelope::Request(request.clone())).unwrap();
    let raw=parse_raw_json(&wire).unwrap();
    assert_eq!(raw["params"],json!({"target":target}));
    assert_eq!(canonical_request(&request).unwrap(),canonical_json(&json!({"method":"terminal.grid.subscribe","params":{"target":target}})).unwrap());
    let old_wire=json!({"v":2,"type":"request","id":id,"method":"terminal.grid.subscribe","params":{"target":target}});
    assert_eq!(decode_payload(&serde_json::to_vec(&old_wire).unwrap(),None).unwrap(),Envelope::Request(request.clone()));
    let mut explicit=old_wire;explicit["params"]["ansiObserver"]=Value::Bool(false);
    assert_eq!(decode_payload(&serde_json::to_vec(&explicit).unwrap(),None).unwrap(),Envelope::Request(request));
    explicit["params"]["ansiObserver"]=Value::Bool(true);
    let Envelope::Request(negotiated)=decode_payload(&serde_json::to_vec(&explicit).unwrap(),None).unwrap() else{panic!()};
    let RequestBody::TerminalGridSubscribe(params)=&negotiated.body else{panic!()};
    assert!(params.ansi_observer);
    assert_eq!(parse_raw_json(&encode_payload(&Envelope::Request(negotiated)).unwrap()).unwrap()["params"]["ansiObserver"],true);
}

#[test]
fn shell_history_negotiation_preserves_old_bytes_and_success_only_actual_metrics() {
    let corpus=corpus();let frames=corpus["frames"].as_array().unwrap();
    let old=frames.iter().find(|case|case["name"]=="result hello").unwrap();
    let Envelope::Response(response)=decode_payload(&input(old),Some(Method::Hello)).unwrap() else{panic!()};
    let Ok(ResultBody::Hello(hello))=response.outcome else{panic!()};assert!(!hello.features.shell_history);
    let old=frames.iter().find(|case|case["name"]=="request shell.grid.subscribe").unwrap();
    let Envelope::Request(old_request)=decode_payload(&input(old),None).unwrap() else{panic!()};
    let old_bytes=encode_payload(&Envelope::Request(old_request.clone())).unwrap();
    assert!(parse_raw_json(&old_bytes).unwrap()["params"].get("history").is_none());
    for enabled in [false,true] {
        let mut request=old_request.clone();let RequestBody::ShellGridSubscribe(params)=&mut request.body else{panic!()};params.history=enabled;
        let wire=encode_payload(&Envelope::Request(request.clone())).unwrap();
        if !enabled {assert_eq!(wire,old_bytes);assert_eq!(canonical_request(&request).unwrap(),old["canonical"].as_str().unwrap().as_bytes());}
        else {assert_eq!(parse_raw_json(&wire).unwrap()["params"]["history"],true);}
    }
    let case=frames.iter().find(|case|case["name"]=="shell.scroll succeeded reports actual clamped metrics").unwrap();
    let mut wire:Value=serde_json::from_slice(&input(case)).unwrap();
    for state in ["reserved","dispatching","dispatched","failed","uncertain"] {
        wire["result"]["receipt"]["state"]=json!(state);
        assert!(decode_payload(&serde_json::to_vec(&wire).unwrap(),Some(Method::ShellScroll)).is_err());
    }
}
