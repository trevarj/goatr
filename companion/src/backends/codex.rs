// SPDX-License-Identifier: GPL-3.0-or-later
//! Stock Codex shared-owner adapter. Native contracts are independently implemented.
use crate::{
    codex_relay::{self, Manifest, OwnerEpoch, Presence},
    config::{self, CodexOwner},
    herdr::{self, Agent, ProcessEvidence, Source, Topology},
    native_json::{self, NATIVE},
    pairing::{self, Authorized, Security, SecurityResult},
    protocol::*,
    state::{ProviderSnapshot, SafeHistory, SessionUpdate, State},
    storage::{
        AuthenticatedPeer, CreationOutcome, ReceiptTransition, Reservation, StableSessionKey,
        StageMetadata, TransitionProof,
    },
};
use futures_util::{SinkExt, StreamExt};
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::{
    collections::{HashMap, HashSet},
    io,
    os::fd::{AsFd, OwnedFd},
    path::Path,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    net::UnixStream,
    sync::{oneshot, watch},
    task::JoinSet,
};
use tokio_tungstenite::{WebSocketStream, tungstenite::Message};

pub const SOURCE_REVISION: &str = "092d3acd6bec3e3a14bdc7e7a2810ab628ab759d";
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Codex native identity or contract unavailable",
    )
}
fn policy(_: impl std::fmt::Display) -> ErrorCode {
    ErrorCode::NotReady
}
/// The reservation callback has already registered the original job when this runs.
/// Creation returns its durable receipt instead of waiting for native completion.
fn reservation_reply(create: bool, reservation: Reservation) -> Option<Receipt> {
    match reservation {
        Reservation::Existing(receipt) => Some(receipt),
        Reservation::Reserved(receipt) if create => Some(receipt),
        Reservation::Reserved(_) => None,
    }
}
fn presence_failure_detail(error: &io::Error) -> (&'static str, io::ErrorKind, Option<i32>) {
    // Exact authored guards and fixed config-template prefixes only. Path suffixes and
    // unrecognized external messages never enter the finite diagnostic categories.
    let category = match error.to_string().as_str() {
        "private parent absent" => "private_parent",
        "oversized or aliased private Codex file" => "private_size",
        "invalid private Codex record" => "private_record",
        "private Codex publication did not stabilize" => "publication_unstable",
        "invalid or oversized native JSON" => "native_json",
        "unsafe private path" => "unsafe_private_path",
        "path is not UTF-8" => "path_encoding",
        "stream did not contain valid UTF-8" => "io_utf8",
        "missing socket parent"
        | "host socket parent absent"
        | "agent socket parent absent"
        | "physical socket parent absent"
        | "listen parent absent" => "socket_parent",
        "installed object changed during trust proof" => "installed_object",
        "unknown Codex binding version"
        | "invalid binding UUID"
        | "invalid owner UUID"
        | "invalid thread ID"
        | "invalid server ID"
        | "invalid incarnation"
        | "invalid Herdr binding ID" => "binding_schema",
        "relay host/provider parent object proof unavailable" => "binding_parent",
        "invalid boot identity" => "owner_boot",
        "Codex owner process/source replaced" => "owner_identity",
        "noncanonical native process number" | "native process number overflow" => "process_number",
        "No verified Codex publisher distribution for this platform" => "distribution_platform",
        "Codex process changed before executable proof" => "distribution_process",
        "Kernel-opened Codex executable/install/process-root objects disagree" => {
            "distribution_object"
        }
        "Codex native executable is not ELF" => "distribution_elf",
        "Codex executable differs from verified source-linked publisher distribution" => {
            "distribution_digest"
        }
        "Codex executable/process changed during digest proof" => "distribution_changed",
        "invalid native socket pathname" | "invalid provider pathname" => "native_pathname",
        "owner process changed before root capture"
        | "owner process changed during root capture"
        | "owner process root changed" => "owner_root",
        "unsafe native socket directory" => "socket_directory",
        "unsafe native rendezvous alias" => "socket_alias",
        "native rendezvous target is not the exact stock hash" => "socket_alias_target",
        "unsafe native physical socket" | "unsafe Codex socket" => "socket_type",
        "advertised host/provider alias objects disagree"
        | "explicit host physical socket mapping disagrees" => "socket_mapping",
        "native physical socket epoch replaced" => "socket_epoch",
        "native alias/root/parent source epoch replaced"
        | "owner alias/root/socket source changed" => "socket_source",
        "PID namespace translation unavailable"
        | "PID namespace nesting limit"
        | "invalid namespace PID"
        | "visible process PID disagrees with proc namespace view" => "process_namespace",
        "invalid process stat" | "invalid process comm" | "process start ticks unavailable" => {
            "process_stat"
        }
        "actual caller PID unavailable"
        | "peer PID unavailable in namespace"
        | "process PID mismatch" => "process_pid",
        "process UID mismatch" => "process_uid",
        "Linux boot identity unavailable" => "process_boot",
        "process changed or procfs PID view unavailable during evidence capture" => {
            "process_changed"
        }
        "Codex relay presence identity changed" => "presence_identity",
        "invalid relay connection epoch" => "connection_epoch",
        "Codex relay presence older than five seconds" => "presence_past_5s",
        "Codex relay presence more than one second ahead" => "presence_future_1s",
        "relay association limit" => "association_limit",
        "invalid associated thread" => "associated_thread",
        "duplicate associated thread" => "associated_duplicate",
        "TUI process proof absent" => "tui_absent",
        "TUI process replaced" => "tui_replaced",
        "TUI relay socket object differs" => "tui_socket_object",
        "disconnected relay advertises associations" => "disconnected_association",
        message if message.starts_with("absolute control-free path required: ") => "path_policy",
        message if message.starts_with("Unix socket path exceeds 107 UTF-8 bytes: ") => {
            "socket_path"
        }
        message
            if message.starts_with("unsafe installed executable: ")
                || message.starts_with("unsafe executable parent: ") =>
        {
            "installed_object"
        }
        _ => "unknown",
    };
    (category, error.kind(), error.raw_os_error())
}
fn launch_failure_proof(
    gate: &'static str,
    code: ErrorCode,
    presence: Option<(&'static str, io::ErrorKind, Option<i32>)>,
) -> String {
    match presence {
        Some((category, kind, errno)) => format!(
            "Native launch proof unavailable; gate={gate}; error={code:?}; presence={category}; io_kind={kind:?}; errno={errno:?}",
        ),
        None => format!("Native launch proof unavailable; gate={gate}; error={code:?}"),
    }
}
fn string(v: &Value, key: &str) -> SecurityResult<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or(ErrorCode::InvalidRequest)
}
fn native_id(v: &Value, key: &str) -> SecurityResult<Id> {
    let id = Id(string(v, key)?);
    id.validate().map_err(|_| ErrorCode::InvalidRequest)?;
    Ok(id)
}
fn paginated_zero_turn_thread(thread: &Value, expected: &Id) -> Result<(), &'static str> {
    if thread["id"].as_str() != Some(expected.0.as_str()) {
        return Err("Native thread response does not match the original created thread");
    }
    match thread["historyMode"].as_str() {
        Some("paginated") => {}
        Some("legacy") => {
            return Err(
                "Native created thread uses legacy history; no materializing read fallback",
            );
        }
        _ => {
            return Err(
                "Native created thread historyMode is missing or unsupported; no materializing read fallback",
            );
        }
    }
    if !thread["turns"].as_array().is_some_and(Vec::is_empty) {
        return Err("Native created thread does not prove zero turns");
    }
    Ok(())
}
fn native_name_set_admission(result: &Value) -> SecurityResult<()> {
    if result.as_object().is_some_and(serde_json::Map::is_empty) {
        Ok(())
    } else {
        Err(ErrorCode::NotReady)
    }
}
fn boolean(v: &Value, key: &str, default: Option<bool>) -> SecurityResult<bool> {
    match v.get(key) {
        Some(Value::Bool(b)) => Ok(*b),
        None => default.ok_or(ErrorCode::InvalidRequest),
        _ => Err(ErrorCode::InvalidRequest),
    }
}
fn closed(v: &Value, keys: &[&str]) -> SecurityResult<()> {
    if v.as_object()
        .is_some_and(|o| o.keys().all(|k| keys.contains(&k.as_str())))
    {
        Ok(())
    } else {
        Err(ErrorCode::InvalidRequest)
    }
}
fn content(state: &State, target: &Target, mime: Mime, text: String) -> SecurityResult<Content> {
    state.content(target, mime, text, pairing::now_ms())
}
fn card_content(
    state: &State,
    target: &Target,
    mime: Mime,
    text: String,
) -> SecurityResult<Content> {
    let c = content(state, target, mime, text)?;
    if matches!(c, Content::Unavailable(_)) {
        return Err(ErrorCode::ResourceLimit);
    }
    Ok(c)
}
fn optional_strings(v: &Value, keys: &[&str]) -> SecurityResult<()> {
    for key in keys {
        if v.get(*key).is_some_and(|v| !v.is_null() && !v.is_string()) {
            return Err(ErrorCode::InvalidRequest);
        }
    }
    Ok(())
}
fn command_details(p: &Value) -> SecurityResult<()> {
    closed(
        p,
        &[
            "threadId",
            "turnId",
            "itemId",
            "startedAtMs",
            "kind",
            "approvalId",
            "environmentId",
            "reason",
            "networkApprovalContext",
            "command",
            "cwd",
            "commandActions",
            "additionalPermissions",
            "availableDecisions",
            "proposedExecpolicyAmendment",
            "proposedNetworkPolicyAmendments",
        ],
    )?;
    optional_strings(
        p,
        &["approvalId", "environmentId", "reason", "command", "cwd"],
    )?;
    if let Some(n) = p.get("networkApprovalContext").filter(|v| !v.is_null()) {
        closed(n, &["host", "protocol"])?;
        string(n, "host")?;
        if !matches!(
            n["protocol"].as_str(),
            Some("http" | "https" | "socks5Tcp" | "socks5Udp")
        ) {
            return Err(ErrorCode::InvalidRequest);
        }
    }
    if let Some(actions) = p.get("commandActions").filter(|v| !v.is_null()) {
        for a in actions.as_array().ok_or(ErrorCode::InvalidRequest)? {
            string(a, "command")?;
            match a["type"].as_str() {
                Some("read") => {
                    closed(a, &["type", "command", "name", "path"])?;
                    string(a, "name")?;
                    string(a, "path")?;
                }
                Some("listFiles") => {
                    closed(a, &["type", "command", "path"])?;
                    optional_strings(a, &["path"])?;
                }
                Some("search") => {
                    closed(a, &["type", "command", "path", "query"])?;
                    optional_strings(a, &["path", "query"])?;
                }
                Some("unknown") => closed(a, &["type", "command"])?,
                _ => return Err(ErrorCode::InvalidRequest),
            }
        }
    }
    Ok(())
}
fn text_content(state: &State, target: &Target, text: String) -> SecurityResult<Content> {
    content(state, target, Mime::TextPlain, text)
}
fn optional_uint(v: &Value, key: &str) -> SecurityResult<Option<Uint>> {
    v.get(key)
        .map(|v| {
            let n = native_json::decimal(v).map_err(policy)?;
            if !n.is_integer() || n.0.starts_with('-') {
                return Err(ErrorCode::InvalidRequest);
            }
            Uint::new(n.0.parse().map_err(|_| ErrorCode::InvalidRequest)?)
                .map_err(|_| ErrorCode::InvalidRequest)
        })
        .transpose()
}
fn permission_profile(v: &Value) -> SecurityResult<()> {
    closed(v, &["network", "fileSystem"])?;
    if let Some(n) = v.get("network").filter(|v| !v.is_null()) {
        closed(n, &["enabled"])?;
        if let Some(b) = n.get("enabled").filter(|v| !v.is_null())
            && !b.is_boolean()
        {
            return Err(ErrorCode::InvalidRequest);
        }
    }
    if let Some(f) = v.get("fileSystem").filter(|v| !v.is_null()) {
        closed(f, &["read", "write", "entries", "globScanMaxDepth"])?;
        for key in ["read", "write"] {
            if let Some(a) = f.get(key).filter(|v| !v.is_null()) {
                let a = a.as_array().ok_or(ErrorCode::InvalidRequest)?;
                if a.iter().any(|p| !p.is_string()) {
                    return Err(ErrorCode::InvalidRequest);
                }
            }
        }
        if let Some(n) = f.get("globScanMaxDepth").filter(|v| !v.is_null()) {
            let d = native_json::decimal(n).map_err(policy)?;
            if !d.is_integer() || d <= Decimal::new("0").map_err(policy)? {
                return Err(ErrorCode::InvalidRequest);
            }
        }
        if let Some(entries) = f.get("entries").filter(|v| !v.is_null()) {
            for e in entries.as_array().ok_or(ErrorCode::InvalidRequest)? {
                closed(e, &["access", "path"])?;
                if !matches!(e["access"].as_str(), Some("read" | "write" | "deny")) {
                    return Err(ErrorCode::InvalidRequest);
                }
                let p = &e["path"];
                match p["type"].as_str() {
                    Some("path") => {
                        closed(p, &["type", "path"])?;
                        string(p, "path")?;
                    }
                    Some("glob_pattern") => {
                        closed(p, &["type", "pattern"])?;
                        string(p, "pattern")?;
                    }
                    Some("special") => {
                        closed(p, &["type", "value"])?;
                        let s = &p["value"];
                        match s["kind"].as_str() {
                            Some("root" | "minimal" | "tmpdir" | "slash_tmp") => {
                                closed(s, &["kind"])?
                            }
                            Some("project_roots") => {
                                closed(s, &["kind", "subpath"])?;
                                if let Some(x) = s.get("subpath").filter(|v| !v.is_null())
                                    && !x.is_string()
                                {
                                    return Err(ErrorCode::InvalidRequest);
                                }
                            }
                            Some("unknown") => {
                                closed(s, &["kind", "path", "subpath"])?;
                                string(s, "path")?;
                                if let Some(x) = s.get("subpath").filter(|v| !v.is_null())
                                    && !x.is_string()
                                {
                                    return Err(ErrorCode::InvalidRequest);
                                }
                            }
                            _ => return Err(ErrorCode::InvalidRequest),
                        }
                    }
                    _ => return Err(ErrorCode::InvalidRequest),
                }
            }
        }
    }
    Ok(())
}
fn decision(v: &Value) -> SecurityResult<(&'static str, ApprovalScope)> {
    match v.as_str() {
        Some("accept") => Ok(("Allow once", ApprovalScope::Once)),
        Some("acceptForSession") => Ok(("Allow for this session", ApprovalScope::Session)),
        Some("decline") => Ok(("Decline; continue turn", ApprovalScope::Once)),
        Some("cancel") => Ok(("Cancel; interrupt turn", ApprovalScope::Once)),
        Some(_) => Err(ErrorCode::InvalidRequest),
        None => {
            if let Some(a) = v.get("acceptWithExecpolicyAmendment") {
                closed(v, &["acceptWithExecpolicyAmendment"])?;
                closed(a, &["execpolicy_amendment"])?;
                if a["execpolicy_amendment"]
                    .as_array()
                    .is_none_or(|a| a.iter().any(|v| !v.is_string()))
                {
                    return Err(ErrorCode::InvalidRequest);
                };
                Ok(("Allow and save execution policy", ApprovalScope::Policy))
            } else if let Some(a) = v.get("applyNetworkPolicyAmendment") {
                closed(v, &["applyNetworkPolicyAmendment"])?;
                closed(a, &["network_policy_amendment"])?;
                let n = &a["network_policy_amendment"];
                closed(n, &["host", "action"])?;
                string(n, "host")?;
                match n["action"].as_str() {
                    Some("allow") => Ok(("Allow and save network policy", ApprovalScope::Policy)),
                    Some("deny") => Ok(("Deny and save network policy", ApprovalScope::Policy)),
                    _ => Err(ErrorCode::InvalidRequest),
                }
            } else {
                Err(ErrorCode::InvalidRequest)
            }
        }
    }
}
fn command_decisions(p: &Value) -> SecurityResult<Vec<Value>> {
    if let Some(list) = p.get("availableDecisions") {
        return list.as_array().cloned().ok_or(ErrorCode::InvalidRequest);
    }
    let mut values = vec![Value::String("accept".into())];
    if p.get("networkApprovalContext")
        .is_some_and(|v| !v.is_null())
    {
        values.push(Value::String("acceptForSession".into()));
        if let Some(proposals) = p
            .get("proposedNetworkPolicyAmendments")
            .filter(|v| !v.is_null())
        {
            let proposals = proposals.as_array().ok_or(ErrorCode::InvalidRequest)?;
            for n in proposals {
                closed(n, &["host", "action"])?;
                string(n, "host")?;
                if !matches!(n["action"].as_str(), Some("allow" | "deny")) {
                    return Err(ErrorCode::InvalidRequest);
                }
            }
            if let Some(n) = proposals.iter().find(|n| n["action"] == "allow") {
                let mut inner = Map::new();
                inner.insert("network_policy_amendment".into(), n.clone());
                let mut outer = Map::new();
                outer.insert("applyNetworkPolicyAmendment".into(), Value::Object(inner));
                values.push(Value::Object(outer));
            }
        }
    } else if !p.get("additionalPermissions").is_some_and(|v| !v.is_null())
        && let Some(a) = p
            .get("proposedExecpolicyAmendment")
            .filter(|v| !v.is_null())
    {
        let mut inner = Map::new();
        inner.insert("execpolicy_amendment".into(), a.clone());
        let mut outer = Map::new();
        outer.insert("acceptWithExecpolicyAmendment".into(), Value::Object(inner));
        values.push(Value::Object(outer));
    }
    values.push(Value::String("cancel".into()));
    Ok(values)
}
fn enum_options(schema: &Value, multi: bool) -> SecurityResult<Vec<FormOption>> {
    let list = schema
        .get("enum")
        .or_else(|| schema.get("oneOf"))
        .or_else(|| schema.get("anyOf"))
        .and_then(Value::as_array)
        .ok_or(ErrorCode::InvalidRequest)?;
    let names = schema
        .get("enumNames")
        .map(|v| v.as_array().ok_or(ErrorCode::InvalidRequest))
        .transpose()?;
    if names.is_some_and(|n| n.len() != list.len()) {
        return Err(ErrorCode::InvalidRequest);
    }
    if multi {
        if schema.get("enum").is_some() {
            closed(schema, &["type", "enum"])?;
            if schema["type"] != "string" {
                return Err(ErrorCode::InvalidRequest);
            }
        } else {
            closed(schema, &["anyOf", "oneOf"])?;
            if schema.get("anyOf").is_some() && schema.get("oneOf").is_some() {
                return Err(ErrorCode::InvalidRequest);
            }
        }
    }
    list.iter()
        .enumerate()
        .map(|(i, v)| {
            let (value, label) = if schema.get("enum").is_some() {
                let s = v.as_str().ok_or(ErrorCode::InvalidRequest)?;
                (
                    s.to_owned(),
                    match names {
                        Some(n) => n[i].as_str().ok_or(ErrorCode::InvalidRequest)?.to_owned(),
                        None => s.to_owned(),
                    },
                )
            } else {
                closed(v, &["const", "title"])?;
                (string(v, "const")?, string(v, "title")?)
            };
            Ok(FormOption {
                value,
                label: Label(label),
            })
        })
        .collect()
}
fn form_fields(state: &State, target: &Target, schema: &Value) -> SecurityResult<Vec<FormField>> {
    closed(schema, &["$schema", "type", "properties", "required"])?;
    if schema["type"] != "object" || schema.get("$schema").is_some_and(|v| !v.is_string()) {
        return Err(ErrorCode::InvalidRequest);
    }
    let properties = schema["properties"]
        .as_object()
        .ok_or(ErrorCode::InvalidRequest)?;
    let required = schema
        .get("required")
        .map(|v| v.as_array().ok_or(ErrorCode::InvalidRequest))
        .transpose()?;
    let mut required_ids = HashSet::new();
    if let Some(list) = required {
        for r in list {
            let r = r.as_str().ok_or(ErrorCode::InvalidRequest)?;
            if !properties.contains_key(r) || !required_ids.insert(r) {
                return Err(ErrorCode::InvalidRequest);
            }
        }
    }
    properties
        .iter()
        .map(|(id, s)| {
            let typ = s["type"].as_str().ok_or(ErrorCode::InvalidRequest)?;
            let enum_type = s.get("enum").is_some() || s.get("oneOf").is_some();
            let keys = match (typ, enum_type) {
                ("string", true) => vec![
                    "type",
                    "title",
                    "description",
                    "enum",
                    "enumNames",
                    "oneOf",
                    "default",
                ],
                ("string", false) => vec![
                    "type",
                    "title",
                    "description",
                    "minLength",
                    "maxLength",
                    "format",
                    "default",
                ],
                ("number" | "integer", _) => vec![
                    "type",
                    "title",
                    "description",
                    "minimum",
                    "maximum",
                    "default",
                ],
                ("boolean", _) => vec!["type", "title", "description", "default"],
                ("array", _) => vec![
                    "type",
                    "title",
                    "description",
                    "minItems",
                    "maxItems",
                    "items",
                    "default",
                ],
                _ => return Err(ErrorCode::InvalidRequest),
            };
            closed(s, &keys)?;
            if s.get("enum").is_some() && s.get("oneOf").is_some() {
                return Err(ErrorCode::InvalidRequest);
            }
            let field_type = match (typ, enum_type) {
                ("string", true) => FieldType::Single,
                ("string", false) => FieldType::String,
                ("number", _) => FieldType::Number,
                ("integer", _) => FieldType::Integer,
                ("boolean", _) => FieldType::Boolean,
                ("array", _) => FieldType::Multi,
                _ => return Err(ErrorCode::InvalidRequest),
            };
            let default = s
                .get("default")
                .map(|v| match field_type {
                    FieldType::Number | FieldType::Integer => native_json::decimal(v)
                        .map(|d| FieldValue::String(d.0))
                        .map_err(policy),
                    FieldType::Boolean => v
                        .as_bool()
                        .map(FieldValue::Boolean)
                        .ok_or(ErrorCode::InvalidRequest),
                    FieldType::Multi => v
                        .as_array()
                        .ok_or(ErrorCode::InvalidRequest)?
                        .iter()
                        .map(|v| {
                            v.as_str()
                                .map(str::to_owned)
                                .ok_or(ErrorCode::InvalidRequest)
                        })
                        .collect::<SecurityResult<Vec<_>>>()
                        .map(FieldValue::Multi),
                    _ => v
                        .as_str()
                        .map(|s| FieldValue::String(s.to_owned()))
                        .ok_or(ErrorCode::InvalidRequest),
                })
                .transpose()?;
            let format = s
                .get("format")
                .map(|v| match v.as_str() {
                    Some("email") => Ok(FieldFormat::Email),
                    Some("uri") => Ok(FieldFormat::Uri),
                    Some("date") => Ok(FieldFormat::Date),
                    Some("date-time") => Ok(FieldFormat::DateTime),
                    _ => Err(ErrorCode::InvalidRequest),
                })
                .transpose()?;
            let f = FormField {
                id: Id(id.clone()),
                title: s
                    .get("title")
                    .map(|v| v.as_str().ok_or(ErrorCode::InvalidRequest))
                    .transpose()?
                    .unwrap_or(id)
                    .to_owned(),
                description: s
                    .get("description")
                    .map(|v| {
                        v.as_str()
                            .ok_or(ErrorCode::InvalidRequest)
                            .and_then(|s| text_content(state, target, s.to_owned()))
                    })
                    .transpose()?,
                required: required_ids.contains(id.as_str()),
                r#type: field_type,
                default,
                format,
                min_length: optional_uint(s, "minLength")?,
                max_length: optional_uint(s, "maxLength")?,
                minimum: s
                    .get("minimum")
                    .map(native_json::decimal)
                    .transpose()
                    .map_err(policy)?,
                maximum: s
                    .get("maximum")
                    .map(native_json::decimal)
                    .transpose()
                    .map_err(policy)?,
                min_items: optional_uint(s, "minItems")?,
                max_items: optional_uint(s, "maxItems")?,
                options: match field_type {
                    FieldType::Single => Some(enum_options(s, false)?),
                    FieldType::Multi => Some(enum_options(&s["items"], true)?),
                    _ => None,
                },
            };
            f.validate().map_err(|_| ErrorCode::InvalidRequest)?;
            Ok(f)
        })
        .collect()
}

pub struct Dialog {
    pub pending: PendingRequest,
    id: NativeRpcId,
    choices: Vec<Value>,
    thread: Id,
    sent: bool,
    fingerprint: [u8; 32],
}
impl Dialog {
    pub fn response(&self, answer: &Answer) -> SecurityResult<Vec<u8>> {
        validate_answer(&self.pending.presentation, answer)
            .map_err(|_| ErrorCode::InvalidAnswer)?;
        let result = match answer {
            Answer::CodexApproval(a) => {
                let i = a
                    .choice_id
                    .0
                    .strip_prefix("choice-")
                    .and_then(|s| s.parse::<usize>().ok())
                    .ok_or(ErrorCode::InvalidAnswer)?;
                self.choices
                    .get(i)
                    .cloned()
                    .ok_or(ErrorCode::InvalidAnswer)?
            }
            Answer::CodexQuestions(a) => {
                let mut answers = Map::new();
                for q in &a.answers {
                    answers.insert(q.id.0.clone(), json!({"answers":&q.values}));
                }
                let mut result = Map::new();
                result.insert("answers".into(), Value::Object(answers));
                Value::Object(result)
            }
            Answer::CodexForm(a) => {
                let action = match a.action {
                    FormAction::Accept => "accept",
                    FormAction::Decline => "decline",
                    FormAction::Cancel => "cancel",
                };
                let mut result = Map::new();
                result.insert("action".into(), Value::String(action.into()));
                if a.action == FormAction::Accept {
                    let Presentation::CodexForm(f) = &self.pending.presentation else {
                        return Err(ErrorCode::InvalidAnswer);
                    };
                    let mut values = Map::new();
                    for answer in a.values.as_ref().ok_or(ErrorCode::InvalidAnswer)? {
                        let field = f
                            .fields
                            .iter()
                            .find(|f| f.id == answer.id)
                            .ok_or(ErrorCode::InvalidAnswer)?;
                        let value = match &answer.value {
                            FieldValue::String(s)
                                if matches!(
                                    field.r#type,
                                    FieldType::Number | FieldType::Integer
                                ) =>
                            {
                                Value::Number(serde_json::Number::from_string_unchecked(
                                    Decimal::new(s)
                                        .map_err(policy)?
                                        .native_number()
                                        .map_err(policy)?
                                        .get()
                                        .to_owned(),
                                ))
                            }
                            FieldValue::String(s) => Value::String(s.clone()),
                            FieldValue::Boolean(b) => Value::Bool(*b),
                            FieldValue::Multi(v) => {
                                Value::Array(v.iter().cloned().map(Value::String).collect())
                            }
                        };
                        values.insert(answer.id.0.clone(), value);
                    }
                    result.insert("content".into(), Value::Object(values));
                }
                Value::Object(result)
            }
            Answer::CodexUrl(a) => {
                json!({"action":match a.action{UrlAction::Accept=>"accept",UrlAction::Decline=>"decline",UrlAction::Cancel=>"cancel"}})
            }
            _ => return Err(ErrorCode::InvalidAnswer),
        };
        #[derive(Serialize)]
        struct Response<'a> {
            id: &'a NativeRpcId,
            result: &'a Value,
        }
        serde_json::to_vec(&Response {
            id: &self.id,
            result: &result,
        })
        .map_err(policy)
    }
}
/// Unsupported detail invalidates the entire card, never an actionable filtered subset.
pub fn project_request(
    state: &State,
    target: &Target,
    frame: &Value,
    file_item: Option<&Value>,
) -> SecurityResult<Dialog> {
    let id = native_json::rpc_id(&frame["id"]).map_err(policy)?;
    let p = &frame["params"];
    let method = string(frame, "method")?;
    let exact_thread = native_id(p, "threadId").ok();
    let supported_scope = exact_thread.is_some();
    let thread = match exact_thread.or_else(|| native_id(p, "conversationId").ok()) {
        Some(id) => id,
        None => {
            state
                .session_state(target, &state.source_epoch(target)?)?
                .summary
                .native_id
        }
    };
    let mut choices = Vec::new();
    let projected = (|| -> SecurityResult<Presentation> {
        if !supported_scope {
            return Err(ErrorCode::InvalidRequest);
        }
        match method.as_str() {
            "item/commandExecution/requestApproval"
            | "item/fileChange/requestApproval"
            | "item/permissions/requestApproval" => {
                native_id(p, "turnId")?;
                native_id(p, "itemId")?;
                if p["startedAtMs"]
                    .as_number()
                    .is_none_or(|n| n.to_string().parse::<i64>().is_err())
                {
                    return Err(ErrorCode::InvalidRequest);
                }
                let mut details = Map::new();
                let approval_type = match method.as_str() {
                    "item/fileChange/requestApproval" => ApprovalType::File,
                    "item/permissions/requestApproval" => ApprovalType::Permissions,
                    _ => ApprovalType::Command,
                };
                let offered = if approval_type == ApprovalType::Command {
                    command_details(p)?;
                    if !matches!(
                        p.get("kind").and_then(Value::as_str),
                        None | Some("command" | "writeStdin")
                    ) {
                        return Err(ErrorCode::InvalidRequest);
                    }
                    if let Some(profile) = p.get("additionalPermissions").filter(|v| !v.is_null()) {
                        permission_profile(profile)?;
                    }
                    for k in [
                        "kind",
                        "approvalId",
                        "environmentId",
                        "command",
                        "cwd",
                        "reason",
                        "networkApprovalContext",
                        "commandActions",
                        "additionalPermissions",
                    ] {
                        if let Some(v) = p.get(k) {
                            details.insert(k.into(), v.clone());
                        }
                    }
                    command_decisions(p)?
                } else if approval_type == ApprovalType::File {
                    closed(
                        p,
                        &[
                            "threadId",
                            "turnId",
                            "itemId",
                            "startedAtMs",
                            "reason",
                            "grantRoot",
                        ],
                    )?;
                    optional_strings(p, &["reason", "grantRoot"])?;
                    for k in ["reason", "grantRoot"] {
                        if let Some(v) = p.get(k) {
                            details.insert(k.into(), v.clone());
                        }
                    }
                    details.insert(
                        "fileChanges".into(),
                        file_item
                            .and_then(|i| i.get("changes"))
                            .cloned()
                            .unwrap_or_else(|| {
                                Value::String("Exact native file-change details unavailable".into())
                            }),
                    );
                    details.insert("scopeNotice".into(),Value::String("Session grant concerns these files; grantRoot enforcement is not guaranteed by stock Codex".into()));
                    vec![
                        json!("accept"),
                        json!("acceptForSession"),
                        json!("decline"),
                        json!("cancel"),
                    ]
                } else {
                    closed(
                        p,
                        &[
                            "threadId",
                            "turnId",
                            "itemId",
                            "startedAtMs",
                            "cwd",
                            "reason",
                            "environmentId",
                            "permissions",
                        ],
                    )?;
                    optional_strings(p, &["reason", "environmentId"])?;
                    let profile = &p["permissions"];
                    permission_profile(profile)?;
                    string(p, "cwd")?;
                    for k in ["cwd", "reason", "environmentId", "permissions"] {
                        if let Some(v) = p.get(k) {
                            details.insert(k.into(), v.clone());
                        }
                    }
                    vec![
                        json!({"permissions":{},"scope":"turn"}),
                        {
                            let mut m = Map::new();
                            m.insert("permissions".into(), profile.clone());
                            m.insert("scope".into(), Value::String("turn".into()));
                            Value::Object(m)
                        },
                        {
                            let mut m = Map::new();
                            m.insert("permissions".into(), profile.clone());
                            m.insert("scope".into(), Value::String("session".into()));
                            Value::Object(m)
                        },
                    ]
                };
                if offered.is_empty() {
                    return Err(ErrorCode::InvalidRequest);
                }
                let mut rendered = Vec::new();
                for (i, v) in offered.into_iter().enumerate() {
                    let (label, scope, result) = if approval_type == ApprovalType::Permissions {
                        let scope = if v["scope"] == "session" {
                            ApprovalScope::Session
                        } else {
                            ApprovalScope::Turn
                        };
                        let label = if i == 0 {
                            "Deny permissions"
                        } else if i == 1 {
                            "Allow requested permissions for turn"
                        } else {
                            "Allow requested permissions for session"
                        };
                        (label, scope, v)
                    } else {
                        let (label, scope) = decision(&v)?;
                        let mut result = Map::new();
                        result.insert("decision".into(), v);
                        (label, scope, Value::Object(result))
                    };
                    rendered.push(ApprovalChoice {
                        choice_id: Id(format!("choice-{i}")),
                        label: Label(label.into()),
                        scope,
                        details: Some(card_content(
                            state,
                            target,
                            Mime::ApplicationJson,
                            result.to_string(),
                        )?),
                    });
                    choices.push(result);
                }
                Ok(Presentation::CodexApproval(PresentationCodexApproval {
                    approval_type,
                    title: text_content(
                        state,
                        target,
                        match approval_type {
                            ApprovalType::Command => "Codex command approval",
                            ApprovalType::File => "Codex file-change approval",
                            ApprovalType::Permissions => "Codex permissions approval",
                        }
                        .into(),
                    )?,
                    details: card_content(
                        state,
                        target,
                        Mime::ApplicationJson,
                        Value::Object(details).to_string(),
                    )?,
                    choices: rendered,
                }))
            }
            "item/tool/requestUserInput" => {
                closed(
                    p,
                    &[
                        "isBlocking",
                        "threadId",
                        "turnId",
                        "itemId",
                        "questions",
                        "autoResolutionMs",
                    ],
                )?;
                if p.get("autoResolutionMs")
                    .is_some_and(|v| !v.is_null() && v.as_u64().is_none())
                {
                    return Err(ErrorCode::InvalidRequest);
                }
                native_id(p, "turnId")?;
                native_id(p, "itemId")?;
                let questions = p["questions"]
                    .as_array()
                    .ok_or(ErrorCode::InvalidRequest)?
                    .iter()
                    .map(|q| {
                        closed(
                            q,
                            &["id", "header", "question", "isOther", "isSecret", "options"],
                        )?;
                        let options = match q.get("options") {
                            None | Some(Value::Null) => Vec::new(),
                            Some(v) => v
                                .as_array()
                                .ok_or(ErrorCode::InvalidRequest)?
                                .iter()
                                .map(|o| {
                                    closed(o, &["label", "description"])?;
                                    Ok(QuestionOption {
                                        label: Label(string(o, "label")?),
                                        description: text_content(
                                            state,
                                            target,
                                            string(o, "description")?,
                                        )?,
                                    })
                                })
                                .collect::<SecurityResult<Vec<_>>>()?,
                        };
                        Ok(Question {
                            id: native_id(q, "id")?,
                            header: string(q, "header")?,
                            prompt: text_content(state, target, string(q, "question")?)?,
                            is_other: boolean(q, "isOther", Some(false))?,
                            is_secret: boolean(q, "isSecret", Some(false))?,
                            options,
                        })
                    })
                    .collect::<SecurityResult<Vec<_>>>()?;
                Ok(Presentation::CodexQuestions(PresentationCodexQuestions {
                    is_blocking: boolean(p, "isBlocking", None)?,
                    questions,
                }))
            }
            "mcpServer/elicitation/request" => {
                string(p, "serverName")?;
                closed(
                    p,
                    match p["mode"].as_str() {
                        Some("form") => &[
                            "threadId",
                            "turnId",
                            "serverName",
                            "mode",
                            "_meta",
                            "message",
                            "requestedSchema",
                        ],
                        Some("url") => &[
                            "threadId",
                            "turnId",
                            "serverName",
                            "mode",
                            "_meta",
                            "message",
                            "url",
                            "elicitationId",
                        ],
                        _ => return Err(ErrorCode::InvalidRequest),
                    },
                )?;
                if let Some(turn) = p.get("turnId").filter(|v| !v.is_null()) {
                    let id = Id(turn.as_str().ok_or(ErrorCode::InvalidRequest)?.into());
                    id.validate().map_err(policy)?;
                }
                match p["mode"].as_str() {
                    Some("form") => Ok(Presentation::CodexForm(PresentationCodexForm {
                        title: text_content(state, target, string(p, "message")?)?,
                        fields: form_fields(state, target, &p["requestedSchema"])?,
                    })),
                    Some("url") => {
                        native_id(p, "elicitationId")?;
                        Ok(Presentation::CodexUrl(PresentationCodexUrl {
                            title: text_content(state, target, string(p, "message")?)?,
                            url: string(p, "url")?,
                        }))
                    }
                    _ => Err(ErrorCode::InvalidRequest),
                }
            }
            _ => Err(ErrorCode::InvalidRequest),
        }
    })();
    let presentation = match projected {
        Ok(p)
            if p.validate().is_ok()
                && supported_contents(&p)
                && native_bytes(&p) < FRAME_MAX - 65_536 =>
        {
            p
        }
        _ => {
            choices.clear();
            Presentation::DesktopOnly(PresentationDesktopOnly{title:text_content(state,target,"Codex request requires desktop".into())?,reason:"Unsupported, malformed or oversized native request; no partial grant or invented response".into()})
        }
    };
    let kind = match &presentation {
        Presentation::CodexApproval(_) => PresentationKind::CodexApproval,
        Presentation::CodexQuestions(_) => PresentationKind::CodexQuestions,
        Presentation::CodexForm(_) => PresentationKind::CodexForm,
        Presentation::CodexUrl(_) => PresentationKind::CodexUrl,
        _ => PresentationKind::DesktopOnly,
    };
    let pending = PendingRequest {
        request_key: Uuid::random(),
        provider: Provider::Codex,
        kind,
        presentation,
    };
    pending.validate().map_err(policy)?;
    Ok(Dialog {
        pending,
        id,
        choices,
        thread,
        sent: false,
        fingerprint: sha256(&serde_json::to_vec(frame).map_err(policy)?),
    })
}

type NativeSocket = WebSocketStream<UnixStream>;
type NativeWriter = futures_util::stream::SplitSink<NativeSocket, Message>;
#[derive(Debug, PartialEq, Eq)]
struct CreationReadRejection {
    code: i64,
    category: &'static str,
}
impl CreationReadRejection {
    fn proof(&self) -> String {
        format!(
            "Native original-thread materializing read rejected; code={}; category={}; no fallback",
            self.code, self.category,
        )
    }
}
fn creation_read_rejection(value: &Value, expected: &Id) -> Option<CreationReadRejection> {
    if value.get("result").is_some() {
        return None;
    }
    let error = value.get("error")?;
    let code = error.get("code")?.as_i64()?;
    if !matches!(code, -32602..=-32600) {
        return None;
    }
    let message = error.get("message")?.as_str()?;
    // Pinned thread_processor.rs read guards; compare borrowed original-ID slices only.
    let category = if code == -32600 && message == "ephemeral threads do not support includeTurns" {
        "ephemeral_include_turns"
    } else if code == -32600
        && message.strip_prefix("thread not loaded: ") == Some(expected.0.as_str())
    {
        "thread_not_loaded"
    } else if code == -32600
        && message.strip_prefix("thread ").and_then(|rest| {
            rest.strip_suffix(
                " is not materialized yet; includeTurns is unavailable before first user message",
            )
        }) == Some(expected.0.as_str())
    {
        "include_turns_unmaterialized"
    } else if code == -32600
        && message.strip_prefix("thread ").and_then(|rest| {
            rest.strip_suffix(
            " is not materialized yet; thread/turns/list is unavailable before first user message",
        )
        }) == Some(expected.0.as_str())
    {
        "full_turns_unmaterialized"
    } else {
        "unclassified_rejection"
    };
    Some(CreationReadRejection { code, category })
}
struct Callback {
    response_limit: usize,
    response_bytes: Option<Arc<AtomicUsize>>,
    id: NativeRpcId,
    reply: oneshot::Sender<SecurityResult<Value>>,
    snapshot: bool,
    read_rejection: Option<(Id, oneshot::Sender<CreationReadRejection>)>,
    thread_start: Option<herdr::CodexThreadStartObservation>,
}
fn ordinary_response(value: &Value) -> SecurityResult<Value> {
    if value.get("result").is_some() == value.get("error").is_some() {
        return Err(ErrorCode::NotReady);
    }
    if let Some(error) = value.get("error") {
        let code = error
            .get("code")
            .and_then(Value::as_number)
            .and_then(|n| n.to_string().parse::<i64>().ok());
        if error.get("message").and_then(Value::as_str).is_none() {
            return Err(ErrorCode::NotReady);
        }
        // Only source-defined pre-admission JSON-RPC rejection classes prove no effect.
        return Err(if matches!(code, Some(-32602..=-32600)) {
            ErrorCode::BackendError
        } else {
            ErrorCode::NotReady
        });
    }
    Ok(value["result"].clone())
}
fn complete_callback(
    callbacks: &Mutex<Vec<Callback>>,
    value: &Value,
    order: u64,
    boundary: &AtomicU64,
) -> io::Result<()> {
    let id = native_json::rpc_id(&value["id"])?;
    let callback = {
        let mut pending = callbacks.lock();
        pending
            .iter()
            .position(|p| p.id == id)
            .map(|i| pending.remove(i))
    };
    if let Some(callback) = callback {
        let result = ordinary_response(value);
        let thread_start = callback.thread_start.clone();
        if let Some(observation) = &thread_start {
            observation.matched(result.as_ref().err().copied());
        }
        let unknown = result == Err(ErrorCode::NotReady);
        if callback.snapshot && result.is_ok() {
            boundary.store(order, Ordering::Release);
        }
        if result == Err(ErrorCode::BackendError)
            && let Some((expected, reply)) = callback.read_rejection
            && let Some(rejection) = creation_read_rejection(value, &expected)
        {
            let _ = reply.send(rejection);
        }
        if let Some(observation) = thread_start {
            observation.delivered(callback.reply.send(result).is_ok());
        } else {
            let _ = callback.reply.send(result);
        }
        if unknown {
            return Err(invalid());
        }
    }
    Ok(())
}
struct NativeEvent {
    value: Value,
    order: u64,
    _bytes: tokio::sync::OwnedSemaphorePermit,
}
#[derive(Clone)]
struct ReaderCapture {
    reporter: herdr::CodexInvalidationReporter,
    target: Target,
    epoch: Uuid,
}
#[derive(Clone, Copy)]
struct PendingReaderFailure {
    guard: &'static str,
    code: ErrorCode,
    io: &'static str,
    errno: i32,
}
#[derive(Default)]
struct ReaderCaptureState {
    cancelled: bool,
    current: Option<ReaderCapture>,
    pending: Option<PendingReaderFailure>,
}
// This outer guard also closes the window if admission is cancelled before the
// original effect future is ever polled.
struct ThreadStartWindowGuard(Option<herdr::CodexThreadStartObservation>);
impl Drop for ThreadStartWindowGuard {
    fn drop(&mut self) {
        if let Some(observation) = &self.0 {
            observation.freeze();
        }
    }
}
struct ThreadEffectDrop {
    thread_start: Option<herdr::CodexThreadStartObservation>,
    reporter: Option<herdr::CodexInvalidationReporter>,
    reader_capture: Arc<Mutex<ReaderCaptureState>>,
    phase: &'static str,
    armed: bool,
}
impl ThreadEffectDrop {
    fn new(
        reporter: Option<herdr::CodexInvalidationReporter>,
        reader_capture: Arc<Mutex<ReaderCaptureState>>,
        thread_start: Option<herdr::CodexThreadStartObservation>,
    ) -> Self {
        Self {
            reporter,
            reader_capture,
            thread_start,
            phase: "thread_effect_drop_start_send",
            armed: true,
        }
    }
    fn phase(&mut self, phase: &'static str) {
        self.phase = phase;
    }
    fn disarm(&mut self) {
        if let Some(observation) = &self.thread_start {
            observation.freeze();
        }
        self.armed = false;
    }
}
impl Drop for ThreadEffectDrop {
    fn drop(&mut self) {
        // Freeze before either first-cause emission; cleanup can deliver callbacks.
        let snapshot = self
            .thread_start
            .as_ref()
            .map(|observation| observation.freeze());
        if !self.armed {
            return;
        }
        let Some(reporter) = &self.reporter else {
            return;
        };
        if let Some(pending) = self.reader_capture.lock().pending {
            reporter.prebinding("reader", pending.guard, pending.code, pending.io);
        }
        if self.phase == "thread_effect_drop_start_reply"
            && let Some(snapshot) = snapshot
        {
            reporter.thread_start_effect_drop(snapshot);
        } else {
            reporter.prebinding("thread", self.phase, ErrorCode::NotReady, "none");
        }
    }
}
fn reader_capture_closed(
    capture: &Arc<Mutex<ReaderCaptureState>>,
    guard: &'static str,
    code: ErrorCode,
    io: &'static str,
    errno: i32,
) {
    let current = {
        let mut state = capture.lock();
        if state.cancelled {
            return;
        }
        if let Some(current) = state.current.clone() {
            Some(current)
        } else {
            state.pending.get_or_insert(PendingReaderFailure {
                guard,
                code,
                io,
                errno,
            });
            None
        }
    };
    if let Some(current) = current {
        current.reporter.target_closed(
            &current.target,
            &current.epoch,
            "reader",
            guard,
            code,
            (io, errno),
        );
    }
}
fn reader_capture(capture: &Arc<Mutex<ReaderCaptureState>>, guard: &'static str, code: ErrorCode) {
    reader_capture_closed(capture, guard, code, "none", 0);
}
fn reader_capture_error(
    capture: &Arc<Mutex<ReaderCaptureState>>,
    guard: &'static str,
    code: ErrorCode,
    error: &io::Error,
) {
    let (io, errno) = herdr::codex_invalidation_io(error);
    reader_capture_closed(capture, guard, code, io, errno);
}
async fn read_native_frames(
    mut reader: futures_util::stream::SplitStream<NativeSocket>,
    pending: Arc<Mutex<Vec<Callback>>>,
    events: tokio::sync::mpsc::Sender<NativeEvent>,
    boundary: Arc<AtomicU64>,
    capture: Arc<Mutex<ReaderCaptureState>>,
    thread_start: Arc<OnceLock<herdr::CodexThreadStartObservation>>,
    skills: Arc<Mutex<SkillsObservation>>,
) -> Result<(), ErrorCode> {
    let bytes = Arc::new(tokio::sync::Semaphore::new(16 * 1024 * 1024));
    let result = async {
        let mut order = 0u64;
        while let Some(message) = reader.next().await {
            let message = match message {
                Ok(message) => message,
                Err(error) => {
                    let code = policy(&error);
                    if let tokio_tungstenite::tungstenite::Error::Io(io) = &error {
                        reader_capture_error(&capture, "frame_read", code, io);
                    } else {
                        let guard = match &error {
                            tokio_tungstenite::tungstenite::Error::ConnectionClosed
                            | tokio_tungstenite::tungstenite::Error::AlreadyClosed => {
                                "frame_closed"
                            }
                            tokio_tungstenite::tungstenite::Error::Protocol(..) => "frame_protocol",
                            tokio_tungstenite::tungstenite::Error::Capacity(..) => "frame_capacity",
                            _ => "frame_other",
                        };
                        reader_capture(&capture, guard, code);
                    }
                    return Err(code);
                }
            };
            if message.is_close() {
                reader_capture(&capture, "frame_close", ErrorCode::NotReady);
                return Err(ErrorCode::NotReady);
            }
            if !matches!(message, Message::Text(_) | Message::Binary(_)) {
                continue;
            }
            if let Some(observation) = thread_start.get() {
                observation.complete_data_frame();
            }
            order = order.checked_add(1).ok_or(ErrorCode::NotReady)?;
            let payload = codex_relay::message_bytes(&message).map_err(|error| {
                let code = policy(error);
                reader_capture(&capture, "frame_bytes", code);
                code
            })?;
            let value = native_json::parse(payload, NATIVE).map_err(|error| {
                let code = policy(error);
                reader_capture(&capture, "frame_parse", code);
                code
            })?;
            skills.lock().observe(&value);
            // The diagnostic observes this already-decoded frame only while its
            // original callback remains registered. It does not inspect payloads.
            if let Some(observation) = thread_start.get() {
                observation.decoded_frame();
            }
            if value.get("method").is_none() && value.get("id").is_some() {
                let id = native_json::rpc_id(&value["id"]).map_err(policy)?;
                let over_budget = {
                    let pending = pending.lock();
                    let callback = pending.iter().find(|p| p.id == id);
                    // Reuse the existing lookup. Invalid IDs never reach this branch;
                    // ordinary links have no observation or additional lock.
                    if callback.is_none()
                        && let Some(observation) = thread_start.get()
                    {
                        observation.unmatched_response();
                    }
                    callback.is_some_and(|p| payload.len()>p.response_limit) || callback.and_then(|p| p.response_bytes.as_ref())
                        .is_some_and(|bytes| {
                            bytes
                                .fetch_add(payload.len(), Ordering::AcqRel)
                                .saturating_add(payload.len())
                                > 16 * 1024 * 1024
                        })
                };
                if over_budget {
                    let callback = {
                        let mut pending = pending.lock();
                        pending
                            .iter()
                            .position(|p| p.id == id)
                            .map(|index| pending.remove(index))
                    };
                    if let Some(callback) = callback {
                        if let Some(observation) = callback.thread_start {
                            observation.matched(Some(ErrorCode::ResourceLimit));
                            observation.delivered(
                                callback.reply.send(Err(ErrorCode::ResourceLimit)).is_ok(),
                            );
                        } else {
                            let _ = callback.reply.send(Err(ErrorCode::ResourceLimit));
                        }
                    }
                    continue;
                }
                complete_callback(&pending, &value, order, &boundary).map_err(|error| {
                    let code = policy(error);
                    reader_capture(&capture, "callback_frame", code);
                    code
                })?;
            } else {
                // This is the existing method-bearing read path. The opt-in
                // diagnostic keeps only its closed family and the presence of
                // an ID, before ordinary event admission remains unchanged.
                if let Some(method) = value.get("method").and_then(Value::as_str)
                    && let Some(observation) = thread_start.get()
                {
                    observation.method(method, value.get("id").is_some());
                }
                let permit = bytes
                    .clone()
                    .try_acquire_many_owned(message.len().max(1) as u32)
                    .map_err(|_| {
                        reader_capture(&capture, "event_capacity", ErrorCode::NotReady);
                        ErrorCode::NotReady
                    })?;
                events
                    .try_send(NativeEvent {
                        value,
                        order,
                        _bytes: permit,
                    })
                    .map_err(|_| {
                        reader_capture(&capture, "event_queue", ErrorCode::NotReady);
                        ErrorCode::NotReady
                    })?;
            }
        }
        Err(ErrorCode::NotReady)
    }
    .await;
    skills.lock().clear();
    if let Err(code) = &result {
        reader_capture(&capture, "reader_terminal", *code);
    }
    for callback in pending.lock().drain(..) {
        if let Some(observation) = callback.thread_start {
            // A terminal drain is not a decoded native response or matching reply.
            observation.freeze();
        }
        let _ = callback.reply.send(Err(ErrorCode::NotReady));
    }
    result
}
#[derive(Default)]
struct ReadBudget {
    rpcs: usize,
    bytes: Arc<AtomicUsize>,
}
impl ReadBudget {
    async fn rpc(&mut self, link: &Link, method: &str, params: Value) -> SecurityResult<Value> {
        if self.rpcs >= 64 || self.bytes.load(Ordering::Acquire) >= 16 * 1024 * 1024 {
            return Err(ErrorCode::ResourceLimit);
        }
        self.rpcs += 1;
        link.rpc_monitored(method, params, Some(self.bytes.clone()))
            .await
    }
}
#[derive(Default)]
struct SkillsObservation {
    thread: Option<Id>, cwd: Option<String>, generation: Option<Uuid>, invalid: bool,
    before_pin: Option<String>, observed_before_pin: bool,
    publication: Option<(Arc<State>,Target,Uuid)>,
}
impl SkillsObservation {
    fn arm(&mut self, thread: &Id) { self.thread=Some(thread.clone()); }
    fn observe(&mut self, frame:&Value) {
        if frame["method"]!="thread/settings/updated" || self.thread.is_none() { return; }
        let params=&frame["params"];
        // A missing/malformed routing identity cannot prove that this notification is unrelated.
        if params["threadId"].as_str().is_some_and(|id|self.thread.as_ref().is_some_and(|t|t.0!=id)) { return; }
        let cwd=params["threadSettings"]["cwd"].as_str().filter(|s|valid_skills_cwd(s));
        if params["threadId"].as_str().is_none() || cwd.is_none() { self.invalid=true; }
        if let Some(pinned)=&self.cwd {
            if cwd!=Some(pinned.as_str()) { self.invalid=true; }
        } else {
            if self.observed_before_pin && self.before_pin.as_deref()!=cwd { self.invalid=true; }
            self.before_pin=cwd.map(str::to_owned); self.observed_before_pin=true;
        }
        self.generation=if self.invalid {None}else{Some(Uuid::random())};
        self.publish();
    }
    fn pin(&mut self, resumed:&Value) {
        let cwd=resumed.get("cwd").and_then(Value::as_str).filter(|s|valid_skills_cwd(s));
        if cwd.is_none() || (self.observed_before_pin && self.before_pin.as_deref()!=cwd) { self.invalid=true; }
        self.cwd=cwd.map(str::to_owned);
        self.generation=if self.invalid {None}else{Some(Uuid::random())};
    }
    fn publish(&self) { if let Some((state,target,source))=&self.publication { let _=state.skills_generation(target,source,self.generation.clone()); } }
    fn snapshot(&self)->SecurityResult<(String,Uuid)> {
        if self.invalid { return Err(ErrorCode::NotReady); }
        Ok((self.cwd.clone().ok_or(ErrorCode::NotReady)?,self.generation.clone().ok_or(ErrorCode::NotReady)?))
    }
    fn clear(&mut self) { if self.invalid && self.generation.is_none() {return;} self.invalid=true;self.generation=None;self.publish(); }
}
fn valid_skills_cwd(cwd:&str)->bool { cwd.starts_with('/') && cwd.len()<=4096 && !cwd.chars().any(char::is_control) }

struct Link {
    skills: Arc<Mutex<SkillsObservation>>,
    writer: tokio::sync::Mutex<NativeWriter>,
    callbacks: Arc<Mutex<Vec<Callback>>>,
    fd: OwnedFd,
    reader: Mutex<Option<tokio::task::JoinHandle<Result<(), ErrorCode>>>>,
    reader_capture: Arc<Mutex<ReaderCaptureState>>,
    thread_start: Arc<OnceLock<herdr::CodexThreadStartObservation>>,
    snapshot_boundary: Arc<AtomicU64>,
}
impl Link {
    async fn connect(
        owner: &CodexOwner,
        epoch: &OwnerEpoch,
    ) -> io::Result<(
        Arc<Self>,
        tokio::sync::mpsc::Receiver<NativeEvent>,
        ProcessEvidence,
    )> {
        let (socket, process) = codex_relay::owner_stream(
            &owner.socket_host,
            &owner.socket_agent,
            epoch,
            codex_relay::websocket_config(NATIVE.bytes, 16 * 1024 * 1024),
        )
        .await?;
        let fd = socket.get_ref().as_fd().try_clone_to_owned()?;
        let (writer, reader) = socket.split();
        let callbacks = Arc::new(Mutex::new(Vec::<Callback>::new()));
        let pending = callbacks.clone();
        // Native reducer capture uses its existing256-event bound; the independent relay
        // retains its separate32-frame bound. Raw capture also has an explicit16-MiB budget.
        let (events, rx) = tokio::sync::mpsc::channel(256);
        let snapshot_boundary = Arc::new(AtomicU64::new(0));
        let boundary = snapshot_boundary.clone();
        let reader_capture = Arc::new(Mutex::new(ReaderCaptureState::default()));
        let thread_start = Arc::new(OnceLock::new());
        let skills = Arc::new(Mutex::new(SkillsObservation::default()));
        let job = tokio::spawn(read_native_frames(
            reader,
            pending,
            events,
            boundary,
            reader_capture.clone(),
            thread_start.clone(),
            skills.clone(),
        ));
        let link = Arc::new(Self {
            skills,
            writer: tokio::sync::Mutex::new(writer),
            callbacks,
            fd,
            reader: Mutex::new(Some(job)),
            reader_capture,
            thread_start,
            snapshot_boundary,
        });
        let ready=link.rpc("initialize",json!({"clientInfo":{"name":"goatr","title":"Goatr","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true,"requestAttestation":false,"optOutNotificationMethods":[]}})).await;
        if !ready.as_ref().is_ok_and(|value| value.is_object()) {
            link.close().await;
            return Err(invalid());
        }
        if link
            .send(serde_json::to_vec(&json!({"method":"initialized"}))?)
            .await
            .is_err()
        {
            link.close().await;
            return Err(invalid());
        }
        if !epoch
            .validate(&owner.socket_host, &owner.socket_agent)
            .is_ok_and(|current| current == process)
        {
            link.close().await;
            return Err(invalid());
        }
        Ok((link, rx, process))
    }
    fn set_reader_capture(
        &self,
        reporter: Option<herdr::CodexInvalidationReporter>,
        target: &Target,
        epoch: &Uuid,
    ) {
        if let Some(reporter) = reporter {
            let mut state = self.reader_capture.lock();
            if state.cancelled || state.current.is_some() {
                return;
            }
            let current = ReaderCapture {
                reporter,
                target: target.clone(),
                epoch: epoch.clone(),
            };
            // Keep attachment ordered with the first pending error before newer reads can report.
            if let Some(pending) = state.pending.take() {
                current.reporter.target_closed(
                    &current.target,
                    &current.epoch,
                    "reader",
                    pending.guard,
                    pending.code,
                    (pending.io, pending.errno),
                );
            }
            state.current = Some(current);
        }
    }
    fn shutdown(&self) {
        self.skills.lock().clear();
        // Explicit local teardown must not masquerade as the original native reader failure.
        self.reader_capture.lock().cancelled = true;
        // SAFETY: shutdown affects this live connected descriptor and all its duplicates.
        unsafe {
            libc::shutdown(std::os::fd::AsRawFd::as_raw_fd(&self.fd), libc::SHUT_RDWR);
        }
    }
    async fn close(&self) {
        self.skills.lock().clear();
        self.shutdown();
        let job = self.reader.lock().take();
        if let Some(job) = job {
            let _ = job.await;
        }
    }
    // Only called after the event channel closes itself, never after local shutdown.
    async fn reader_failure(&self) -> ErrorCode {
        let job = self.reader.lock().take();
        match job {
            Some(job) => match job.await {
                Ok(Err(failure)) => failure,
                Ok(Ok(())) => ErrorCode::NotReady,
                Err(_) => ErrorCode::NotReady,
            },
            None => ErrorCode::NotReady,
        }
    }
    async fn send(&self, bytes: Vec<u8>) -> SecurityResult<()> {
        let mut writer = tokio::time::timeout(Duration::from_secs(5), self.writer.lock())
            .await
            .map_err(|_| ErrorCode::Busy)?;
        tokio::time::timeout(
            Duration::from_secs(5),
            writer.send(Message::Text(
                String::from_utf8(bytes).map_err(policy)?.into(),
            )),
        )
        .await
        .map_err(|_| ErrorCode::Timeout)?
        .map_err(policy)
    }
    fn callback(
        &self,
        id: NativeRpcId,
        snapshot: bool,
    ) -> SecurityResult<oneshot::Receiver<SecurityResult<Value>>> {
        self.callback_with_observation(id, snapshot, None, None)
    }
    fn callback_with_read_rejection(
        &self,
        id: NativeRpcId,
        snapshot: bool,
        read_rejection: Option<(Id, oneshot::Sender<CreationReadRejection>)>,
    ) -> SecurityResult<oneshot::Receiver<SecurityResult<Value>>> {
        self.callback_with_observation(id, snapshot, read_rejection, None)
    }
    fn callback_with_thread_start(
        &self,
        id: NativeRpcId,
        snapshot: bool,
        thread_start: herdr::CodexThreadStartObservation,
    ) -> SecurityResult<oneshot::Receiver<SecurityResult<Value>>> {
        self.callback_with_observation(id, snapshot, None, Some(thread_start))
    }
    fn callback_with_observation(
        &self,
        id: NativeRpcId,
        snapshot: bool,
        read_rejection: Option<(Id, oneshot::Sender<CreationReadRejection>)>,
        thread_start: Option<herdr::CodexThreadStartObservation>,
    ) -> SecurityResult<oneshot::Receiver<SecurityResult<Value>>> {
        let (tx, rx) = oneshot::channel();
        let mut callbacks = self.callbacks.lock();
        if callbacks.len() >= 64 || callbacks.iter().any(|c| c.id == id) {
            return Err(ErrorCode::Busy);
        }
        callbacks.push(Callback {
            response_limit: 16 * 1024 * 1024,
            response_bytes: None,
            id,
            reply: tx,
            snapshot,
            read_rejection,
            thread_start: thread_start.clone(),
        });
        if let Some(observation) = thread_start {
            // Set only for the single opted-in original start callback; ordinary
            // frame readers only inspect an empty lock-free slot.
            let _ = self.thread_start.set(observation.clone());
            observation.activate();
        }
        Ok(rx)
    }
    async fn rpc(&self, method: &str, params: Value) -> SecurityResult<Value> {
        self.rpc_monitored(method, params, None).await
    }
    async fn rpc_monitored(
        &self,
        method: &str,
        params: Value,
        bytes: Option<Arc<AtomicUsize>>,
    ) -> SecurityResult<Value> {
        let id = Uuid::random().0;
        let rx = self.callback(NativeRpcId::String(id.clone()), method == "thread/resume")?;
        if let Some(bytes) = bytes
            && let Some(callback) = self
                .callbacks
                .lock()
                .iter_mut()
                .find(|c| c.id == NativeRpcId::String(id.clone()))
        {
            callback.response_bytes = Some(bytes);
        }
        if method=="skills/list" { if let Some(callback)=self.callbacks.lock().iter_mut().find(|c|c.id==NativeRpcId::String(id.clone())) { callback.response_limit=1024*1024; } }
        let result = async {
            self.send(
                serde_json::to_vec(&json!({"id":id,"method":method,"params":params}))
                    .map_err(policy)?,
            )
            .await?;
            tokio::time::timeout(Duration::from_secs(10), rx)
                .await
                .map_err(|_| ErrorCode::Timeout)?
                .map_err(|_| ErrorCode::NotReady)?
        }
        .await;
        if result.is_err() {
            self.callbacks
                .lock()
                .retain(|c| c.id != NativeRpcId::String(id.clone()));
        }
        result
    }
    async fn loaded(&self, expected: &Id) -> SecurityResult<bool> {
        self.loaded_budgeted(expected, &mut ReadBudget::default())
            .await
    }
    async fn loaded_budgeted(
        &self,
        expected: &Id,
        budget: &mut ReadBudget,
    ) -> SecurityResult<bool> {
        let mut found = false;
        let mut cursor: Option<String> = None;
        let mut previous: Option<Id> = None;
        let mut seen = HashSet::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let mut params = json!({"limit":100});
            if let Some(c) = &cursor {
                params["cursor"] = Value::String(c.clone());
            }
            let result = match tokio::time::timeout_at(
                deadline,
                budget.rpc(self, "thread/loaded/list", params),
            )
            .await
            {
                Ok(r) => r?,
                Err(_) => {
                    self.shutdown();
                    return Err(ErrorCode::Timeout);
                }
            };
            for value in result["data"].as_array().ok_or(ErrorCode::NotReady)? {
                let id = Id(value.as_str().ok_or(ErrorCode::NotReady)?.into());
                id.validate().map_err(policy)?;
                if previous.as_ref().is_some_and(|p| p.0 >= id.0) {
                    return Err(ErrorCode::NotReady);
                }
                found |= &id == expected;
                previous = Some(id);
            }
            let next = next_cursor(&result)?;
            let Some(next) = next else { break };
            if !seen.insert(next.clone()) {
                return Err(ErrorCode::NotReady);
            }
            cursor = Some(next);
        }
        Ok(found)
    }
}
impl Drop for Link {
    fn drop(&mut self) {
        self.shutdown();
        if let Some(job) = self.reader.lock().take() {
            job.abort();
        }
    }
}
fn next_cursor(v: &Value) -> SecurityResult<Option<String>> {
    match v.get("nextCursor") {
        Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => {
            if s.is_empty() || s.len() > 65_536 {
                return Err(ErrorCode::ResourceLimit);
            }
            Ok(Some(s.clone()))
        }
        _ => Err(ErrorCode::NotReady),
    }
}
fn safe_native(value: &Value) -> Value {
    fn codex_private(v: &mut Value) {
        match v {
            Value::Object(map) => {
                map.retain(|key, _| {
                    let key = key.to_ascii_lowercase().replace(['_', '-'], "");
                    !key.ends_with("token")
                        && !matches!(
                            key.as_str(),
                            "meta"
                                | "headers"
                                | "attestation"
                                | "challenge"
                                | "elicitationid"
                                | "url"
                                | "secretanswer"
                                | "privatepayload"
                        )
                });
                for v in map.values_mut() {
                    codex_private(v);
                }
            }
            Value::Array(values) => {
                for v in values {
                    codex_private(v);
                }
            }
            _ => {}
        }
    }
    let mut safe = super::omp::redacted(value);
    codex_private(&mut safe);
    safe
}
/// Projection is display-only; credentials/capability payloads never become originals.
pub fn project_item(
    state: &State,
    target: &Target,
    turn: &Id,
    item: &Value,
    order: u64,
    source: RecordSource,
    complete: bool,
) -> SecurityResult<SafeHistory> {
    project_safe_item(
        state,
        target,
        turn,
        &safe_native(item),
        order,
        source,
        (complete, None),
    )
}
fn project_safe_item(
    state: &State,
    target: &Target,
    turn: &Id,
    item: &Value,
    order: u64,
    source: RecordSource,
    lifecycle: (bool, Option<Uint>),
) -> SecurityResult<SafeHistory> {
    SafeHistory::from_redacted_display(project_item_record(
        state, target, turn, item, order, source, lifecycle,
    )?)
}
fn project_item_record(
    state: &State,
    target: &Target,
    turn: &Id,
    item: &Value,
    order: u64,
    source: RecordSource,
    lifecycle: (bool, Option<Uint>),
) -> SecurityResult<HistoryRecord> {
    let (complete, timestamp) = lifecycle;
    let id = native_id(item, "id")?;
    let typ = string(item, "type")?;
    let mut record = HistoryRecord {
        id,
        parent_id: None,
        turn_id: Some(turn.clone()),
        order: Uint(order),
        timestamp,
        role: Role::Unknown,
        kind: RecordKind::Unsupported,
        text: None,
        tool: None,
        original: None,
        source,
        complete,
        source_truncated: false,
    };
    match typ.as_str() {
        "userMessage" => {
            record.role = Role::User;
            record.kind = RecordKind::Message;
            let values = item["content"].as_array().ok_or(ErrorCode::NotReady)?;
            let text = values
                .iter()
                .map(|v| match v["type"].as_str() {
                    Some("text") => string(v, "text"),
                    Some("image" | "localImage") => {
                        Ok("[Native image; no automatic loading]".into())
                    }
                    _ => Ok("[Unsupported native user input]".into()),
                })
                .collect::<SecurityResult<Vec<_>>>()?
                .join("\n");
            record.text = Some(text_content(state, target, text)?);
        }
        "agentMessage" => {
            record.role = Role::Assistant;
            record.kind = RecordKind::Message;
            record.text = Some(text_content(state, target, string(item, "text")?)?);
        }
        "reasoning" => {
            record.role = Role::Assistant;
            record.kind = RecordKind::Notice;
            let mut text = Vec::new();
            for key in ["summary", "content"] {
                if let Some(a) = item.get(key).and_then(Value::as_array) {
                    for v in a {
                        if let Some(s) = v.as_str() {
                            text.push(s.to_owned());
                        } else {
                            return Err(ErrorCode::NotReady);
                        }
                    }
                }
            }
            record.text = Some(text_content(state, target, text.join("\n"))?);
        }
        "commandExecution"
        | "fileChange"
        | "mcpToolCall"
        | "dynamicToolCall"
        | "collabAgentToolCall"
        | "webSearch"
        | "imageGeneration" => {
            record.role = Role::Tool;
            record.kind = RecordKind::Tool;
            let status = match item["status"].as_str() {
                Some("inProgress" | "running") => ToolStatus::Running,
                Some("completed" | "succeeded") => ToolStatus::Succeeded,
                Some("failed" | "declined") => ToolStatus::Failed,
                _ => ToolStatus::Unknown,
            };
            let mut input = std::collections::BTreeMap::new();
            for key in [
                "command",
                "cwd",
                "actions",
                "changes",
                "server",
                "tool",
                "arguments",
                "prompt",
                "query",
                "agentIds",
                "receiverThreadIds",
                "senderThreadId",
            ] {
                if let Some(v) = item.get(key) {
                    input.insert(key, v);
                }
            }
            let mut output = std::collections::BTreeMap::new();
            for key in [
                "aggregatedOutput",
                "result",
                "error",
                "exitCode",
                "content",
                "durationMs",
                "agentsStates",
                "success",
                "revisedPrompt",
            ] {
                if let Some(v) = item.get(key) {
                    output.insert(key, v);
                }
            }
            record.tool = Some(Tool {
                name: typ,
                call_id: Some(record.id.clone()),
                status,
                input: Some(content(
                    state,
                    target,
                    Mime::ApplicationJson,
                    serde_json::to_string(&input).map_err(policy)?,
                )?),
                output: Some(content(
                    state,
                    target,
                    Mime::ApplicationJson,
                    serde_json::to_string(&output).map_err(policy)?,
                )?),
            });
        }
        "plan" => {
            record.role = Role::Assistant;
            record.kind = RecordKind::Notice;
            record.text = Some(text_content(state, target, string(item, "text")?)?);
        }
        "contextCompaction" => {
            record.role = Role::System;
            record.kind = RecordKind::Notice;
            record.text = Some(text_content(
                state,
                target,
                "Native context compacted".into(),
            )?);
        }
        _ => {
            record.text = Some(text_content(
                state,
                target,
                "Unsupported native history item".into(),
            )?);
        }
    }
    if complete {
        record.original = Some(content(
            state,
            target,
            Mime::ApplicationJson,
            item.to_string(),
        )?);
    }
    Ok(record)
}
fn item_activity(record: HistoryRecord) -> Activity {
    let tool_record_id = record.tool.as_ref().map(|_| record.id);
    let text = record
        .text
        .or_else(|| record.tool.and_then(|tool| tool.output.or(tool.input)));
    Activity {
        state: ActivityState::Working,
        text,
        tool_record_id,
    }
}
#[derive(Clone)]
struct NativeRecord {
    turn: Id,
    item: Value,
    order: u64,
    complete: bool,
    timestamp: Option<Uint>,
    source: RecordSource,
    bytes: usize,
    cached: bool,
    live_base: bool,
    fingerprint: Option<[u8; 32]>,
    gap: Option<ContentUnavailable>,
}
fn native_timestamp(value: &Value) -> Option<Uint> {
    value
        .as_number()
        .and_then(|n| n.to_string().parse::<i64>().ok())
        .filter(|n| *n >= 0)
        .and_then(|n| Uint::new(n as u64).ok())
}
fn item_metadata(item: &Value) -> Value {
    let mut result = Map::new();
    for key in ["id", "type", "status"] {
        if let Some(v) = item.get(key) {
            result.insert(key.into(), v.clone());
        }
    }
    Value::Object(result)
}
fn native_fingerprint(item: &Value) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    struct Hash(Sha256);
    impl std::io::Write for Hash {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.update(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut hash = Hash(Sha256::new());
    serde_json::to_writer(&mut hash, item).expect("native display serialization");
    hash.0.finalize().into()
}
fn unavailable_activity(id: Id, r: &NativeRecord) -> SecurityResult<Activity> {
    let typ = string(&r.item, "type")?;
    let mut gap = r.gap.clone().unwrap_or(ContentUnavailable {
        reason: ContentUnavailableReason::NotCached,
        byte_length: None,
    });
    if typ != "agentMessage" {
        gap.byte_length = None;
    }
    let unavailable = Content::Unavailable(gap);
    Ok(Activity {
        state: ActivityState::Working,
        text: Some(unavailable),
        tool_record_id: (!matches!(typ.as_str(), "agentMessage" | "reasoning")).then_some(id),
    })
}
fn native_bytes<T: Serialize + ?Sized>(value: &T) -> usize {
    struct Count(usize);
    impl std::io::Write for Count {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count(0);
    serde_json::to_writer(&mut count, value).expect("native value serialization");
    count.0
}
#[derive(Clone)]
struct Continuation {
    cursor: Option<String>,
    anchor: Option<Id>,
    remaining: Arc<Vec<NativeRecord>>,
    offset: usize,
    before: u64,
    turns: Arc<Vec<HistoryTurn>>,
    turn_index: usize,
    item_cursor: Option<String>,
    seen_turns: Arc<HashSet<String>>,
    seen_items: Arc<HashSet<String>>,
}
#[derive(Clone)]
enum HistoryTurn {
    Full(Vec<NativeRecord>),
    Items(Id),
}
impl Continuation {
    fn seed(anchor: Option<Id>, before: u64) -> Self {
        Self {
            cursor: None,
            anchor,
            remaining: Arc::new(Vec::new()),
            offset: 0,
            before,
            turns: Arc::new(Vec::new()),
            turn_index: 0,
            item_cursor: None,
            seen_turns: Arc::new(HashSet::new()),
            seen_items: Arc::new(HashSet::new()),
        }
    }
    fn has_more(&self) -> bool {
        self.offset < self.remaining.len()
            || self.turn_index < self.turns.len()
            || self.cursor.is_some()
    }
    fn request(&self, thread: &Id) -> (&'static str, Value) {
        if let Some(HistoryTurn::Items(turn)) = self.turns.get(self.turn_index) {
            let mut params =
                json!({"threadId":thread.0,"turnId":turn.0,"limit":100,"sortDirection":"desc"});
            if let Some(cursor) = &self.item_cursor {
                params["cursor"] = json!(cursor);
            }
            ("thread/items/list", params)
        } else {
            let mut params =
                json!({"threadId":thread.0,"limit":50,"sortDirection":"desc","itemsView":"full"});
            if let Some(cursor) = &self.cursor {
                params["cursor"] = json!(cursor);
            }
            ("thread/turns/list", params)
        }
    }
    fn advance_cursor(
        seen: &mut Arc<HashSet<String>>,
        page: &Value,
    ) -> SecurityResult<Option<String>> {
        let next = next_cursor(page)?;
        if let Some(token) = &next
            && !Arc::make_mut(seen).insert(token.clone())
        {
            return Err(ErrorCode::NotReady);
        }
        Ok(next)
    }
    // One response advances the immutable walker. Fallback hydration never issues
    // another RPC here, so a sparse/native-truncated turn cannot monopolize a request.
    fn accept_page(&mut self, page: &Value) -> SecurityResult<()> {
        let mut entries = Vec::new();
        if let Some(HistoryTurn::Items(turn)) = self.turns.get(self.turn_index) {
            let data = page["data"].as_array().ok_or(ErrorCode::NotReady)?;
            if data.len() > 100 {
                return Err(ErrorCode::ResourceLimit);
            }
            for entry in data {
                if native_id(entry, "turnId")? != *turn {
                    return Err(ErrorCode::NotReady);
                }
                entries.push(Self::record(
                    turn.clone(),
                    &entry["item"],
                    entry.get("completedAtMs").is_some_and(|v| !v.is_null()),
                    native_timestamp(&entry["startedAtMs"]),
                )?);
            }
            self.item_cursor = Self::advance_cursor(&mut self.seen_items, page)?;
            if self.item_cursor.is_none() {
                self.turn_index += 1;
                self.seen_items = Arc::new(HashSet::new());
            }
        } else {
            let data = page["data"].as_array().ok_or(ErrorCode::NotReady)?;
            if data.len() > 50 {
                return Err(ErrorCode::ResourceLimit);
            }
            let mut turns = Vec::with_capacity(data.len());
            for turn in data {
                let id = native_id(turn, "id")?;
                if turn.get("itemsView").and_then(Value::as_str) == Some("full") {
                    let records = turn["items"]
                        .as_array()
                        .ok_or(ErrorCode::NotReady)?
                        .iter()
                        .rev()
                        .map(|item| {
                            Self::record(id.clone(), item, turn["status"] != "inProgress", None)
                        })
                        .collect::<SecurityResult<Vec<_>>>()?;
                    turns.push(HistoryTurn::Full(records));
                } else {
                    turns.push(HistoryTurn::Items(id));
                }
            }
            self.cursor = Self::advance_cursor(&mut self.seen_turns, page)?;
            self.turns = Arc::new(turns);
            self.turn_index = 0;
        }
        // Native turns and fallback item pages are both descending. Full turns
        // following a completed fallback can be consumed without another native read.
        if self.item_cursor.is_none() {
            while let Some(HistoryTurn::Full(records)) = self.turns.get(self.turn_index) {
                entries.extend(records.iter().cloned());
                self.turn_index += 1;
            }
        }
        if self.turn_index == self.turns.len() {
            self.turns = Arc::new(Vec::new());
            self.turn_index = 0;
        }
        if let Some(anchor) = &self.anchor {
            if let Some(index) = entries
                .iter()
                .position(|r| native_id(&r.item, "id").as_ref() == Ok(anchor))
            {
                entries.drain(..=index);
                self.anchor = None;
            } else {
                entries.clear();
            }
        }
        self.offset = 0;
        self.remaining = Arc::new(entries);
        if !self.has_more() && self.anchor.is_some() {
            return Err(ErrorCode::CursorExpired);
        }
        if self.storage_bytes(
            &mut HashSet::new(),
            &mut HashSet::new(),
            &mut HashSet::new(),
        ) > 64 * 1024 * 1024
        {
            return Err(ErrorCode::ResourceLimit);
        }
        Ok(())
    }
    fn record(
        turn: Id,
        item: &Value,
        complete: bool,
        timestamp: Option<Uint>,
    ) -> SecurityResult<NativeRecord> {
        native_id(item, "id")?;
        let item = safe_native(item);
        let bytes = native_bytes(&item);
        Ok(NativeRecord {
            turn,
            item,
            order: 0,
            complete,
            timestamp,
            source: RecordSource::NativePage,
            bytes,
            cached: true,
            live_base: false,
            fingerprint: None,
            gap: None,
        })
    }
    fn storage_bytes(
        &self,
        records: &mut HashSet<*const Vec<NativeRecord>>,
        turns: &mut HashSet<*const Vec<HistoryTurn>>,
        cycles: &mut HashSet<*const HashSet<String>>,
    ) -> usize {
        let mut bytes = 256
            + self.cursor.as_ref().map_or(0, String::len)
            + self.item_cursor.as_ref().map_or(0, String::len)
            + self.anchor.as_ref().map_or(0, |id| id.0.len());
        if records.insert(Arc::as_ptr(&self.remaining)) {
            bytes += self
                .remaining
                .iter()
                .map(|r| r.bytes + r.turn.0.len())
                .sum::<usize>();
        }
        if turns.insert(Arc::as_ptr(&self.turns)) {
            bytes += self
                .turns
                .iter()
                .map(|turn| match turn {
                    HistoryTurn::Full(records) => records
                        .iter()
                        .map(|r| r.bytes + r.turn.0.len())
                        .sum::<usize>(),
                    HistoryTurn::Items(id) => id.0.len() + 64,
                })
                .sum::<usize>();
        }
        for seen in [&self.seen_turns, &self.seen_items] {
            if cycles.insert(Arc::as_ptr(seen)) {
                bytes += seen.iter().map(|s| s.len() + 32).sum::<usize>();
            }
        }
        bytes
    }
}
struct History {
    records: HashMap<Id, NativeRecord>,
    #[cfg(test)]
    continuations: HashMap<Id, Continuation>,
    older: Option<String>,
    first: u64,
    next: u64,
    bytes: usize,
}
impl Default for History {
    fn default() -> Self {
        Self {
            records: HashMap::new(),
            #[cfg(test)]
            continuations: HashMap::new(),
            older: None,
            first: UINT_MAX / 2,
            next: UINT_MAX / 2,
            bytes: 0,
        }
    }
}
impl History {
    fn live_item(
        &mut self,
        state: &State,
        target: &Target,
        turn: &Id,
        item: &Value,
        lifecycle: (bool, Option<Uint>),
    ) -> Result<SessionUpdate, ErrorCode> {
        let id = native_id(item, "id")?;
        let order = if let Some(old) = self.records.get(&id) {
            if old.turn != *turn {
                return Err(ErrorCode::StaleSession);
            }
            old.order
        } else {
            let n = self.next;
            self.next = self
                .next
                .checked_add(1)
                .filter(|n| *n <= UINT_MAX)
                .ok_or(ErrorCode::ResourceLimit)?;
            n
        };
        let (complete, time) = lifecycle;
        let time = time.or_else(|| self.records.get(&id).and_then(|r| r.timestamp));
        let projected = project_item_record(
            state,
            target,
            turn,
            &safe_native(item),
            order,
            RecordSource::Live,
            (complete, time),
        )?;
        let update = if complete {
            SessionUpdate::History(vec![SafeHistory::from_redacted_display(projected)?])
        } else {
            SessionUpdate::Activity(item_activity(projected))
        };
        self.retain(
            turn.clone(),
            item.clone(),
            order,
            complete,
            RecordSource::Live,
        );
        if let Some(r) = self.records.get_mut(&id) {
            r.timestamp = time;
        }
        Ok(update)
    }
    fn delta(
        &mut self,
        state: &State,
        target: &Target,
        method: &str,
        p: &Value,
    ) -> Result<SessionUpdate, ErrorCode> {
        let id = native_id(p, "itemId")?;
        let turn = native_id(p, "turnId")?;
        let delta = p["delta"].as_str().ok_or(ErrorCode::NotReady)?;
        let Some(old) = self.records.get_mut(&id) else {
            return Ok(SessionUpdate::Activity(Activity {
                state: ActivityState::Working,
                text: Some(text_content(state, target, delta.to_owned())?),
                tool_record_id: None,
            }));
        };
        if old.turn != turn {
            return Err(ErrorCode::StaleSession);
        }
        if !old.live_base {
            return Ok(SessionUpdate::Activity(Activity {
                state: ActivityState::Working,
                text: Some(text_content(state, target, delta.to_owned())?),
                tool_record_id: Some(id),
            }));
        }
        let old_bytes = old.bytes;
        if old.cached {
            let command_output = method == "item/commandExecution/outputDelta";
            let command_item =
                old.item.get("type").and_then(Value::as_str) == Some("commandExecution");
            let key = if method == "item/agentMessage/delta" {
                "text"
            } else if method.contains("reasoning") {
                "content"
            } else {
                "aggregatedOutput"
            };
            let text = if key == "content" {
                let index = p
                    .get("contentIndex")
                    .or_else(|| p.get("summaryIndex"))
                    .and_then(Value::as_u64)
                    .and_then(|v| usize::try_from(v).ok())
                    .ok_or(ErrorCode::NotReady)?;
                let key = if method.contains("summary") {
                    "summary"
                } else {
                    "content"
                };
                if index >= native_json::NATIVE.members {
                    return Err(ErrorCode::ResourceLimit);
                }
                let map = old.item.as_object_mut().ok_or(ErrorCode::NotReady)?;
                let array = map
                    .entry(key)
                    .or_insert_with(|| Value::Array(Vec::new()))
                    .as_array_mut()
                    .ok_or(ErrorCode::NotReady)?;
                while array.len() <= index {
                    array.push(Value::String(String::new()));
                }
                array.get_mut(index).ok_or(ErrorCode::NotReady)?
            } else {
                let text = old
                    .item
                    .as_object_mut()
                    .ok_or(ErrorCode::NotReady)?
                    .entry(key)
                    .or_insert_with(|| Value::String(String::new()));
                // The stock item/commandExecution schema permits the started item's
                // aggregatedOutput to be null until its first textual output delta.
                if command_output && command_item && text.is_null() {
                    *text = Value::String(String::new());
                }
                text
            };
            let Value::String(text) = text else {
                return Err(ErrorCode::NotReady);
            };
            let length = text
                .len()
                .checked_add(delta.len())
                .ok_or(ErrorCode::ResourceLimit)?;
            if length > CONTENT_MAX as usize {
                old.gap = Some(ContentUnavailable {
                    reason: ContentUnavailableReason::TooLarge,
                    byte_length: Uint::new(length as u64).ok(),
                });
                old.item = item_metadata(&old.item);
                old.cached = false;
            } else {
                text.push_str(delta);
            }
        } else if let Some(gap) = &mut old.gap
            && let Some(length) = gap.byte_length
        {
            gap.byte_length = length
                .0
                .checked_add(delta.len() as u64)
                .and_then(|n| Uint::new(n).ok());
        }
        old.source = RecordSource::Live;
        old.complete = false;
        old.fingerprint = None;
        old.bytes = native_bytes(&old.item);
        let activity = if old.cached {
            item_activity(project_item_record(
                state,
                target,
                &turn,
                &old.item,
                old.order,
                RecordSource::Live,
                (false, old.timestamp),
            )?)
        } else {
            unavailable_activity(id, old)?
        };
        let current_bytes = old.bytes;
        self.bytes = self.bytes.saturating_sub(old_bytes) + current_bytes;
        self.trim();
        Ok(SessionUpdate::Activity(activity))
    }
    fn page(
        &mut self,
        state: &State,
        target: &Target,
        entries: Vec<NativeRecord>,
        next: Option<String>,
        resume_base: bool,
    ) -> SecurityResult<(Vec<SafeHistory>, bool)> {
        let base = self
            .first
            .checked_sub(entries.len() as u64)
            .ok_or(ErrorCode::ResourceLimit)?;
        let skip = entries.len().saturating_sub(1000);
        let mut records = Vec::with_capacity(entries.len().min(1000));
        for (index, r) in entries.into_iter().enumerate().skip(skip) {
            let id = native_id(&r.item, "id")?;
            let order = self
                .records
                .get(&id)
                .map_or(base + index as u64, |r| r.order);
            if r.complete {
                records.push(project_safe_item(
                    state,
                    target,
                    &r.turn,
                    &r.item,
                    order,
                    RecordSource::NativePage,
                    (r.complete, r.timestamp),
                )?);
            }
            // Incomplete native items remain correlatable even though they are not history.
            self.retain(r.turn, r.item, order, r.complete, RecordSource::NativePage);
            if let Some(cached) = self.records.get_mut(&id) {
                cached.live_base |= resume_base;
                cached.timestamp = cached.timestamp.or(r.timestamp);
            }
        }
        self.first = base;
        self.older = next;
        Ok((records, skip > 0 || self.older.is_some()))
    }
    fn retain(&mut self, turn: Id, item: Value, order: u64, complete: bool, source: RecordSource) {
        let Ok(id) = native_id(&item, "id") else {
            return;
        };
        let mut item = safe_native(&item);
        let mut complete = complete;
        let mut source = source;
        let mut live_base = source == RecordSource::Live;
        if let Some(old) = self.records.remove(&id) {
            self.bytes = self.bytes.saturating_sub(old.bytes);
            live_base |= old.live_base;
            if old.source == RecordSource::Live && source != RecordSource::Live {
                if !old.cached {
                    self.bytes += old.bytes;
                    self.records.insert(id, old);
                    return;
                }
                if let (Some(current), Some(incoming)) =
                    (old.item.as_object(), item.as_object_mut())
                {
                    for (key, value) in current {
                        if !value.is_null() {
                            incoming.insert(key.clone(), value.clone());
                        }
                    }
                }
                complete = old.complete;
                source = old.source;
            }
        }
        // Completed display bodies already live in State's finite Content owner. Retain only
        // source/order/fingerprint metadata; reload the exact native item, never a second store.
        let fingerprint = complete.then(|| native_fingerprint(&item));
        let cached = !complete;
        if !cached {
            item = item_metadata(&item);
        }
        let bytes = native_bytes(&item);
        self.bytes += bytes;
        self.records.insert(
            id,
            NativeRecord {
                turn,
                item,
                order,
                complete,
                timestamp: None,
                source,
                bytes,
                cached,
                live_base,
                fingerprint,
                gap: None,
            },
        );
        self.trim();
    }
    fn trim(&mut self) {
        while self.records.len() > 1000 {
            let Some(id) = self
                .records
                .iter()
                .min_by_key(|(_, r)| r.order)
                .map(|(id, _)| id.clone())
            else {
                break;
            };
            if let Some(old) = self.records.remove(&id) {
                self.bytes = self.bytes.saturating_sub(old.bytes);
            }
        }
        while self.bytes > 31 * 1024 * 1024 {
            let Some(old) = self
                .records
                .values_mut()
                .filter(|r| r.cached)
                .min_by_key(|r| r.order)
            else {
                break;
            };
            self.bytes = self.bytes.saturating_sub(old.bytes);
            old.item = item_metadata(&old.item);
            old.cached = false;
            old.bytes = native_bytes(&old.item);
            self.bytes += old.bytes;
            old.gap = Some(ContentUnavailable {
                reason: ContentUnavailableReason::NotCached,
                byte_length: None,
            });
        }
    }
    #[cfg(test)]
    fn continuation(&mut self, value: Continuation) -> SecurityResult<Id> {
        self.continuation_with_budget(value, 64 * 1024 * 1024)
    }
    #[cfg(test)]
    fn continuation_with_budget(
        &mut self,
        value: Continuation,
        budget: usize,
    ) -> SecurityResult<Id> {
        // Retained-page exhaustion can be requested repeatedly while live records append.
        // Reuse the exact captured boundary instead of minting a different opaque cursor
        // whose anchor/order would be resolved against a newer cache.
        if value.remaining.is_empty()
            && value.turns.is_empty()
            && value.seen_turns.is_empty()
            && value.seen_items.is_empty()
            && let Some(key) = self.continuations.iter().find_map(|(key, current)| {
                (current.remaining.is_empty()
                    && current.turns.is_empty()
                    && current.seen_turns.is_empty()
                    && current.seen_items.is_empty()
                    && current.cursor == value.cursor
                    && current.anchor == value.anchor
                    && current.offset == value.offset
                    && current.before == value.before)
                    .then(|| key.clone())
            })
        {
            return Ok(key);
        }
        let mut records = HashSet::new();
        let mut turns = HashSet::new();
        let mut cycles = HashSet::new();
        let incoming = value.storage_bytes(&mut records, &mut turns, &mut cycles);
        if incoming + self.bytes > budget {
            return Err(ErrorCode::ResourceLimit);
        }
        let retained = self
            .continuations
            .values()
            .map(|c| c.storage_bytes(&mut records, &mut turns, &mut cycles))
            .sum::<usize>();
        if self.continuations.len() >= 64 || retained + incoming + self.bytes > budget {
            self.continuations.clear();
        }
        let key = Id(Uuid::random().0);
        self.continuations.insert(key.clone(), value);
        Ok(key)
    }
}

fn supported_contents(p: &Presentation) -> bool {
    let available = |c: &Content| !matches!(c, Content::Unavailable(_));
    match p {
        Presentation::CodexApproval(p) => {
            available(&p.title)
                && available(&p.details)
                && p.choices
                    .iter()
                    .all(|c| c.details.as_ref().is_none_or(available))
        }
        Presentation::CodexQuestions(p) => p
            .questions
            .iter()
            .all(|q| available(&q.prompt) && q.options.iter().all(|o| available(&o.description))),
        Presentation::CodexForm(p) => {
            available(&p.title)
                && p.fields
                    .iter()
                    .all(|f| f.description.as_ref().is_none_or(available))
        }
        Presentation::CodexUrl(p) => available(&p.title),
        _ => true,
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HistoryMode {
    Paginated,
    Legacy,
}
impl HistoryMode {
    fn from_thread(thread: &Value, expected: &Id) -> SecurityResult<Self> {
        if native_id(thread, "id")? != *expected {
            return Err(ErrorCode::StaleSession);
        }
        match thread["historyMode"].as_str() {
            Some("paginated") => Ok(Self::Paginated),
            Some("legacy") => Ok(Self::Legacy),
            _ => Err(ErrorCode::NotReady),
        }
    }
}
#[derive(Clone)]
struct FlatPosition {
    cursor: Option<String>,
    before: u64,
    cursors: HashSet<String>,
    items: HashSet<Id>,
}
impl FlatPosition {
    fn head() -> Self {
        Self {
            cursor: None,
            before: UINT_MAX,
            cursors: HashSet::new(),
            items: HashSet::new(),
        }
    }
    fn request(&self, thread: &Id) -> Value {
        let mut params = json!({"threadId":thread.0,"limit":100,"sortDirection":"desc"});
        if let Some(cursor) = &self.cursor {
            params["cursor"] = json!(cursor);
        }
        params
    }
    fn accept(mut self, page: &Value) -> SecurityResult<(Vec<NativeRecord>, Option<Self>)> {
        let data = page["data"].as_array().ok_or(ErrorCode::NotReady)?;
        if data.len() > 100 {
            return Err(ErrorCode::ResourceLimit);
        }
        let mut records = Vec::new();
        for (offset, entry) in data.iter().enumerate() {
            let turn = native_id(entry, "turnId")?;
            let id = native_id(&entry["item"], "id")?;
            if !self.items.insert(id) {
                return Err(ErrorCode::NotReady);
            }
            let completed = match entry.get("completedAtMs") {
                Some(Value::Null) => false,
                Some(time) if native_timestamp(time).is_some() => true,
                _ => return Err(ErrorCode::NotReady),
            };
            let mut record = Continuation::record(
                turn,
                &entry["item"],
                completed,
                native_timestamp(&entry["startedAtMs"]),
            )?;
            record.order = self
                .before
                .checked_sub(offset as u64)
                .ok_or(ErrorCode::ResourceLimit)?;
            records.push(record);
        }
        self.before = self
            .before
            .checked_sub(data.len() as u64)
            .ok_or(ErrorCode::ResourceLimit)?;
        self.cursor = next_cursor(page)?;
        if let Some(cursor) = &self.cursor
            && !self.cursors.insert(cursor.clone())
        {
            return Err(ErrorCode::NotReady);
        }
        let more = self.cursor.is_some();
        Ok((records, more.then_some(self)))
    }
}
#[derive(Clone)]
enum BrowseSource {
    Flat(FlatPosition),
    Legacy(Continuation),
}
impl BrowseSource {
    fn position(self) -> SecurityResult<crate::state::BrowsePosition> {
        let bytes = match &self {
            Self::Flat(p) => {
                256 + p.cursors.iter().map(|s| s.len() + 32).sum::<usize>()
                    + p.items.iter().map(|s| s.0.len() + 32).sum::<usize>()
            }
            Self::Legacy(p) => p.storage_bytes(
                &mut HashSet::new(),
                &mut HashSet::new(),
                &mut HashSet::new(),
            ),
        };
        crate::state::BrowsePosition::new(self, bytes)
    }
}
pub(crate) struct SkillsReadContext {
    original: Arc<Connection>,
    cwd: String,
    generation: Uuid,
}

struct Connection {
    target: Target,
    epoch: Uuid,
    key: StableSessionKey,
    owner: ProcessEvidence,
    owner_config: CodexOwner,
    manifest: Manifest,
    path: std::path::PathBuf,
    presence: Presence,
    source: Source,
    agent: Agent,
    link: Arc<Link>,
    dialogs: Mutex<Vec<Dialog>>,
    resolved: Mutex<Vec<NativeRpcId>>,
    history: Mutex<History>,
    history_mode: Mutex<Option<HistoryMode>>,
    observation: Mutex<()>,
    invalidation_reporter: Option<herdr::CodexInvalidationReporter>,
}
fn presence_observation(
    state: &State,
    target: &Target,
    epoch: &Uuid,
    observed: Result<codex_relay::ValidatedPresence, codex_relay::PresenceFailure>,
) -> Result<Option<codex_relay::ValidatedPresence>, ErrorCode> {
    match observed {
        Ok(presence) => Ok(Some(presence)),
        Err(failure) if presence_publication_pending(&failure) => {
            state.observation_pending(target, epoch)?;
            Ok(None)
        }
        Err(_) => Err(ErrorCode::NotReady),
    }
}
fn strict_unpinned_presence(
    observed: codex_relay::ValidatedPresence,
) -> Result<codex_relay::ValidatedPresence, ErrorCode> {
    if matches!(
        &observed,
        codex_relay::ValidatedPresence::Past(_) | codex_relay::ValidatedPresence::Unavailable(_)
    ) {
        Err(ErrorCode::NotReady)
    } else {
        Ok(observed)
    }
}
fn unavailable_resume_rejected(observed: &codex_relay::ValidatedPresence, thread: &Id) -> bool {
    matches!(
        observed,
        codex_relay::ValidatedPresence::Unavailable(presence)
            if presence.resume_rejected_thread.as_ref() == Some(thread)
    )
}
// Keep the established comparison order while retaining its first failed predicate for capture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OriginalPresenceScopeFailure {
    State,
    Binding,
    OwnerEpoch,
    Relay,
    ConnectionEpoch,
    Tui,
    Associated,
    Resumed,
}
impl OriginalPresenceScopeFailure {
    fn guard(self) -> &'static str {
        match self {
            Self::State => "original_presence_state",
            Self::Binding => "original_presence_binding",
            Self::OwnerEpoch => "original_presence_owner_epoch",
            Self::Relay => "original_presence_relay",
            Self::ConnectionEpoch => "original_presence_connection_epoch",
            Self::Tui => "original_presence_tui",
            Self::Associated => "original_presence_associated",
            Self::Resumed => "original_presence_resumed",
        }
    }
}
fn original_presence_state_error_guard(error: Option<&str>) -> &'static str {
    match error {
        Some(
            "Relay identity, transport or bounded admission failed; explicit reconciliation required",
        ) => "original_presence_state_relay_bounded_admission_failed",
        Some(
            "Stock TUI rejected exact thread/resume; zero-turn rollout may be unmaterialized. No warm-up or substitute was attempted",
        ) => "original_presence_state_stock_tui_resume_rejected",
        Some("TUI transport read failed") => "original_presence_state_tui_read",
        Some("TUI data frame invalid") => "original_presence_state_tui_frame",
        Some("TUI observation failed") => "original_presence_state_tui_observation",
        Some("Owner transport read failed") => "original_presence_state_owner_read",
        Some("Owner data frame invalid") => "original_presence_state_owner_frame",
        Some("Owner observation failed") => "original_presence_state_owner_observation",
        Some("Owner/manifest source proof failed") => "original_presence_state_manifest_proof",
        Some("Relay publication source changed") => "original_presence_state_publication_changed",
        Some("Native observation publication failed") => {
            "original_presence_state_observation_publish"
        }
        Some("Relay presence publication failed") => "original_presence_state_presence_publish",
        Some("Owner-bound relay queue failed") => "original_presence_state_owner_queue",
        Some("TUI-bound relay queue failed") => "original_presence_state_tui_queue",
        Some("Owner write timed out") => "original_presence_state_owner_write_timeout",
        Some("Owner write failed") => "original_presence_state_owner_write",
        Some("TUI write timed out") => "original_presence_state_tui_write_timeout",
        Some("TUI write failed") => "original_presence_state_tui_write",
        Some("Herdr pane.process_info rejected: pane_not_found") => {
            "original_presence_state_pane_not_found"
        }
        Some("Herdr terminal source proof failed") => "original_presence_state_terminal_proof",
        Some("Bound terminal closed") => "original_presence_state_terminal_closed",
        Some("Relay workers absent") => "original_presence_state_relay_workers_absent",
        Some("Relay worker failed") => "original_presence_state_relay_worker_failed",
        Some("Native relay forwarding closed") => "original_presence_state_native_relay_closed",
        _ => "original_presence_state",
    }
}
fn original_presence_state_guard(
    observed: Option<&codex_relay::ValidatedPresence>,
) -> &'static str {
    let error = observed.and_then(|observed| match observed {
        codex_relay::ValidatedPresence::Fresh(presence)
        | codex_relay::ValidatedPresence::Past(presence)
        | codex_relay::ValidatedPresence::Unavailable(presence) => {
            presence.error.as_ref().map(|error| error.0.as_str())
        }
    });
    original_presence_state_error_guard(error)
}
fn original_presence_terminal_capture(
    observed: Option<&codex_relay::ValidatedPresence>,
) -> Option<codex_relay::TerminalCapture> {
    let error = observed.and_then(|observed| match observed {
        codex_relay::ValidatedPresence::Fresh(presence)
        | codex_relay::ValidatedPresence::Past(presence)
        | codex_relay::ValidatedPresence::Unavailable(presence) => {
            presence.error.as_ref().map(|error| error.0.as_str())
        }
    });
    error.and_then(codex_relay::terminal_capture_annotation)
}
fn original_presence_terminal_capture_guard(capture: codex_relay::TerminalCapture) -> &'static str {
    match (capture.phase, capture.response_timeout) {
        ("snapshot", true) => "original_presence_state_terminal_snapshot_timeout",
        ("process_info", true) => "original_presence_state_terminal_process_info_timeout",
        ("connect", false) => "original_presence_state_terminal_connect",
        ("snapshot_source", false) => "original_presence_state_terminal_snapshot_source",
        ("snapshot", false) => "original_presence_state_terminal_snapshot",
        ("reporter_process", false) => "original_presence_state_terminal_reporter_process",
        ("process_info_connect", false) => "original_presence_state_terminal_process_info_connect",
        ("process_info_source", false) => "original_presence_state_terminal_process_info_source",
        ("process_info", false) => "original_presence_state_terminal_process_info",
        ("tui_distribution", false) => "original_presence_state_terminal_tui_distribution",
        ("tui_argv", false) => "original_presence_state_terminal_tui_argv",
        ("tui_foreground", false) => "original_presence_state_terminal_tui_foreground",
        ("tui_write_unpolled_not_pending", false) => {
            "original_presence_state_terminal_tui_write_unpolled_not_pending"
        }
        ("tui_write_ready_not_pending", false) => {
            "original_presence_state_terminal_tui_write_ready_not_pending"
        }
        ("tui_write_ready_pending", false) => {
            "original_presence_state_terminal_tui_write_ready_pending"
        }
        ("tui_write_start_send_not_pending", false) => {
            "original_presence_state_terminal_tui_write_start_send_not_pending"
        }
        ("tui_write_flush_not_pending", false) => {
            "original_presence_state_terminal_tui_write_flush_not_pending"
        }
        ("tui_write_flush_pending", false) => {
            "original_presence_state_terminal_tui_write_flush_pending"
        }
        _ => "original_presence_state",
    }
}
fn original_presence_scope(
    presence: &Presence,
    original: &Presence,
    native_thread: &Id,
) -> Result<(), OriginalPresenceScopeFailure> {
    if presence.state != codex_relay::PresenceState::Connected {
        return Err(OriginalPresenceScopeFailure::State);
    }
    if presence.binding_id != original.binding_id {
        return Err(OriginalPresenceScopeFailure::Binding);
    }
    if presence.owner_epoch != original.owner_epoch {
        return Err(OriginalPresenceScopeFailure::OwnerEpoch);
    }
    if presence.relay != original.relay {
        return Err(OriginalPresenceScopeFailure::Relay);
    }
    if presence.connection_epoch != original.connection_epoch {
        return Err(OriginalPresenceScopeFailure::ConnectionEpoch);
    }
    if presence.tui != original.tui {
        return Err(OriginalPresenceScopeFailure::Tui);
    }
    if !presence.associated_thread_ids.contains(native_thread) {
        return Err(OriginalPresenceScopeFailure::Associated);
    }
    if !presence.resumed_thread_ids.contains(native_thread) {
        return Err(OriginalPresenceScopeFailure::Resumed);
    }
    Ok(())
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OriginalPresenceDecision {
    Unavailable(u64),
    Fresh(u64),
    Past(u64),
}
impl OriginalPresenceDecision {
    fn is_fresh(self) -> bool {
        matches!(self, Self::Fresh(_))
    }
    fn final_freshness(self, now: u64) -> Result<bool, codex_relay::PresenceFailure> {
        match self {
            Self::Unavailable(updated_at) => {
                codex_relay::freshness_at_completion(updated_at, now)?;
                Ok(false)
            }
            Self::Fresh(updated_at) => codex_relay::freshness_at_completion(updated_at, now),
            Self::Past(updated_at) => {
                // A clock rollback remains terminal, while an initially past record
                // cannot acquire authority in this observation.
                codex_relay::freshness_at_completion(updated_at, now)?;
                Ok(false)
            }
        }
    }
}
enum OriginalPresenceDecisionFailure {
    Scope(OriginalPresenceScopeFailure),
    Code(ErrorCode),
}
fn original_presence_decision_checked(
    state: &State,
    target: &Target,
    epoch: &Uuid,
    observed: Option<&codex_relay::ValidatedPresence>,
    original: &Presence,
    native_thread: &Id,
) -> Result<OriginalPresenceDecision, OriginalPresenceDecisionFailure> {
    let Some(observed) = observed else {
        return Ok(OriginalPresenceDecision::Unavailable(0));
    };
    let (presence, availability) = match observed {
        codex_relay::ValidatedPresence::Fresh(presence) => (presence, Some(true)),
        codex_relay::ValidatedPresence::Past(presence) => (presence, Some(false)),
        codex_relay::ValidatedPresence::Unavailable(presence) => (presence, None),
    };
    original_presence_scope(presence, original, native_thread)
        .map_err(OriginalPresenceDecisionFailure::Scope)?;
    match availability {
        Some(true) => Ok(OriginalPresenceDecision::Fresh(presence.updated_at.0)),
        Some(false) => {
            state
                .observation_pending(target, epoch)
                .map_err(OriginalPresenceDecisionFailure::Code)?;
            Ok(OriginalPresenceDecision::Past(presence.updated_at.0))
        }
        None => {
            state
                .observation_pending(target, epoch)
                .map_err(OriginalPresenceDecisionFailure::Code)?;
            Ok(OriginalPresenceDecision::Unavailable(presence.updated_at.0))
        }
    }
}
fn report_launch_pending(
    reporter: &Option<herdr::CodexInvalidationReporter>,
    stopping: bool,
    pending: &str,
) {
    let guard = match (stopping, pending) {
        (false, "source_unavailable") => "launch_deadline_source_unavailable",
        (false, "publication_pending") => "launch_deadline_publication_pending",
        (false, "presence_past") => "launch_deadline_presence_past",
        (false, "presence_unavailable") => "launch_deadline_presence_unavailable",
        (false, "relay_not_connected") => "launch_deadline_relay_not_connected",
        (false, "resume_association_missing") => "launch_deadline_resume_association_missing",
        (false, "terminal_proof_false") => "launch_deadline_terminal_proof_false",
        (false, "loaded_membership_false") => "launch_deadline_loaded_membership_false",
        (false, "connection_absent") => "launch_deadline_connection_absent",
        (false, "connection_readiness_pending") => "launch_deadline_connection_readiness_pending",
        (false, "created_binding_pending") => "launch_deadline_created_binding_pending",
        (false, "source_recheck_pending") => "launch_deadline_source_recheck_pending",
        (false, "drain") => "launch_deadline_drain",
        (false, _) => "launch_deadline_unobserved",
        (true, "source_unavailable") => "launch_stopping_source_unavailable",
        (true, "publication_pending") => "launch_stopping_publication_pending",
        (true, "presence_past") => "launch_stopping_presence_past",
        (true, "presence_unavailable") => "launch_stopping_presence_unavailable",
        (true, "relay_not_connected") => "launch_stopping_relay_not_connected",
        (true, "resume_association_missing") => "launch_stopping_resume_association_missing",
        (true, "terminal_proof_false") => "launch_stopping_terminal_proof_false",
        (true, "loaded_membership_false") => "launch_stopping_loaded_membership_false",
        (true, "connection_absent") => "launch_stopping_connection_absent",
        (true, "connection_readiness_pending") => "launch_stopping_connection_readiness_pending",
        (true, "created_binding_pending") => "launch_stopping_created_binding_pending",
        (true, "source_recheck_pending") => "launch_stopping_source_recheck_pending",
        (true, "drain") => "launch_stopping_drain",
        (true, _) => "launch_stopping_unobserved",
    };
    if let Some(reporter) = reporter {
        reporter.prebinding("creation", guard, ErrorCode::Timeout, "none");
    }
}

#[cfg(test)]
fn original_presence_decision(
    state: &State,
    target: &Target,
    epoch: &Uuid,
    observed: Option<&codex_relay::ValidatedPresence>,
    original: &Presence,
    native_thread: &Id,
) -> Result<OriginalPresenceDecision, ErrorCode> {
    original_presence_decision_checked(state, target, epoch, observed, original, native_thread)
        .map_err(|error| match error {
            OriginalPresenceDecisionFailure::Scope(_) => ErrorCode::StaleSession,
            OriginalPresenceDecisionFailure::Code(code) => code,
        })
}
fn presence_publication_pending(failure: &codex_relay::PresenceFailure) -> bool {
    failure.phase == codex_relay::PresencePhase::PresenceRecord
        && codex_relay::publication_unstable(&failure.error)
}
#[cfg(test)]
fn revalidate_observation<T>(
    state: &State,
    target: &Target,
    epoch: &Uuid,
    serial: &Mutex<()>,
    physical: impl FnOnce() -> Result<bool, ErrorCode>,
    topology: impl FnOnce() -> Result<Option<T>, ErrorCode>,
) -> Result<Option<T>, ErrorCode> {
    revalidate_observation_with_final(state, target, epoch, serial, physical, topology, || {
        Ok(true)
    })
}
fn revalidate_observation_with_final<T>(
    state: &State,
    target: &Target,
    epoch: &Uuid,
    serial: &Mutex<()>,
    physical: impl FnOnce() -> Result<bool, ErrorCode>,
    topology: impl FnOnce() -> Result<Option<T>, ErrorCode>,
    final_freshness: impl FnOnce() -> Result<bool, ErrorCode>,
) -> Result<Option<T>, ErrorCode> {
    // The observation and its gate update form one synchronous operation. An older
    // success cannot race a later unavailable observation and clear its gate.
    let _observation = serial.lock();
    let observed = physical()?;
    let source = topology()?;
    if !final_freshness()? {
        if observed {
            state.observation_pending(target, epoch)?;
        }
        return Ok(None);
    }
    if observed && source.is_some() {
        state.observation_revalidated(target, epoch)?;
        Ok(source)
    } else {
        Ok(None)
    }
}
impl Connection {
    fn report_invalidation(&self, phase: &'static str, guard: &'static str, code: ErrorCode) {
        if let Some(reporter) = &self.invalidation_reporter {
            reporter.target(&self.target, &self.epoch, phase, guard, code, "none");
        }
    }
    fn report_invalidation_closed(
        &self,
        phase: &'static str,
        guard: &'static str,
        code: ErrorCode,
        detail: (&'static str, i32),
    ) {
        if let Some(reporter) = &self.invalidation_reporter {
            reporter.target_closed(&self.target, &self.epoch, phase, guard, code, detail);
        }
    }
    fn physical(&self, state: &State) -> Result<(), ErrorCode> {
        let _observation = self.observation.lock();
        let observed = match self.observe_physical(state) {
            Ok(observed) => observed,
            Err(code) => {
                self.report_invalidation("connection", "physical_or_presence", code);
                return Err(code);
            }
        };
        if !observed
            .final_freshness(pairing::now_ms())
            .map_err(|f| policy(f.error))?
        {
            if observed.is_fresh() {
                state.observation_pending(&self.target, &self.epoch)?;
            }
            self.report_invalidation("connection", "presence_freshness", ErrorCode::NotReady);
            return Err(ErrorCode::NotReady);
        }
        let result = state.observation_ready(&self.target, &self.epoch);
        if let Err(code) = &result {
            self.report_invalidation("connection", "physical_state", *code);
        }
        result
    }
    /// Called with observation held through any subsequent readiness/gate update.
    fn observe_physical(&self, state: &State) -> Result<OriginalPresenceDecision, ErrorCode> {
        state
            .validate_source(&self.target, &self.epoch)
            .inspect_err(|&code| {
                self.report_invalidation("connection", "state_before_presence", code);
            })?;
        let manifest = Manifest::load(&self.path)
            .map_err(policy)
            .inspect_err(|&code| {
                self.report_invalidation("connection", "manifest_read", code);
            })?;
        let presence = presence_observation(
            state,
            &self.target,
            &self.epoch,
            manifest.validated_presence(&self.path),
        )
        .inspect_err(|&code| {
            self.report_invalidation("connection", "presence_observation", code);
        })?;
        self.observe_verified_presence(state, &manifest, presence.as_ref())
    }
    /// The caller holds observation, including the pending decision for this source.
    fn observe_verified_presence(
        &self,
        state: &State,
        manifest: &Manifest,
        presence: Option<&codex_relay::ValidatedPresence>,
    ) -> Result<OriginalPresenceDecision, ErrorCode> {
        state
            .validate_source(&self.target, &self.epoch)
            .inspect_err(|&code| {
                self.report_invalidation("connection", "state_after_presence", code);
            })?;
        if serde_json::to_vec(&manifest)
            .map_err(policy)
            .inspect_err(|&code| {
                self.report_invalidation("connection", "manifest_encode", code);
            })?
            != serde_json::to_vec(&self.manifest)
                .map_err(policy)
                .inspect_err(|&code| {
                    self.report_invalidation("connection", "manifest_encode", code);
                })?
        {
            self.report_invalidation("connection", "manifest_changed", ErrorCode::StaleSession);
            return Err(ErrorCode::StaleSession);
        }
        if manifest
            .owner_epoch
            .validate(
                &self.owner_config.socket_host,
                &self.owner_config.socket_agent,
            )
            .map_err(policy)
            .inspect_err(|&code| {
                self.report_invalidation("connection", "owner_epoch", code);
            })?
            != self.owner
        {
            self.report_invalidation("connection", "owner_changed", ErrorCode::StaleSession);
            return Err(ErrorCode::StaleSession);
        }
        self.source
            .recheck_identity()
            .map_err(policy)
            .inspect_err(|&code| {
                self.report_invalidation("connection", "source_identity", code);
            })?;
        match original_presence_decision_checked(
            state,
            &self.target,
            &self.epoch,
            presence,
            &self.presence,
            &self.key.native_conversation_id,
        ) {
            Ok(observed) => Ok(observed),
            Err(OriginalPresenceDecisionFailure::Scope(failure)) => {
                if failure == OriginalPresenceScopeFailure::State
                    && let Some(capture) = original_presence_terminal_capture(presence)
                {
                    self.report_invalidation_closed(
                        "connection",
                        original_presence_terminal_capture_guard(capture),
                        ErrorCode::StaleSession,
                        (capture.io, capture.errno),
                    );
                } else {
                    let guard = if failure == OriginalPresenceScopeFailure::State {
                        original_presence_state_guard(presence)
                    } else {
                        failure.guard()
                    };
                    self.report_invalidation("connection", guard, ErrorCode::StaleSession);
                }
                Err(ErrorCode::StaleSession)
            }
            Err(OriginalPresenceDecisionFailure::Code(code)) => {
                self.report_invalidation("connection", "observation_pending", code);
                Err(code)
            }
        }
    }
    fn validate(&self, state: &State, topology: &Topology) -> Result<(), ErrorCode> {
        self.readiness(state, topology)?
            .ok_or(ErrorCode::NotReady)
            .map(|_| ())
    }
    fn readiness(&self, state: &State, topology: &Topology) -> Result<Option<Source>, ErrorCode> {
        let observed_at = std::cell::Cell::new(OriginalPresenceDecision::Unavailable(0));
        let result = revalidate_observation_with_final(
            state,
            &self.target,
            &self.epoch,
            &self.observation,
            || {
                let observed = self.observe_physical(state).inspect_err(|&code| {
                    self.report_invalidation("connection", "physical_or_presence", code);
                })?;
                observed_at.set(observed);
                Ok(observed.is_fresh())
            },
            || {
                let Some(source) =
                    topology
                        .source_readiness(&self.key.server_id)
                        .inspect_err(|&code| {
                            self.report_invalidation("connection", "topology_source", code);
                        })?
                else {
                    return Ok(None);
                };
                if source.incarnation != self.key.server_incarnation
                    || source.evidence != self.source.evidence
                {
                    self.report_invalidation(
                        "connection",
                        "topology_source",
                        ErrorCode::StaleSession,
                    );
                    return Err(ErrorCode::StaleSession);
                }
                let agent = source
                    .snapshot
                    .agents
                    .iter()
                    .find(|a| a.terminal_id == self.key.terminal_id)
                    .ok_or_else(|| {
                        self.report_invalidation(
                            "connection",
                            "terminal_absent",
                            ErrorCode::StaleSession,
                        );
                        ErrorCode::StaleSession
                    })?;
                if agent.agent != self.agent.agent
                    || agent.agent_session != self.agent.agent_session
                {
                    if let Some(reporter) = &self.invalidation_reporter {
                        reporter.native_report(
                            &self.target,
                            &self.epoch,
                            "connection",
                            &Some((self.agent.agent.clone(), self.agent.agent_session.clone())),
                            &Some((agent.agent.clone(), agent.agent_session.clone())),
                        );
                    }
                    return Err(ErrorCode::StaleSession);
                }
                let topology_ready = source.recheck_readiness().map_err(|error| {
                    let code = policy(error);
                    self.report_invalidation("connection", "effective_topology_loss", code);
                    code
                })?;
                if topology_ready {
                    Ok(Some(source))
                } else {
                    Ok(None)
                }
            },
            || {
                observed_at
                    .get()
                    .final_freshness(pairing::now_ms())
                    .map_err(|failure| policy(failure.error))
            },
        );
        if let Err(code) = &result {
            self.report_invalidation("connection", "readiness", *code);
        }
        result
    }
}
pub struct Runtime {
    topology: Arc<Topology>,
    security: Arc<Security>,
    connections: Mutex<HashMap<Uuid, Arc<Connection>>>,
    jobs: Mutex<JoinSet<()>>,
    stopping: AtomicBool,
    thread_stop: watch::Sender<bool>,
    invalidation_reporter: Option<herdr::CodexInvalidationReporter>,
    owner_start: tokio::sync::Mutex<()>,
}
impl Runtime {
    pub fn new(topology: Arc<Topology>, security: Arc<Security>) -> Arc<Self> {
        Self::new_with_invalidation_reporter(topology, security, None)
    }
    pub(crate) fn new_with_invalidation_reporter(
        topology: Arc<Topology>,
        security: Arc<Security>,
        invalidation_reporter: Option<herdr::CodexInvalidationReporter>,
    ) -> Arc<Self> {
        if let Some(reporter) = &invalidation_reporter {
            topology.set_codex_invalidation_reporter(reporter.clone());
        }
        let (thread_stop, _) = watch::channel(false);
        Arc::new(Self {
            topology,
            security,
            connections: Mutex::new(HashMap::new()),
            jobs: Mutex::new(JoinSet::new()),
            stopping: AtomicBool::new(false),
            thread_stop,
            invalidation_reporter,
            owner_start: tokio::sync::Mutex::new(()),
        })
    }
    pub fn backend(&self) -> Backend {
        let ready = self
            .connections
            .lock()
            .values()
            .any(|c| c.validate(&self.security.state, &self.topology).is_ok());
        let reason = self
            .topology
            .config
            .backend_reason(Provider::Codex)
            .ok()
            .flatten()
            .or_else(|| self.creation_unavailable());
        Backend{provider:Provider::Codex,version:"Stock Codex0.162.1 native contracts".into(),availability:if ready{Availability::Ready}else if self.topology.config.creation_codex_owner().is_err(){Availability::SetupRequired}else if reason.is_some(){Availability::Unavailable}else{Availability::SetupRequired},reason:Some(reason.unwrap_or(if ready{"Bound launch thread; desktop selection unknown; stock loaded/resume checks are not atomic"}else{"Shared mode required: independent owned app-server, publisher provenance and exact TUI/Herdr/kernel/socket association proof"}).into())}
    }
    fn connection(&self, target: &Target) -> SecurityResult<Arc<Connection>> {
        let c = self
            .connections
            .lock()
            .get(&target.session_id)
            .cloned()
            .ok_or(ErrorCode::NotReady)?;
        if c.target != *target {
            return Err(ErrorCode::StaleSession);
        }
        c.validate(&self.security.state, &self.topology)?;
        Ok(c)
    }
    pub(crate) fn artifact_context(self:&Arc<Self>,target:&Target)->SecurityResult<herdr::artifacts::ArtifactContext>{
        let original=self.connection(target)?;let process=original.presence.tui.clone().ok_or(ErrorCode::NotReady)?;let runtime=Arc::downgrade(self);let retained=original.clone();
        let guard=Arc::new(move|validate:&mut dyn FnMut()->SecurityResult<()>|{let runtime=runtime.upgrade().ok_or(ErrorCode::NotReady)?;retained.validate(&runtime.security.state,&runtime.topology)?;let connections=runtime.connections.lock();if !connections.get(&retained.target.session_id).is_some_and(|current|Arc::ptr_eq(current,&retained)){return Err(ErrorCode::StaleSession)}validate()});
        self.topology.agent_artifact_context(target,process,Provider::Codex,guard)
    }
    pub(crate) fn agent_terminal_context(self:&Arc<Self>,target:&Target)->SecurityResult<herdr::AgentTerminalContext> {
        let original=self.connection(target)?;let process=original.presence.tui.clone().ok_or(ErrorCode::NotReady)?;
        let runtime=self.clone();let retained=original.clone();
        let guard=Arc::new(move|ack:bool,validate:&mut dyn FnMut()->SecurityResult<()>|{
            if ack {if !retained.observe_physical(&runtime.security.state)?.is_fresh(){return Err(ErrorCode::NotReady)}}else{retained.validate(&runtime.security.state,&runtime.topology)?;}
            let connections=runtime.connections.lock();
            if !connections.get(&retained.target.session_id).is_some_and(|current|Arc::ptr_eq(current,&retained)){return Err(ErrorCode::StaleSession)}
            validate()
        });
        self.topology.agent_terminal_context(target,process,Provider::Codex,guard)
    }
    pub(crate) fn attachment_context(self:&Arc<Self>,target:&Target)->SecurityResult<crate::attachments::ProviderContext> {
        let original=self.connection(target)?;let runtime=self.clone();let retained=original.clone();
        let validate=Arc::new(move||{let current=runtime.connections.lock().get(&retained.target.session_id).cloned().ok_or(ErrorCode::NotReady)?;if !Arc::ptr_eq(&current,&retained){return Err(ErrorCode::StaleSession)}retained.validate(&runtime.security.state,&runtime.topology)});
        let runtime=self.clone();let retained=original.clone();
        let publish=Arc::new(move|enqueue:&mut dyn FnMut()->SecurityResult<()>|{
            let connections=runtime.connections.lock();
            if !connections.get(&retained.target.session_id).is_some_and(|current|Arc::ptr_eq(current,&retained)){return Err(ErrorCode::StaleSession)}
            retained.validate(&runtime.security.state,&runtime.topology)?;
            enqueue()
        });
        crate::attachments::ProviderContext::capture(&self.topology.config,original.target.clone(),original.epoch.clone(),original.source.clone(),original.owner.clone(),validate,publish)
    }
    fn read_connection(&self, target: &Target) -> Result<Arc<Connection>, ErrorCode> {
        let c = self
            .connections
            .lock()
            .get(&target.session_id)
            .cloned()
            .ok_or(ErrorCode::NotReady)?;
        if c.target != *target {
            return Err(ErrorCode::StaleSession);
        }
        // Retained snapshots remain readable while either exact-source observation gate
        // is pending. This single check never retries a private publication read.
        c.readiness(&self.security.state, &self.topology)?;
        Ok(c)
    }
    fn creation_connection_ready(
        &self,
        connection: &Arc<Connection>,
    ) -> SecurityResult<Option<Source>> {
        if !self
            .connections
            .lock()
            .get(&connection.target.session_id)
            .is_some_and(|current| Arc::ptr_eq(current, connection))
        {
            return Err(ErrorCode::StaleSession);
        }
        let source = connection.readiness(&self.security.state, &self.topology)?;
        match self
            .security
            .state
            .session_state(&connection.target, &connection.epoch)?
            .summary
            .connectivity
        {
            SessionConnectivity::Ready => Ok(source),
            SessionConnectivity::Synchronizing => Ok(None),
            _ => Err(ErrorCode::NotReady),
        }
    }
    async fn native_history(
        &self,
        c: &Connection,
        method: &str,
        params: Value,
    ) -> Result<Value, ErrorCode> {
        tokio::time::timeout(Duration::from_secs(14), async {
            let mut budget = ReadBudget::default();
            c.physical(&self.security.state)?;
            if !c
                .link
                .loaded_budgeted(&c.key.native_conversation_id, &mut budget)
                .await?
            {
                return Err(ErrorCode::NotReady);
            }
            let value = budget.rpc(&c.link, method, params).await?;
            if !c
                .link
                .loaded_budgeted(&c.key.native_conversation_id, &mut budget)
                .await?
            {
                return Err(ErrorCode::NotReady);
            }
            c.physical(&self.security.state)?;
            Ok(value)
        })
        .await
        .map_err(|_| ErrorCode::Timeout)?
    }
    pub fn validate(&self, request: &Request) -> SecurityResult<()> {
        if self.stopping.load(Ordering::Acquire) {
            return Err(ErrorCode::NotReady);
        }
        if let RequestBody::SessionCreate(p) = &request.body {
            if p.provider != Provider::Codex {
                return Err(ErrorCode::NotReady);
            }
            self.topology.validate_create(request)?;
            return self.creation_available();
        }
        self.security.state.validate_session_mutation(request)?;
        let target = match &request.body {
            RequestBody::PromptSubmit(p) => &p.target,
            RequestBody::TurnCancel(p) => &p.target,
            RequestBody::RequestRespond(p) => &p.target,
            _ => return Err(ErrorCode::InvalidRequest),
        };
        let c = self.connection(target)?;
        if let RequestBody::RequestRespond(p) = &request.body {
            let dialogs = c.dialogs.lock();
            let dialog = dialogs
                .iter()
                .find(|d| d.pending.request_key == p.request_key && !d.sent)
                .ok_or(ErrorCode::AlreadyResolved)?;
            validate_answer(&dialog.pending.presentation, &p.answer)
                .map_err(|_| ErrorCode::InvalidAnswer)?;
        }
        Ok(())
    }
    pub async fn run(self: Arc<Self>, mut stop: watch::Receiver<bool>) -> io::Result<()> {
        let reason = self.creation_unavailable();
        let _ = self.topology.codex_availability(reason);
        let mut followers: HashMap<(Uuid, Id), tokio::task::JoinHandle<()>> = HashMap::new();
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        while !*stop.borrow() {
            tokio::select! {_=stop.changed()=>{},_=tick.tick()=>{
                let done:Vec<_>=followers.iter().filter(|(_,j)|j.is_finished()).map(|(k,_)|k.clone()).collect();for key in done{if let Some(job)=followers.remove(&key){let _=job.await;}}
                {let mut jobs=self.jobs.lock();while jobs.try_join_next().is_some(){}}
                for source in self.topology.sources(){for agent in &source.snapshot.agents{if agent.agent.as_deref()!=Some("codex"){continue}let key=(source.server_id.clone(),agent.terminal_id.clone());if followers.contains_key(&key)||followers.len()>=512{continue}let runtime=self.clone();let source=source.clone();let agent=agent.clone();let rx=stop.clone();followers.insert(key,tokio::spawn(async move{if runtime.clone().follow(source.clone(),agent.clone(),rx).await.is_err(){let _=runtime.topology.unbound_reason(&source,&agent.terminal_id,Provider::Codex,UnboundState::SetupRequired,"Shared mode required: no current exact owner/TUI/thread/source binding; no cold fallback");}}));}}
            }}
        }
        self.stopping.store(true, Ordering::Release);
        self.thread_stop.send_replace(true);
        let _ = self
            .topology
            .codex_availability(Some("Codex creation unavailable: companion is stopping"));
        for connection in self.connections.lock().values() {
            connection.link.shutdown();
        }
        for (_, job) in followers {
            let _ = job.await;
        }
        let barrier = match self.security.storage.admission().await {
            Ok(a) => a.call(|_| Ok(())).await,
            Err(e) => Err(e),
        };
        let mut jobs = {
            let mut owned = self.jobs.lock();
            std::mem::replace(&mut *owned, JoinSet::new())
        };
        while jobs.join_next().await.is_some() {}
        barrier.map_err(|_| invalid())?;
        Ok(())
    }
    async fn binding(
        &self,
        source: &Source,
        agent: &Agent,
    ) -> SecurityResult<(CodexOwner, std::path::PathBuf, Manifest, Presence)> {
        let mut found = None;
        for owner in &self.topology.config.codex_owners {
            let entries = match std::fs::read_dir(&owner.bindings_dir) {
                Ok(entries) => entries,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(_) => return Err(ErrorCode::NotReady),
            };
            let mut count = 0;
            for entry in entries {
                let path = entry.map_err(policy)?.path();
                if path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                count += 1;
                if count > 512 {
                    return Err(ErrorCode::ResourceLimit);
                }
                let Ok(m) = Manifest::load(&path) else {
                    continue;
                };
                if m.owner_id != owner.id
                    || m.upstream_host != owner.socket_host
                    || m.upstream_agent != owner.socket_agent
                    || m.herdr.server_id != source.server_id
                    || m.herdr.incarnation != source.incarnation
                    || m.herdr.evidence != source.evidence
                    || m.herdr.terminal_id != agent.terminal_id
                {
                    continue;
                }
                let Ok(p) = m.presence(&path) else { continue };
                if p.state != codex_relay::PresenceState::Connected
                    || !p.resumed_thread_ids.contains(&m.expected_thread_id)
                    || !p.associated_thread_ids.contains(&m.expected_thread_id)
                {
                    continue;
                }
                if !m.terminal(p.tui.as_ref()).await.map_err(policy)? {
                    continue;
                }
                if found.is_some() {
                    return Err(ErrorCode::NotReady);
                }
                found = Some((owner.clone(), path, m, p));
            }
        }
        found.ok_or(ErrorCode::NotReady)
    }
    async fn follow(
        self: Arc<Self>,
        source: Source,
        agent: Agent,
        mut stop: watch::Receiver<bool>,
    ) -> SecurityResult<()> {
        let setup: Result<_, ErrorCode> = async {
            let (owner, path, manifest, presence) = self
                .binding(&source, &agent)
                .await
                ?;
            self.prove_owner(&owner, &manifest.owner_epoch)
                .await
                ?;
            let (link, events, process) = Link::connect(&owner, &manifest.owner_epoch)
                .await
                .map_err(policy)?;
            if !link
                .loaded(&manifest.expected_thread_id)
                .await
                ?
            {
                return Err( ErrorCode::NotReady);
            }
            manifest
                .presence(&path)
                .map_err(policy)?;
            source
                .recheck()
                .map_err(policy)?;
            link.skills.lock().arm(&manifest.expected_thread_id);
            let resumed = link
                .rpc("thread/resume", json!({"threadId":manifest.expected_thread_id.0,"excludeTurns":true,"initialTurnsPage":{"limit":20,"sortDirection":"desc","itemsView":"full"}}))
                .await
                ?;
            if native_id(&resumed["thread"], "id")
                ?
                != manifest.expected_thread_id
                || !link
                    .loaded(&manifest.expected_thread_id)
                    .await
                    ?
            {
                return Err( ErrorCode::NotReady);
            }
            let after = manifest
                .presence(&path)
                .map_err(policy)?;
            if after.connection_epoch != presence.connection_epoch
                || after.tui != presence.tui
                || !after.associated_thread_ids.contains(&manifest.expected_thread_id)
            {
                return Err( ErrorCode::StaleSession);
            }
            source
                .recheck()
                .map_err(policy)?;
            link.skills.lock().pin(&resumed);
            Ok((owner, path, manifest, presence, link, events, process, resumed))
        }
        .await;
        let (owner, path, manifest, presence, link, mut events, process, resumed) = match setup {
            Ok(setup) => setup,
            Err(failure) => return Err(failure),
        };
        let result=async{
            let key=StableSessionKey{host_id:self.security.host.clone(),server_id:source.server_id.clone(),server_incarnation:source.incarnation.clone(),terminal_id:agent.terminal_id.clone(),provider_owner_identity:Id(format!("codex:{}:{}:{}:{}:{}:{}",owner.id.0,process.linux_boot_id.0,process.pid,process.start_ticks,manifest.owner_epoch.socket_device,manifest.owner_epoch.socket_inode)),native_conversation_id:manifest.expected_thread_id.clone()};
            let mut initial=initial_state(&source,&agent,&manifest.expected_thread_id);initial.summary.capabilities.artifacts=presence.tui.as_ref().is_some_and(|tui|herdr::artifacts::agent_artifacts_available(&source,&agent,tui));initial.summary.capabilities.terminal_create=presence.tui.as_ref().is_some_and(|tui|herdr::agent_terminal_available(&source,&agent,tui));initial.summary.capabilities.attachments=crate::attachments::Namespace::capture(&self.topology.config,&process).is_ok();
            let reader_link = link.clone();
            let reader_reporter = self.invalidation_reporter.clone();
            let (target,epoch)=self.topology.begin_native_reported(key.clone(),process.clone(),initial,true,move|target,epoch|reader_link.set_reader_capture(reader_reporter,target,epoch)).await?;
            let connection=Arc::new(Connection{target:target.clone(),epoch:epoch.clone(),key,owner:process,owner_config:owner,manifest,path,presence,source,agent,link:link.clone(),dialogs:Mutex::new(Vec::new()),resolved:Mutex::new(Vec::new()),history:Mutex::new(History::default()),history_mode:Mutex::new(None),observation:Mutex::new(()),invalidation_reporter:self.invalidation_reporter.clone()});
            let installed: Result<(), ErrorCode> = async{
                connection.physical(&self.security.state)?;
                let metadata=self.native_history(&connection,"thread/read",json!({"threadId":connection.key.native_conversation_id.0,"includeTurns":false})).await?;
                let mode=HistoryMode::from_thread(&metadata["thread"],&connection.key.native_conversation_id)?;
                *connection.history_mode.lock()=Some(mode);
                let (records,older_available)=if mode==HistoryMode::Paginated {
                    let page=self.native_history(&connection,"thread/items/list",FlatPosition::head().request(&connection.key.native_conversation_id)).await?;
                    let (mut entries,next)=FlatPosition::head().accept(&page)?;
                    entries.reverse();
                    connection.history.lock().page(&self.security.state,&target,entries,next.and_then(|p|p.cursor),true)?
                } else {
                    let page=if resumed.get("initialTurnsPage").is_some_and(|p|!p.is_null()){resumed["initialTurnsPage"].clone()}else{self.native_history(&connection,"thread/turns/list",json!({"threadId":connection.key.native_conversation_id.0,"limit":20,"sortDirection":"desc","itemsView":"full"})).await?};
                    let mut walk=Continuation::seed(None,UINT_MAX);
                    walk.accept_page(&page)?;
                    let mut entries=walk.remaining.as_ref().clone(); entries.reverse();
                    let more=walk.has_more();
                    let (records,_)=connection.history.lock().page(&self.security.state,&target,entries,None,true)?;
                    (records,more)
                };
                {let history=connection.history.lock();self.security.state.adapter_history_size(&target,&epoch,history.bytes+history.records.iter().map(|(id,row)|256+id.0.len()+row.turn.0.len()).sum::<usize>())?;}
                let mut display=self.security.state.session_state(&target,&epoch)?;apply_thread_state(&mut display,&resumed["thread"]);
                if let Some(turns)=resumed["initialTurnsPage"]["data"].as_array() {
                    for turn in turns { if turn["status"]=="inProgress" { display.native_turn_id=Some(native_id(turn,"id")?); } }
                }
                display.summary.connectivity=SessionConnectivity::Ready;
                display.summary.history_coverage.older_available=older_available;
                self.topology.finish_native(&connection.key,&connection.owner,&epoch,ProviderSnapshot{state:display,pending:Vec::new(),history:records,older_available,native_older:None}).await?;
                { let mut skills=link.skills.lock(); skills.publication=Some((self.security.state.clone(),target.clone(),epoch.clone())); skills.publish(); }
                self.connections.lock().insert(target.session_id.clone(),connection.clone());
                let status=self.security.state.session_state(&target,&epoch)?.summary.status;
                let change=crate::storage::AttentionTransition{target:target.clone(),correlation:Id("snapshot".into()),previous:status,current:status,snapshot:true,new_actionable:false};
                self.attention_commit(&connection,move|store|store.attention_transition(&change,crate::pairing::now_ms())).await?;
                // No stock replay-complete marker exists: queued requests/events are applied in wire order.
                let mut tick=tokio::time::interval(Duration::from_secs(1));
                loop{tokio::select!{
                    _=stop.changed()=>{if *stop.borrow(){break}},
                    event=events.recv()=>{
                        let Some(event)=event else {
                            let code=link.reader_failure().await;
                            connection.report_invalidation("follower", "native_read", code);
                            return Err(code);
                        };
                        // Keep the original authenticated stream ordered while observation
                        // is pending; the reducer's independent gate prevents mutation/Ready.
                        if let Err(code) = connection.readiness(&self.security.state,&self.topology) {
                            connection.report_invalidation("follower", "connection_readiness", code);
                            return Err(code);
                        }
                        if event.order>link.snapshot_boundary.load(Ordering::Acquire){self.attention_before_event(&connection,&event.value).await?;}
                        if (event.order>link.snapshot_boundary.load(Ordering::Acquire)||event.value.get("id").is_some()||event.value["method"]=="serverRequest/resolved") && let Err(code) = self.apply_event(&connection,&event.value) {
                            connection.report_invalidation("follower", "event_application", code);
                            return Err(code);
                        }
                    },
                    _=tick.tick()=>{
                        if let Err(code) = self.read_connection(&target) {
                            connection.report_invalidation("follower", "readiness", code);
                            return Err(code);
                        }
                    },
                }}Ok(())
            }.await;
            if let Err(code) = &installed {
                connection.report_invalidation("installation", "initial_installation", *code);
                let _=self.security.state.abort_install(&target,&epoch);
            }
            {let mut connections=self.connections.lock();if connections.get(&target.session_id).is_some_and(|c|c.epoch==epoch){connections.remove(&target.session_id);}}
            let _=self.security.state.binding_offline(&target,&epoch,"Native Codex source/binding disconnected; completed history retained, responses never replayed");installed
        }.await;
        link.close().await;
        result
    }
}
fn initial_state(source: &Source, agent: &Agent, thread: &Id) -> SessionState {
    SessionState{summary:SessionSummary{session_id:Uuid::random(),generation:Uuid::random(),server_id:source.server_id.clone(),workspace_id:agent.workspace_id.clone(),terminal_id:agent.terminal_id.clone(),pane_id:agent.pane_id.clone(),provider:Provider::Codex,native_id:thread.clone(),title:"Codex".into(),status:SessionStatus::Unknown,connectivity:SessionConnectivity::Synchronizing,capabilities:Capabilities{ artifacts:false, terminal_create:false, attachments:false, worktree_create: false,history:true,prompt:true,cancel:true,respond:true,prompt_mode:PromptMode::SubmitOrSteer,cancel_mode:CancelMode::NativeTurn,terminal_read:false,worktrees_read:false,changes_read:false,workspace_rename: false, agent_rename: false, worktree_remove:false,skills_read:false},skills_generation:None,history_coverage:HistoryCoverage{scope:HistoryScope::Thread,source:HistorySource::NativePages,older_available:true,source_truncated:false,desktop_branch_known:false},reason:Some("Exact bound launch thread; desktop selection unknown. Stock loaded/resume identity checks are non-atomic".into())},native_turn_id:None,desktop_selection:DesktopSelection::BoundLaunchThread,activity:Activity{state:ActivityState::Unknown,text:None,tool_record_id:None}}
}
fn apply_thread_state(state: &mut SessionState, thread: &Value) {
    if let Some(name) = thread.get("name").and_then(Value::as_str) {
        let title = name.to_owned();
        if title.validate().is_ok() {
            state.summary.title = title;
        }
    }
    let status = thread.get("status").unwrap_or(thread);
    let typ = status
        .get("type")
        .and_then(Value::as_str)
        .or_else(|| status.as_str());
    state.summary.status = match typ {
        Some("idle") => SessionStatus::Idle,
        Some("active")
            if status
                .get("activeFlags")
                .and_then(Value::as_array)
                .is_some_and(|a| !a.is_empty()) =>
        {
            SessionStatus::Attention
        }
        Some("active") => SessionStatus::Working,
        _ => SessionStatus::Unknown,
    };
    if typ == Some("idle") {
        state.native_turn_id = None;
    }
    state.activity.state = match state.summary.status {
        SessionStatus::Idle => ActivityState::Idle,
        SessionStatus::Working => ActivityState::Working,
        _ => ActivityState::Unknown,
    };
}

impl Runtime {
    async fn attention_commit<T:Send+'static>(self:&Arc<Self>,c:&Arc<Connection>,commit:impl FnOnce(&mut crate::storage::Storage)->crate::storage::StoreResult<T>+Send+'static)->SecurityResult<T>{
        let runtime=self.clone();let original=c.clone();
        self.security.storage.call(move|store|{
            original.readiness(&runtime.security.state,&runtime.topology).map_err(crate::storage::StorageError::Policy)?;
            let connections=runtime.connections.lock();
            if !connections.get(&original.target.session_id).is_some_and(|current|Arc::ptr_eq(current,&original)){return Err(crate::storage::StorageError::Policy(ErrorCode::StaleSession))}
            runtime.security.state.with_attention_source(&original.target,&original.epoch,Provider::Codex,||commit(store)).map_err(crate::storage::StorageError::Policy)?
        }).await.map_err(pairing::store_error)
    }
    async fn attention_before_event(self:&Arc<Self>,c:&Arc<Connection>,event:&Value)->SecurityResult<()>{
        if c.readiness(&self.security.state,&self.topology)?.is_none(){return Ok(())}
        let p=&event["params"];if p.get("threadId").and_then(Value::as_str)!=Some(c.key.native_conversation_id.0.as_str()){return Ok(())}
        let before=self.security.state.session_state(&c.target,&c.epoch)?;let mut current=before.summary.status;let mut native=None;let mut actionable=false;
        if event.get("id").is_some(){
            let id=native_json::rpc_id(&event["id"]).map_err(policy)?;
            {if c.resolved.lock().contains(&id)||c.dialogs.lock().iter().any(|d|d.id==id){return Ok(())}}
            native=Some(serde_json::to_string(&id).map_err(policy)?);current=SessionStatus::Attention;actionable=true;
        }else{match event["method"].as_str(){
            Some("turn/started")=>{current=SessionStatus::Working;native=Some(native_id(&p["turn"],"id")?.0);},
            Some("turn/completed")=>{let turn=native_id(&p["turn"],"id")?;if before.native_turn_id.as_ref()!=Some(&turn){return Ok(())}current=SessionStatus::Idle;native=Some(turn.0);},
            Some("thread/status/changed")=>{let mut display=before.clone();apply_thread_state(&mut display,&p["status"]);current=display.summary.status;native=before.native_turn_id.as_ref().map(|id|id.0.clone());},
            _=>return Ok(())
        }}
        let family=if actionable{"request"}else if current==SessionStatus::Idle{"completed"}else{"state"};
        let correlation=native.as_deref().map(|native|crate::push::correlation(&c.target,"codex",native,family)).unwrap_or_else(||Id("native-checkpoint".into()));
        let change=crate::storage::AttentionTransition{target:c.target.clone(),correlation,previous:before.summary.status,current,snapshot:native.is_none(),new_actionable:actionable};
        let events=self.attention_commit(c,move|store|store.attention_transition(&change,crate::pairing::now_ms())).await?;
        self.security.publish_attention(events).await
    }
    fn apply_event(&self, c: &Connection, event: &Value) -> Result<(), ErrorCode> {
        let method = string(event, "method")?;
        let p = &event["params"];
        (|| -> Result<(), ErrorCode> {
            let scoped = p
                .get("threadId")
                .or_else(|| p.get("conversationId"))
                .and_then(Value::as_str);
            if scoped.is_some_and(|id| id != c.key.native_conversation_id.0)
                || scoped.is_none() && event.get("id").is_none()
            {
                return Ok(());
            }
            let state = &self.security.state;
            if event.get("id").is_some() {
                let id = native_json::rpc_id(&event["id"]).map_err(policy)?;
                let mut dialogs = c.dialogs.lock();
                if c.resolved.lock().contains(&id) {
                    return Ok(());
                }
                if let Some(existing) = dialogs.iter().find(|d| d.id == id) {
                    if existing.fingerprint != sha256(&serde_json::to_vec(event).map_err(policy)?) {
                        return Err(ErrorCode::NotReady);
                    }
                    return Ok(());
                }
                if dialogs.len() >= 64 {
                    let mut display = state.session_state(&c.target, &c.epoch)?;
                    display.summary.status = SessionStatus::Attention;
                    display.summary.reason=Some("Additional native requests are desktop-only: mobile pending-card capacity reached; no partial responses are offered".into());
                    state.update_from_source(&c.target, &c.epoch, SessionUpdate::State(display))?;
                    return Ok(());
                }
                let history = c.history.lock();
                let file = p
                    .get("itemId")
                    .and_then(Value::as_str)
                    .and_then(|id| history.records.get(&Id(id.into())))
                    .map(|r| &r.item);
                let mut dialog = project_request(state, &c.target, event, file)?;
                let occupied = dialogs
                    .iter()
                    .map(|d| native_bytes(&d.pending))
                    .sum::<usize>();
                if occupied + native_bytes(&dialog.pending) > FRAME_MAX - 65_536 {
                    dialog.pending.kind = PresentationKind::DesktopOnly;
                    dialog.pending.presentation=Presentation::DesktopOnly(PresentationDesktopOnly{title:text_content(state,&c.target,"Native request is desktop-only".into())?,reason:"Complete card exceeds the remaining bounded mobile snapshot budget; no choices or fields were filtered".into()});
                    dialog.choices.clear();
                }
                state.update_from_source(
                    &c.target,
                    &c.epoch,
                    SessionUpdate::Open(dialog.pending.clone()),
                )?;
                dialogs.push(dialog);
                return Ok(());
            }
            match method.as_str() {
                "thread/closed" => {
                    return Err(ErrorCode::NotReady);
                }
                "thread/name/updated" => {
                    let mut display = state.session_state(&c.target, &c.epoch)?;
                    if let Some(name) = p.get("threadName").and_then(Value::as_str) {
                        let title = name.to_owned();
                        if title.validate().is_ok() {
                            display.summary.title = title;
                        }
                    } else if p.get("threadName").is_none_or(Value::is_null) {
                        display.summary.title = "Codex".into();
                    }
                    state.update_from_source(&c.target, &c.epoch, SessionUpdate::State(display))?;
                }
                "serverRequest/resolved" => {
                    let id = native_json::rpc_id(&p["requestId"]).map_err(policy)?;
                    let mut dialogs = c.dialogs.lock();
                    if let Some(i) = dialogs
                        .iter()
                        .position(|d| d.id == id && d.thread == c.key.native_conversation_id)
                    {
                        let dialog = dialogs.remove(i);
                        state.update_from_source(
                            &c.target,
                            &c.epoch,
                            SessionUpdate::Close(
                                dialog.pending.request_key,
                                RequestClosedReason::Resolved,
                            ),
                        )?;
                    }
                    let mut resolved = c.resolved.lock();
                    if !resolved.contains(&id) {
                        if resolved.len() >= 64 {
                            resolved.remove(0);
                        }
                        resolved.push(id);
                    }
                }
                "thread/status/changed" => {
                    if p["status"]["type"] == "notLoaded" || p["status"] == "notLoaded" {
                        return Err(ErrorCode::NotReady);
                    }
                    let mut display = state.session_state(&c.target, &c.epoch)?;
                    apply_thread_state(&mut display, &p["status"]);
                    state.update_from_source(&c.target, &c.epoch, SessionUpdate::State(display))?;
                }
                "turn/started" | "turn/completed" => {
                    let turn = native_id(&p["turn"], "id")?;
                    let mut display = state.session_state(&c.target, &c.epoch)?;
                    if method == "turn/started" {
                        display.native_turn_id = Some(turn.clone());
                        display.summary.status = SessionStatus::Working;
                        display.activity.state = ActivityState::Working;
                    } else if display.native_turn_id.as_ref() == Some(&turn) {
                        display.native_turn_id = None;
                        display.summary.status = SessionStatus::Idle;
                        display.activity = Activity {
                            state: ActivityState::Idle,
                            text: None,
                            tool_record_id: None,
                        };
                    }
                    state.update_from_source(&c.target, &c.epoch, SessionUpdate::State(display))?;
                    if let Some(items) = p["turn"].get("items").and_then(Value::as_array) {
                        for item in items {
                            self.live_item(c, &turn, item, (method == "turn/completed", None))?;
                        }
                    }
                }
                "item/started" | "item/completed" => {
                    let turn = native_id(p, "turnId")?;
                    self.live_item(
                        c,
                        &turn,
                        &p["item"],
                        (
                            method == "item/completed",
                            native_timestamp(&p["startedAtMs"]),
                        ),
                    )?;
                }
                "item/agentMessage/delta"
                | "item/commandExecution/outputDelta"
                | "item/fileChange/outputDelta"
                | "item/reasoning/summaryTextDelta"
                | "item/reasoning/textDelta" => {
                    let update = c.history.lock().delta(state, &c.target, &method, p)?;
                    state.update_from_source(&c.target, &c.epoch, update)?;
                }
                "turn/diff/updated" | "turn/plan/updated" | "item/mcpToolCall/progress" => {
                    let safe = safe_native(p);
                    state.update_from_source(
                        &c.target,
                        &c.epoch,
                        SessionUpdate::Activity(Activity {
                            state: ActivityState::Working,
                            text: Some(content(
                                state,
                                &c.target,
                                Mime::ApplicationJson,
                                safe.to_string(),
                            )?),
                            tool_record_id: p
                                .get("itemId")
                                .and_then(Value::as_str)
                                .map(|s| Id(s.into())),
                        }),
                    )?;
                }
                // thread/started is informational, never TUI binding or selection authority.
                _ => {}
            }
            Ok(())
        })()
    }
    fn live_item(
        &self,
        c: &Connection,
        turn: &Id,
        item: &Value,
        lifecycle: (bool, Option<Uint>),
    ) -> Result<(), ErrorCode> {
        let (update, bytes) = {
            let mut history = c.history.lock();
            let update =
                history.live_item(&self.security.state, &c.target, turn, item, lifecycle)?;
            let bytes = history.bytes
                + history
                    .records
                    .iter()
                    .map(|(id, row)| 256 + id.0.len() + row.turn.0.len())
                    .sum::<usize>();
            (update, bytes)
        };
        self.security
            .state
            .adapter_history_size(&c.target, &c.epoch, bytes)?;
        match update {
            SessionUpdate::History(records) => self
                .security
                .state
                .history_from_source(&c.target, &c.epoch, records),
            update => self
                .security
                .state
                .update_from_source(&c.target, &c.epoch, update),
        }
    }
    pub async fn session_open(
        &self,
        auth: &Authorized,
        request: Uuid,
        target: &Target,
    ) -> SecurityResult<Uuid> {
        self.read_connection(target)?;
        let _guard = self.security.guard(auth).await?;
        self.security
            .state
            .session_open(&auth.connection_id, request, target, pairing::now_ms())
    }
    /// One existing-owner read; the pinned cwd never comes from the phone or a fallback.
    pub(crate) async fn skills_list(&self, target:&Target) -> SecurityResult<(SkillsReadContext,SkillsListResult)> {
        let c=self.connection(target)?;
        let (cwd,generation)=c.link.skills.lock().snapshot()?;
        let before=self.security.state.session_state(target,&c.epoch)?;
        if !before.summary.capabilities.skills_read || before.summary.skills_generation.as_ref()!=Some(&generation) { return Err(ErrorCode::NotReady); }
        let native=c.link.rpc("skills/list",json!({"cwds":[cwd],"forceReload":false})).await?;
        let current=self.connection(target)?;
        if !Arc::ptr_eq(&c,&current) || c.link.skills.lock().snapshot()? != (cwd.clone(),generation.clone()) { return Err(ErrorCode::StaleSession); }
        let data=native["data"].as_array().ok_or(ErrorCode::NotReady)?;
        if data.len()!=1 || data[0]["cwd"].as_str()!=Some(cwd.as_str()) { return Err(ErrorCode::NotReady); }
        let rows=data[0]["skills"].as_array().ok_or(ErrorCode::NotReady)?;
        let errors=data[0]["errors"].as_array().ok_or(ErrorCode::NotReady)?;
        if rows.len()>128 || errors.len()>32 { return Err(ErrorCode::ResourceLimit); }
        fn text(value:&Value,key:&str)->SecurityResult<Text> { value[key].as_str().map(|s|s.to_owned()).ok_or(ErrorCode::NotReady) }
        fn optional(value:&Value,key:&str)->SecurityResult<Option<Text>> { match value.get(key) { None|Some(Value::Null)=>Ok(None),Some(Value::String(s))=>Ok(Some(s.clone())),_=>Err(ErrorCode::NotReady) } }
        let skills=rows.iter().map(|row| {
            let interface=row.get("interface").filter(|v|!v.is_null());
            if interface.is_some_and(|v|!v.is_object()) { return Err(ErrorCode::NotReady); }
            Ok(SkillEntry{name:text(row,"name")?,description:text(row,"description")?,path:Some(text(row,"path")?),scope:text(row,"scope")?,enabled:row["enabled"].as_bool().ok_or(ErrorCode::NotReady)?,display_name:interface.map(|v|optional(v,"displayName")).transpose()?.flatten(),short_description:optional(row,"shortDescription")?.or(interface.map(|v|optional(v,"shortDescription")).transpose()?.flatten()),command_name:None,document_ref:None})
        }).collect::<SecurityResult<Vec<_>>>()?;
        let errors=errors.iter().map(|row|Ok(SkillLoadError{path:text(row,"path")?,message:text(row,"message")?})).collect::<SecurityResult<Vec<_>>>()?;
        let result=SkillsListResult{target:target.clone(),catalogue_generation:generation.clone(),skills,errors,provider:Provider::Codex};
        result.validate().map_err(|_|ErrorCode::ResourceLimit)?;
        Ok((SkillsReadContext{original:c,cwd,generation},result))
    }
    /// Re-admit the retained original after authorization waits, without resolving a replacement.
    pub(crate) fn enqueue_skills_list(&self,connection:&Uuid,response_id:Uuid,context:&SkillsReadContext,result:SkillsListResult)->SecurityResult<()> {
        let c=&context.original;
        let installed=||self.connections.lock().get(&c.target.session_id).is_some_and(|current|Arc::ptr_eq(current,c));
        if result.target!=c.target || result.catalogue_generation!=context.generation || !installed() {return Err(ErrorCode::StaleSession);}
        c.validate(&self.security.state,&self.topology)?;
        // Match shutdown's connections -> Skills order and pin original membership until enqueue.
        let connections=self.connections.lock();
        if !connections.get(&c.target.session_id).is_some_and(|current|Arc::ptr_eq(current,c)) {return Err(ErrorCode::StaleSession);}
        let skills=c.link.skills.lock();
        if skills.snapshot()? != (context.cwd.clone(),context.generation.clone()) {return Err(ErrorCode::StaleSession);}
        // Keep ingress invalidation serialized through the existing atomic State enqueue.
        self.security.state.skills_list_response(connection,&c.target,&c.epoch,response_id,result)
    }
    pub async fn history(
        &self,
        peer: &AuthenticatedPeer,
        params: &SessionHistoryParams,
    ) -> SecurityResult<SessionHistoryResult> {
        use crate::state::{BrowseLookup, BrowseTicket};
        let state = &self.security.state;
        let c = self.read_connection(&params.target)?;
        let limit = params.limit.unwrap_or(Uint(100)).0 as usize;
        let ticket: BrowseTicket = if let Some(cursor) = &params.cursor {
            match state.browse_page(
                peer,
                &params.target,
                params.view_id.as_ref().ok_or(ErrorCode::InvalidRequest)?,
                cursor,
                limit,
                pairing::now_ms(),
            )? {
                BrowseLookup::Cached(page) => return Ok(page),
                BrowseLookup::Fetch(ticket) => ticket,
            }
        } else {
            if params.view_id.is_some() {
                return Err(ErrorCode::InvalidRequest);
            }
            let position = match *c.history_mode.lock() {
                Some(HistoryMode::Paginated) => BrowseSource::Flat(FlatPosition::head()),
                Some(HistoryMode::Legacy) => {
                    BrowseSource::Legacy(Continuation::seed(None, UINT_MAX))
                }
                None => return Err(ErrorCode::NotReady),
            };
            state.begin_browse(
                peer,
                &params.target,
                &c.epoch,
                position.position()?,
                limit,
                pairing::now_ms(),
            )?
        };
        let gate = ticket.gate();
        let _fetch = gate.lock().await;
        // Another waiter may already have materialized this exact native page.
        let ticket = match state.browse_page(
            peer,
            &params.target,
            ticket.view_id(),
            &ticket.cursor(),
            limit,
            pairing::now_ms(),
        )? {
            BrowseLookup::Cached(page) => return Ok(page),
            BrowseLookup::Fetch(ticket) => ticket,
        };
        if ticket.source() != &c.epoch {
            return Err(ErrorCode::CursorExpired);
        }
        let position = ticket.position::<BrowseSource>()?.clone();
        let (entries, next) = match position {
            BrowseSource::Flat(position) => {
                let page = self
                    .native_history(
                        &c,
                        "thread/items/list",
                        position.request(&c.key.native_conversation_id),
                    )
                    .await?;
                let (entries, next) = position.accept(&page)?;
                (entries, next.map(BrowseSource::Flat))
            }
            BrowseSource::Legacy(mut position) => {
                if position.offset >= position.remaining.len() {
                    let (method, request) = position.request(&c.key.native_conversation_id);
                    let page = self.native_history(&c, method, request).await?;
                    position.accept_page(&page)?;
                }
                let end = (position.offset + 100).min(position.remaining.len());
                let mut entries = position.remaining[position.offset..end].to_vec();
                for (index, entry) in entries.iter_mut().enumerate() {
                    entry.order = position
                        .before
                        .checked_sub(index as u64)
                        .ok_or(ErrorCode::ResourceLimit)?;
                }
                position.before = position
                    .before
                    .checked_sub(entries.len() as u64)
                    .ok_or(ErrorCode::ResourceLimit)?;
                position.offset = end;
                let more = position.has_more();
                (entries, more.then_some(BrowseSource::Legacy(position)))
            }
        };
        let mut records = Vec::new();
        for entry in entries.into_iter().filter(|e| e.complete) {
            records.push(project_safe_item(
                state,
                &c.target,
                &entry.turn,
                &entry.item,
                entry.order,
                RecordSource::NativePage,
                (true, entry.timestamp),
            )?);
        }
        c.physical(state)?;
        if !self
            .connections
            .lock()
            .get(&params.target.session_id)
            .is_some_and(|current| Arc::ptr_eq(current, &c))
        {
            return Err(ErrorCode::StaleSession);
        }
        state.finish_browse(
            ticket,
            records,
            next.map(BrowseSource::position).transpose()?,
            pairing::now_ms(),
        )
    }
    pub async fn mutate(
        self: Arc<Self>,
        auth: Arc<Authorized>,
        request: Request,
    ) -> SecurityResult<Receipt> {
        let create = matches!(request.body, RequestBody::SessionCreate(_));
        let action = request.id.clone();
        // Session creation keeps one absolute budget across owner, thread, relay and Ready.
        let deadline =
            tokio::time::Instant::now() + Duration::from_secs(if create { 300 } else { 120 });
        let (tx, rx) = oneshot::channel();
        let owner = self.clone();
        let validator = self.clone();
        let original = request.clone();
        let authorization = auth.clone();
        let handle = tokio::runtime::Handle::current();
        let reservation = self
            .security
            .reserve(
                &auth,
                request,
                move |r| {
                    validator.validate(r)?;
                    if validator.jobs.lock().len() >= 64 {
                        return Err(ErrorCode::Busy);
                    }
                    Ok(())
                },
                move |committed| {
                    let worker = owner.clone();
                    owner.jobs.lock().spawn_on(
                        async move {
                            let action = committed.action_id;
                            let outcome = if let RequestBody::SessionCreate(params) = &original.body
                            {
                                worker
                                    .creation_job(&authorization, &action, params, deadline)
                                    .await
                            } else {
                                worker
                                    .write_once(&authorization, original)
                                    .await
                                    .map(|_| ())
                            };
                            if let Err(code) = outcome {
                                let _ = worker.fail_unentered(&authorization, &action, code).await;
                            }
                            let _ = tx.send(worker.receipt(&authorization, &action).await);
                        },
                        &handle,
                    );
                },
            )
            .await?;
        if let Some(receipt) = reservation_reply(create, reservation) {
            return Ok(receipt);
        }
        match tokio::time::timeout(Duration::from_secs(if create { 119 } else { 14 }), rx).await {
            Ok(Ok(r)) => r,
            _ => self.receipt(&auth, &action).await,
        }
    }
    async fn receipt(&self, auth: &Authorized, action: &Uuid) -> SecurityResult<Receipt> {
        let peer = auth.proof.clone();
        let id = action.clone();
        let receipt = self
            .security
            .storage
            .call(move |s| s.status(&peer, &id, pairing::now_ms()))
            .await
            .map_err(pairing::store_error)?
            .ok_or(ErrorCode::NotFound)?;
        self.security.state.peer_event(
            &auth.proof,
            EventBody::ActionStatus(ActionStatusEvent {
                receipt: receipt.clone(),
            }),
        )?;
        Ok(receipt)
    }
    async fn fail_unentered(
        &self,
        auth: &Authorized,
        action: &Uuid,
        code: ErrorCode,
    ) -> SecurityResult<Receipt> {
        let proof = auth.proof.clone();
        let id = action.clone();
        self.security
            .storage
            .call(move |s| {
                let current = s
                    .status(&proof, &id, pairing::now_ms())?
                    .ok_or(crate::storage::StorageError::Policy(ErrorCode::NotFound))?;
                if matches!(
                    current.state,
                    ReceiptState::Failed
                        | ReceiptState::Succeeded
                        | ReceiptState::Dispatched
                        | ReceiptState::Uncertain
                ) {
                    return Ok(current);
                }
                let pending = current.state == ReceiptState::Dispatching
                    && current
                        .creation
                        .as_ref()
                        .is_none_or(|p| p.stage_state == StageState::Dispatching);
                s.transition(
                    &proof,
                    &id,
                    ReceiptTransition {
                        state: if pending {
                            ReceiptState::Uncertain
                        } else {
                            ReceiptState::Failed
                        },
                        proof: if pending {
                            TransitionProof::LostOutcome
                        } else {
                            TransitionProof::DefinitiveNoEffect(Id(
                                "No later native effect admitted".into(),
                            ))
                        },
                        result: None,
                        error: Some(crate::transport::wire_error(code)),
                        at: pairing::now_ms(),
                    },
                )
            })
            .await
            .map_err(pairing::store_error)
    }
    async fn write_once(&self, auth: &Authorized, request: Request) -> SecurityResult<Receipt> {
        let target = match &request.body {
            RequestBody::PromptSubmit(p) => &p.target,
            RequestBody::TurnCancel(p) => &p.target,
            RequestBody::RequestRespond(p) => &p.target,
            _ => return Err(ErrorCode::InvalidRequest),
        };
        let c = self.connection(target)?;
        if !c.link.loaded(&c.key.native_conversation_id).await? {
            return Err(ErrorCode::NotReady);
        }
        c.validate(&self.security.state, &self.topology)?;
        let mut writer = tokio::time::timeout(Duration::from_secs(5), c.link.writer.lock())
            .await
            .map_err(|_| ErrorCode::Busy)?;
        let mut callback = None;
        let (bytes, initial) = match &request.body {
            RequestBody::PromptSubmit(p) => {
                let id = NativeRpcId::String(request.id.0.clone());
                callback = Some(c.link.callback(id, false)?);
                (serde_json::to_vec(&json!({"id":request.id.0,"method":"turn/start","params":{"threadId":c.key.native_conversation_id.0,"clientUserMessageId":request.id.0,"input":[{"type":"text","text":p.text.0,"text_elements":[]}]}})).map_err(policy)?,ActionResult::Prompt(ActionResultPrompt{native_turn_id:None}))
            }
            RequestBody::TurnCancel(p) => {
                let turn = p
                    .native_turn_id
                    .as_ref()
                    .filter(|t| !t.0.is_empty())
                    .ok_or(ErrorCode::InvalidRequest)?;
                callback = Some(
                    c.link
                        .callback(NativeRpcId::String(request.id.0.clone()), false)?,
                );
                (serde_json::to_vec(&json!({"id":request.id.0,"method":"turn/interrupt","params":{"threadId":c.key.native_conversation_id.0,"turnId":turn.0}})).map_err(policy)?,ActionResult::Cancel(ActionResultCancel{native_turn_id:Some(turn.clone())}))
            }
            RequestBody::RequestRespond(p) => {
                let dialogs = c.dialogs.lock();
                let dialog = dialogs
                    .iter()
                    .find(|d| d.pending.request_key == p.request_key)
                    .ok_or(ErrorCode::AlreadyResolved)?;
                (
                    dialog.response(&p.answer)?,
                    ActionResult::Response(ActionResultResponse {
                        resolution: ResponseResolution::Written,
                    }),
                )
            }
            _ => return Err(ErrorCode::InvalidRequest),
        };
        let fd = c.link.fd.as_fd().try_clone_to_owned().map_err(policy)?;
        let action = request.id.clone();
        let outcome = self
            .security
            .boundary(
                auth,
                action.clone(),
                || self.validate(&request),
                fd,
                async {
                    if let RequestBody::RequestRespond(p) = &request.body {
                        let mut dialogs = c.dialogs.lock();
                        let Some(dialog) = dialogs
                            .iter_mut()
                            .find(|d| d.pending.request_key == p.request_key && !d.sent)
                        else {
                            return ReceiptTransition {
                                state: ReceiptState::Failed,
                                proof: TransitionProof::DefinitiveNoEffect(Id(
                                    "Native dialog already consumed locally".into(),
                                )),
                                result: None,
                                error: Some(crate::transport::wire_error(
                                    ErrorCode::AlreadyResolved,
                                )),
                                at: pairing::now_ms(),
                            };
                        };
                        dialog.sent = true;
                    }
                    if writer
                        .send(Message::Text(
                            String::from_utf8(bytes)
                                .expect("serialized native JSON")
                                .into(),
                        ))
                        .await
                        .is_err()
                    {
                        return ReceiptTransition {
                            state: ReceiptState::Uncertain,
                            proof: TransitionProof::LostOutcome,
                            result: None,
                            error: None,
                            at: pairing::now_ms(),
                        };
                    }
                    if let Some(reply) = callback {
                        match reply.await {
                            Ok(Ok(result)) => {
                                let acknowledged = match initial {
                                    ActionResult::Prompt(_) => {
                                        turn_admission(&result).map(|turn| {
                                            ActionResult::Prompt(ActionResultPrompt {
                                                native_turn_id: Some(turn),
                                            })
                                        })
                                    }
                                    ActionResult::Cancel(r) => {
                                        if result.is_object() {
                                            Ok(ActionResult::Cancel(r))
                                        } else {
                                            Err(ErrorCode::NotReady)
                                        }
                                    }
                                    _ => Err(ErrorCode::NotReady),
                                };
                                match acknowledged {
                                    Ok(result) => ReceiptTransition {
                                        state: ReceiptState::Succeeded,
                                        proof: TransitionProof::CorrelatedAcknowledgement(Id(
                                            "Exact native admission response".into(),
                                        )),
                                        result: Some(result),
                                        error: None,
                                        at: pairing::now_ms(),
                                    },
                                    Err(_) => ReceiptTransition {
                                        state: ReceiptState::Uncertain,
                                        proof: TransitionProof::LostOutcome,
                                        result: None,
                                        error: None,
                                        at: pairing::now_ms(),
                                    },
                                }
                            }
                            Ok(Err(ErrorCode::BackendError)) => ReceiptTransition {
                                state: ReceiptState::Failed,
                                proof: TransitionProof::DefinitiveNoEffect(Id(
                                    "Native request rejected".into(),
                                )),
                                result: None,
                                error: Some(crate::transport::wire_error(ErrorCode::BackendError)),
                                at: pairing::now_ms(),
                            },
                            _ => ReceiptTransition {
                                state: ReceiptState::Uncertain,
                                proof: TransitionProof::LostOutcome,
                                result: None,
                                error: None,
                                at: pairing::now_ms(),
                            },
                        }
                    } else {
                        ReceiptTransition {
                            state: ReceiptState::Dispatched,
                            proof: TransitionProof::WriteCompleted,
                            result: Some(initial),
                            error: None,
                            at: pairing::now_ms(),
                        }
                    }
                },
            )
            .await;
        c.link
            .callbacks
            .lock()
            .retain(|p| p.id != NativeRpcId::String(action.0.clone()));
        outcome?;
        self.receipt(auth, &action).await
    }
}

fn turn_admission(result: &Value) -> SecurityResult<Id> {
    let turn = &result["turn"];
    if turn["status"] != "inProgress" || turn["items"].as_array().is_none() {
        return Err(ErrorCode::NotReady);
    }
    native_id(turn, "id")
}

#[derive(Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct OwnerRecord {
    v: u32,
    owner_id: Uuid,
    action_id: Uuid,
    unit: String,
    launcher: std::path::PathBuf,
    wrapper: std::path::PathBuf,
    cwd: std::path::PathBuf,
    socket_agent: std::path::PathBuf,
    argv: Vec<String>,
    environment_digest: String,
    manager: ProcessEvidence,
    service: Option<ProcessEvidence>,
    epoch: Option<OwnerEpoch>,
}
fn owner_record(owner: &CodexOwner) -> std::path::PathBuf {
    owner
        .bindings_dir
        .join(format!("owner-{}.owner", owner.id.0))
}
fn owner_launcher_argv(launcher: &Path, record: &Path) -> SecurityResult<Vec<String>> {
    Ok(vec![
        launcher.to_str().ok_or(ErrorCode::NotReady)?.to_owned(),
        "codex-owner".into(),
        "--record".into(),
        record.to_str().ok_or(ErrorCode::NotReady)?.into(),
    ])
}
fn owner_exec_invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Codex owner launcher record rejected",
    )
}
fn owner_exec_record_shape(
    record: &OwnerRecord,
    record_path: &Path,
    current: &Path,
) -> io::Result<()> {
    let expected_name = format!("owner-{}.owner", record.owner_id.0);
    record
        .owner_id
        .validate()
        .map_err(|_| owner_exec_invalid())?;
    record
        .action_id
        .validate()
        .map_err(|_| owner_exec_invalid())?;
    if record.v != 2
        || record.launcher != current
        || record.service.is_some()
        || record.epoch.is_some()
        || record.unit.is_empty()
        || record.unit.len() > 255
        || !record.unit.ends_with(".service")
        || !record
            .unit
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.@".contains(&byte))
        || !record_path.is_absolute()
        || record_path.file_name().and_then(|name| name.to_str()) != Some(expected_name.as_str())
        || record.argv
            != [
                "app-server".to_owned(),
                "--listen".into(),
                format!("unix://{}", record.socket_agent.display()),
            ]
    {
        return Err(owner_exec_invalid());
    }
    Ok(())
}
fn validate_owner_exec_record(
    record: OwnerRecord,
    record_path: &Path,
    current: &Path,
) -> io::Result<OwnerRecord> {
    owner_exec_record_shape(&record, record_path, current)?;
    config::absolute(current)?;
    config::executable(current)?;
    config::executable(&record.wrapper)?;
    config::project_directory(&record.cwd)?;
    config::socket_path(&record.socket_agent)?;
    Ok(record)
}
fn owner_exec_record(record_path: &Path) -> io::Result<OwnerRecord> {
    let record: OwnerRecord = codex_relay::private_read(record_path)?;
    let current = std::env::current_exe()?;
    let record = validate_owner_exec_record(record, record_path, &current)?;
    let (manager_pid, manager_uid) = manager_peer().map_err(|_| owner_exec_invalid())?;
    let own = herdr::process_evidence(std::process::id()).map_err(|_| owner_exec_invalid())?;
    owner_manager_scope(&own, (manager_pid, manager_uid), &record.manager)?;
    let cgroup = std::fs::read_to_string("/proc/self/cgroup")?;
    let unit = cgroup
        .lines()
        .find_map(|line| line.strip_prefix("0::"))
        .and_then(|path| Path::new(path).file_name())
        .and_then(|name| name.to_str());
    if unit != Some(record.unit.as_str()) {
        return Err(owner_exec_invalid());
    }
    Ok(record)
}
fn owner_manager_scope(
    own: &ProcessEvidence,
    peer: (u32, u32),
    manager: &ProcessEvidence,
) -> io::Result<()> {
    // A zero-capability child cannot open a capability-bearing ancestor's namespace links.
    // The admitted record and authenticated socket pin that ancestor; public edges are rechecked.
    if own.namespaces.user != manager.namespaces.user
        || own.namespaces.pid != manager.namespaces.pid
        || own.linux_boot_id != manager.linux_boot_id
        || peer != (manager.pid, pairing::uid())
    {
        return Err(owner_exec_invalid());
    }
    let mut pid = own.pid;
    let mut seen = HashSet::new();
    let mut edges = Vec::new();
    let mut found = false;
    for _ in 0..128 {
        if !seen.insert(pid) {
            return Err(owner_exec_invalid());
        }
        let (parent, start, uid) = public_ancestor(pid)?;
        if uid != pairing::uid() {
            return Err(owner_exec_invalid());
        }
        edges.push((pid, parent, start, uid));
        if pid == manager.pid {
            if start != manager.start_ticks {
                return Err(owner_exec_invalid());
            }
            found = true;
            break;
        }
        pid = parent;
    }
    if !found {
        return Err(owner_exec_invalid());
    }
    for (pid, parent, start, uid) in edges {
        if public_ancestor(pid)? != (parent, start, uid) {
            return Err(owner_exec_invalid());
        }
    }
    Ok(())
}
pub fn exec_owner(record_path: &Path) -> io::Result<()> {
    use std::os::unix::process::CommandExt;
    let record = owner_exec_record(record_path)?;
    use std::os::unix::fs::MetadataExt;
    let inherited = std::fs::metadata(".")?;
    let expected = std::fs::metadata(&record.cwd)?;
    if (inherited.dev(), inherited.ino()) != (expected.dev(), expected.ino()) {
        return Err(owner_exec_invalid());
    }
    Err(std::process::Command::new(&record.wrapper)
        .args(&record.argv)
        .exec())
}
fn installed_command(name: &str) -> SecurityResult<std::path::PathBuf> {
    let path = std::env::var_os("PATH").ok_or(ErrorCode::NotReady)?;
    for parent in std::env::split_paths(&path) {
        if !parent.is_absolute() {
            continue;
        }
        let file = parent.join(name);
        if file.is_file() {
            config::executable(&file).map_err(policy)?;
            return Ok(file);
        }
    }
    Err(ErrorCode::NotReady)
}
async fn command_output(binary: &Path, args: &[String]) -> SecurityResult<(bool, String)> {
    use tokio::io::AsyncReadExt;
    config::executable(binary).map_err(policy)?;
    let mut child = tokio::process::Command::new(binary)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(policy)?;
    let mut output = child.stdout.take().ok_or(ErrorCode::NotReady)?;
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        let mut bytes = Vec::new();
        (&mut output)
            .take(65_537)
            .read_to_end(&mut bytes)
            .await
            .map_err(policy)?;
        if bytes.len() > 65_536 {
            return Err(ErrorCode::ResourceLimit);
        }
        let status = child.wait().await.map_err(policy)?;
        Ok((status.success(), String::from_utf8(bytes).map_err(policy)?))
    })
    .await;
    match result {
        Ok(Ok(v)) => Ok(v),
        other => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            match other {
                Ok(Err(e)) => Err(e),
                _ => Err(ErrorCode::Timeout),
            }
        }
    }
}
async fn unit_properties(unit: &str) -> SecurityResult<HashMap<String, String>> {
    let systemctl = installed_command("systemctl")?;
    let (_,output)=command_output(&systemctl,&["--user".into(),"show".into(),unit.into(),"--property=LoadState,MainPID,Type,UMask,WorkingDirectory,ExecStart,PartOf,BindsTo,Restart,Transient,FragmentPath,ControlGroup".into()]).await?;
    let mut properties = HashMap::new();
    for line in output.lines() {
        let (key, value) = line.split_once('=').ok_or(ErrorCode::NotReady)?;
        if properties.insert(key.into(), value.into()).is_some() {
            return Err(ErrorCode::NotReady);
        }
    }
    Ok(properties)
}
async fn service_property(unit: &str, name: &str) -> SecurityResult<Value> {
    let busctl = installed_command("busctl")?;
    let options = vec![
        "--user".into(),
        "--json=short".into(),
        "--auto-start=no".into(),
        "--allow-interactive-authorization=no".into(),
        "--timeout=5s".into(),
    ];
    let mut args = options.clone();
    args.extend([
        "call".into(),
        "org.freedesktop.systemd1".into(),
        "/org/freedesktop/systemd1".into(),
        "org.freedesktop.systemd1.Manager".into(),
        "GetUnit".into(),
        "s".into(),
        unit.into(),
    ]);
    let (ok, reply) = command_output(&busctl, &args).await?;
    if !ok {
        return Err(ErrorCode::NotReady);
    }
    let reply = native_json::parse(reply.as_bytes(), native_json::REGISTRY).map_err(policy)?;
    if reply["type"] != "o" {
        return Err(ErrorCode::NotReady);
    }
    let path = reply["data"][0]
        .as_str()
        .ok_or(ErrorCode::NotReady)?
        .to_owned();
    let mut args = options;
    args.extend([
        "get-property".into(),
        "org.freedesktop.systemd1".into(),
        path,
        "org.freedesktop.systemd1.Service".into(),
        name.into(),
    ]);
    let (ok, reply) = command_output(&busctl, &args).await?;
    if !ok {
        return Err(ErrorCode::NotReady);
    }
    native_json::parse(reply.as_bytes(), native_json::REGISTRY).map_err(policy)
}
fn exact_service_argv(property: &Value, expected: &[String]) -> SecurityResult<()> {
    if property["type"] != "a(sasbttttuii)" {
        return Err(ErrorCode::NotReady);
    }
    let commands = property["data"].as_array().ok_or(ErrorCode::NotReady)?;
    if commands.len() != 1 {
        return Err(ErrorCode::NotReady);
    }
    let command = commands[0].as_array().ok_or(ErrorCode::NotReady)?;
    if command.len() != 10 {
        return Err(ErrorCode::NotReady);
    }
    let argv = command
        .get(1)
        .and_then(Value::as_array)
        .ok_or(ErrorCode::NotReady)?;
    if command.first().and_then(Value::as_str) != expected.first().map(String::as_str)
        || command.get(2).and_then(Value::as_bool) != Some(false)
        || argv.len() != expected.len()
        || !argv
            .iter()
            .zip(expected)
            .all(|(actual, expected)| actual.as_str() == Some(expected))
    {
        return Err(ErrorCode::NotReady);
    }
    Ok(())
}
fn service_environment(property: &Value) -> SecurityResult<Vec<(String, String)>> {
    if property["type"] != "as" {
        return Err(ErrorCode::NotReady);
    }
    let mut values = Vec::new();
    let mut seen = HashSet::new();
    for value in property["data"].as_array().ok_or(ErrorCode::NotReady)? {
        let (key, value) = value
            .as_str()
            .and_then(|s| s.split_once('='))
            .ok_or(ErrorCode::NotReady)?;
        if !seen.insert(key) {
            return Err(ErrorCode::NotReady);
        }
        values.push((key.into(), value.into()));
    }
    values.sort();
    Ok(values)
}
fn service_pid(properties: &HashMap<String, String>) -> SecurityResult<u32> {
    if properties.get("LoadState").map(String::as_str) != Some("loaded")
        || properties.get("Type").map(String::as_str) != Some("exec")
        || properties.get("UMask").map(String::as_str) != Some("0077")
        || properties.get("Restart").map(String::as_str) != Some("no")
        || properties.get("PartOf").is_none_or(|s| !s.is_empty())
        || properties.get("BindsTo").is_none_or(|s| !s.is_empty())
        || properties.get("Transient").map(String::as_str) != Some("yes")
    {
        return Err(ErrorCode::NotReady);
    }
    properties
        .get("MainPID")
        .and_then(|s| s.parse().ok())
        .filter(|p| *p > 0)
        .ok_or(ErrorCode::NotReady)
}
fn native_owner_arguments(argv: &[String], expected: &[String]) -> bool {
    argv.len() > expected.len() && argv.ends_with(expected)
}
fn owned_native_process(
    properties: &HashMap<String, String>,
    service: &ProcessEvidence,
    expected: &[String],
) -> SecurityResult<ProcessEvidence> {
    use std::io::Read;
    let group = properties.get("ControlGroup").ok_or(ErrorCode::NotReady)?;
    config::absolute(Path::new(group)).map_err(policy)?;
    let path = Path::new("/sys/fs/cgroup")
        .join(group.strip_prefix('/').ok_or(ErrorCode::NotReady)?)
        .join("cgroup.procs");
    let mut content = String::new();
    std::fs::File::open(path)
        .map_err(policy)?
        .take(65_537)
        .read_to_string(&mut content)
        .map_err(policy)?;
    if content.len() > 65_536 {
        return Err(ErrorCode::ResourceLimit);
    }
    let mut found = None;
    for (index, value) in content.split_whitespace().enumerate() {
        if index >= 128 {
            return Err(ErrorCode::ResourceLimit);
        }
        let pid = value.parse::<u32>().map_err(policy)?;
        if pid == 0 {
            return Err(ErrorCode::NotReady);
        }
        let Ok(process) = herdr::process_evidence(pid) else {
            continue;
        };
        let Ok(argv) = codex_relay::process_arguments(&process) else {
            continue;
        };
        if !native_owner_arguments(&argv, expected)
            || process_descends(&process, service).is_err()
            || codex_relay::verify_distribution(&process).is_err()
        {
            continue;
        }
        if found.replace(process).is_some() {
            return Err(ErrorCode::NotReady);
        }
    }
    if herdr::process_evidence(service.pid).map_err(policy)? != *service {
        return Err(ErrorCode::StaleSession);
    }
    found.ok_or(ErrorCode::NotReady)
}
fn process_descends(child: &ProcessEvidence, parent: &ProcessEvidence) -> SecurityResult<()> {
    let mut process = child.clone();
    for _ in 0..128 {
        if process == *parent {
            return Ok(());
        }
        let stat =
            std::fs::read_to_string(format!("/proc/{}/stat", process.pid)).map_err(policy)?;
        let end = stat.rfind(')').ok_or(ErrorCode::NotReady)?;
        let pid = stat[end + 2..]
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .filter(|pid| *pid > 0)
            .ok_or(ErrorCode::NotReady)?;
        if herdr::process_evidence(process.pid).map_err(policy)? != process {
            return Err(ErrorCode::NotReady);
        }
        process = herdr::process_evidence(pid).map_err(policy)?;
    }
    Err(ErrorCode::NotReady)
}
fn manager_socket_object(path: &Path) -> io::Result<(u64, u64)> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    config::socket_path(path)?;
    config::project_directory(path.parent().ok_or_else(invalid)?)?;
    let metadata = std::fs::symlink_metadata(path)?;
    // Native systemd binds its private bus under umask0077, not Codex's0600 policy.
    if !metadata.file_type().is_socket()
        || metadata.uid() != pairing::uid()
        || metadata.mode() & 0o7777 != 0o700
        || std::fs::canonicalize(path)? != path
    {
        return Err(invalid());
    }
    Ok((metadata.dev(), metadata.ino()))
}
fn manager_process() -> SecurityResult<ProcessEvidence> {
    let (pid, _) = manager_peer()?;
    herdr::process_evidence(pid).map_err(policy)
}
fn manager_peer() -> SecurityResult<(u32, u32)> {
    use std::os::fd::AsFd;
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .ok_or(ErrorCode::NotReady)?;
    config::absolute(&runtime).map_err(policy)?;
    if runtime.as_path() != std::path::Path::new(&format!("/run/user/{}", pairing::uid())) {
        return Err(ErrorCode::NotReady);
    }
    pairing::private_dir(&runtime, false).map_err(policy)?;
    let path = runtime.join("systemd/private");
    let before = manager_socket_object(&path).map_err(policy)?;
    // SAFETY: the owned nonblocking Unix descriptor never waits under a start fence.
    let raw = unsafe {
        libc::socket(
            libc::AF_UNIX,
            libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
            0,
        )
    };
    if raw < 0 {
        return Err(ErrorCode::NotReady);
    }
    use std::os::fd::{AsRawFd, FromRawFd};
    let stream = unsafe { OwnedFd::from_raw_fd(raw) };
    let bytes = path.as_os_str().as_encoded_bytes();
    if bytes.len() > 107 {
        return Err(ErrorCode::NotReady);
    }
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (out, byte) in address.sun_path.iter_mut().zip(bytes) {
        *out = *byte as libc::c_char;
    }
    let length =
        (std::mem::offset_of!(libc::sockaddr_un, sun_path) + bytes.len() + 1) as libc::socklen_t;
    if unsafe {
        libc::connect(
            stream.as_raw_fd(),
            (&address as *const libc::sockaddr_un).cast(),
            length,
        )
    } != 0
    {
        return Err(ErrorCode::NotReady);
    }
    let fd = stream.as_fd();
    let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: live connected descriptor and correctly sized ucred storage.
    if unsafe {
        libc::getsockopt(
            std::os::fd::AsRawFd::as_raw_fd(&fd),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut credentials as *mut libc::ucred).cast(),
            &mut len,
        )
    } != 0
        || credentials.uid != pairing::uid()
        || manager_socket_object(&path).map_err(policy)? != before
    {
        return Err(ErrorCode::NotReady);
    }
    Ok((
        u32::try_from(credentials.pid).map_err(policy)?,
        credentials.uid,
    ))
}
fn public_ancestor(pid: u32) -> io::Result<(u32, u64, u32)> {
    let root = std::path::PathBuf::from(format!("/proc/{pid}"));
    let first = std::fs::read_to_string(root.join("stat"))?;
    let start = herdr::process_start_ticks(&first, pid)?;
    let end = first.rfind(')').ok_or_else(owner_exec_invalid)?;
    let parent = first[end + 2..]
        .split_whitespace()
        .nth(1)
        .and_then(|v| v.parse().ok())
        .filter(|v| *v > 0)
        .ok_or_else(owner_exec_invalid)?;
    let status = std::fs::read_to_string(root.join("status"))?;
    let uids = status
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))
        .ok_or_else(owner_exec_invalid)?
        .split_whitespace()
        .map(str::parse::<u32>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| owner_exec_invalid())?;
    if uids.len() != 4 || uids.iter().any(|uid| *uid != uids[0]) {
        return Err(owner_exec_invalid());
    }
    let uid = uids[0];
    let second = std::fs::read_to_string(root.join("stat"))?;
    let second_end = second.rfind(')').ok_or_else(owner_exec_invalid)?;
    let second_parent = second[second_end + 2..]
        .split_whitespace()
        .nth(1)
        .and_then(|v| v.parse::<u32>().ok());
    if herdr::process_start_ticks(&second, pid)? != start || second_parent != Some(parent) {
        return Err(owner_exec_invalid());
    }
    Ok((parent, start, uid))
}
fn environment() -> SecurityResult<Vec<(String, String)>> {
    let mut variables = std::env::vars_os()
        .map(|(key, value)| {
            Ok((
                key.into_string().map_err(|_| ErrorCode::NotReady)?,
                value.into_string().map_err(|_| ErrorCode::NotReady)?,
            ))
        })
        .collect::<SecurityResult<Vec<_>>>()?;
    variables.sort();
    if variables.iter().any(|(key, value)| {
        key.is_empty()
            || key.contains('=')
            || key.chars().any(char::is_control)
            || value.contains('\0')
    }) {
        return Err(ErrorCode::NotReady);
    }
    Ok(variables)
}
fn environment_digest(variables: &[(String, String)]) -> String {
    pairing::hex(&sha256(
        &serde_json::to_vec(variables).expect("environment strings"),
    ))
}
fn owner_metadata(owner: &CodexOwner, epoch: &OwnerEpoch) -> StageMetadata {
    StageMetadata {
        linux_boot_id: Some(epoch.linux_boot_id.clone()),
        pid: Some(epoch.pid),
        pid_start_ticks: epoch.start_ticks.parse().ok(),
        socket_path: owner.socket_host.to_str().map(str::to_owned),
        unit_name: Some(owner.unit.clone()),
        ..StageMetadata::default()
    }
}
fn stage(
    resources: CreationResources,
    metadata: StageMetadata,
    state: StageState,
    proof: Option<&str>,
) -> CreationOutcome {
    CreationOutcome {
        resources,
        metadata,
        state,
        definitive: proof.map(|s| Id(s.into())),
        at: pairing::now_ms(),
    }
}
fn thread_uncertain(
    resources: CreationResources,
    metadata: StageMetadata,
    phase: &'static str,
    code: ErrorCode,
) -> CreationOutcome {
    // Persist only a closed phase and wire ErrorCode; native replies stay private.
    let proof = launch_failure_proof(phase, code, None);
    stage(resources, metadata, StageState::Uncertain, Some(&proof))
}
async fn child_status(mut child: std::process::Child) -> SecurityResult<bool> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait().map_err(policy)? {
            return Ok(status.success());
        }
        if tokio::time::Instant::now() >= deadline {
            let _ = child.kill();
            // Reap off Tokio's worker; cancellation leaves the task owning this child.
            let _ = tokio::task::spawn_blocking(move || child.wait()).await;
            return Err(ErrorCode::Timeout);
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
#[derive(Clone, Copy)]
struct OwnerServiceFailure {
    guard: &'static str,
    code: ErrorCode,
    io: &'static str,
}
impl OwnerServiceFailure {
    const fn new(guard: &'static str, code: ErrorCode) -> Self {
        Self {
            guard,
            code,
            io: "none",
        }
    }
    fn from_io(guard: &'static str, error: &io::Error) -> Self {
        let io = match error.kind() {
            io::ErrorKind::NotFound => "not_found",
            io::ErrorKind::PermissionDenied => "permission_denied",
            io::ErrorKind::ConnectionRefused => "connection_refused",
            io::ErrorKind::ConnectionReset => "connection_reset",
            io::ErrorKind::ConnectionAborted => "connection_aborted",
            io::ErrorKind::NotConnected => "not_connected",
            io::ErrorKind::BrokenPipe => "broken_pipe",
            io::ErrorKind::TimedOut => "timed_out",
            io::ErrorKind::UnexpectedEof => "unexpected_eof",
            io::ErrorKind::InvalidData => "invalid_data",
            _ => "other",
        };
        Self {
            guard,
            code: policy(error),
            io,
        }
    }
}
impl Runtime {
    fn creation_unavailable(&self) -> Option<&'static str> {
        match self.topology.config.backend_reason(Provider::Codex) {
            Ok(Some(reason)) => return Some(reason),
            Err(_) => {
                return Some(
                    "Codex creation unavailable: unsafe or inaccessible configured wrapper/runtime/owner roots",
                );
            }
            Ok(None) => {}
        }
        if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            return Some(
                "Codex creation unavailable: no verified native publisher distribution for this platform",
            );
        }
        if std::env::current_exe()
            .ok()
            .is_none_or(|p| config::executable(&p).is_err())
        {
            return Some(
                "Codex creation unavailable: protected absolute same packaged binary cannot be proved",
            );
        }
        if installed_command("systemd-run").is_err() {
            return Some(
                "Codex creation unavailable: protected systemd-run executable unavailable",
            );
        }
        if installed_command("systemctl").is_err() || installed_command("busctl").is_err() {
            return Some(
                "Codex creation unavailable: protected systemctl/busctl typed service inspection unavailable",
            );
        }
        if manager_process().is_err() {
            return Some(
                "Codex creation unavailable: unchanged /run/user/UID private manager socket and actual kernel PID/start/namespace proof unavailable",
            );
        }
        None
    }
    fn creation_available(&self) -> SecurityResult<()> {
        if self.creation_unavailable().is_some() {
            Err(ErrorCode::NotReady)
        } else {
            Ok(())
        }
    }
    async fn prove_service(
        &self,
        owner: &CodexOwner,
        record: &OwnerRecord,
    ) -> Result<(ProcessEvidence, HashMap<String, String>), OwnerServiceFailure> {
        if record.v != 2
            || record.owner_id != owner.id
            || record.unit != owner.unit
            || record.socket_agent != owner.socket_agent
            || self.topology.config.wrapper(Provider::Codex) != Some(record.wrapper.as_path())
            || record.argv
                != [
                    "app-server".to_owned(),
                    "--listen".into(),
                    format!("unix://{}", owner.socket_agent.display()),
                ]
        {
            return Err(OwnerServiceFailure::new(
                "owner_record_shape",
                ErrorCode::NotReady,
            ));
        }
        if manager_process()
            .map_err(|code| OwnerServiceFailure::new("owner_record_manager", code))?
            != record.manager
        {
            return Err(OwnerServiceFailure::new(
                "owner_record_manager",
                ErrorCode::NotReady,
            ));
        }
        config::executable(&record.launcher)
            .map_err(|error| OwnerServiceFailure::from_io("owner_record_shape", &error))?;
        config::executable(&record.wrapper)
            .map_err(|error| OwnerServiceFailure::from_io("owner_record_shape", &error))?;
        let properties = unit_properties(&owner.unit)
            .await
            .map_err(|code| OwnerServiceFailure::new("owner_service_properties", code))?;
        let pid = service_pid(&properties)
            .map_err(|code| OwnerServiceFailure::new("owner_service_pid", code))?;
        let service = herdr::process_evidence(pid)
            .map_err(|error| OwnerServiceFailure::from_io("owner_service_kernel", &error))?;
        if record.service.as_ref() != Some(&service) {
            return Err(OwnerServiceFailure::new(
                "owner_service_identity",
                ErrorCode::NotReady,
            ));
        }
        if properties.get("WorkingDirectory") != record.cwd.to_str().map(|s| s.to_owned()).as_ref()
        {
            return Err(OwnerServiceFailure::new(
                "owner_service_cwd",
                ErrorCode::NotReady,
            ));
        }
        let expected = owner_launcher_argv(&record.launcher, &owner_record(owner))
            .map_err(|code| OwnerServiceFailure::new("owner_service_exec_start", code))?;
        exact_service_argv(
            &service_property(&owner.unit, "ExecStart")
                .await
                .map_err(|code| OwnerServiceFailure::new("owner_service_exec_start", code))?,
            &expected,
        )
        .map_err(|code| OwnerServiceFailure::new("owner_service_exec_start", code))?;
        if environment_digest(
            &service_environment(
                &service_property(&owner.unit, "Environment")
                    .await
                    .map_err(|code| OwnerServiceFailure::new("owner_service_environment", code))?,
            )
            .map_err(|code| OwnerServiceFailure::new("owner_service_environment", code))?,
        ) != record.environment_digest
        {
            return Err(OwnerServiceFailure::new(
                "owner_service_environment",
                ErrorCode::NotReady,
            ));
        }
        if manager_process()
            .map_err(|code| OwnerServiceFailure::new("owner_service_identity", code))?
            != record.manager
            || herdr::process_evidence(service.pid)
                .map_err(|error| OwnerServiceFailure::from_io("owner_service_identity", &error))?
                != service
        {
            return Err(OwnerServiceFailure::new(
                "owner_service_identity",
                ErrorCode::StaleSession,
            ));
        }
        Ok((service, properties))
    }
    async fn prove_owner_record(
        &self,
        owner: &CodexOwner,
        record: &OwnerRecord,
        epoch: &OwnerEpoch,
    ) -> SecurityResult<()> {
        if record.epoch.as_ref() != Some(epoch) {
            return Err(ErrorCode::NotReady);
        }
        let (service, _) = self
            .prove_service(owner, record)
            .await
            .map_err(|error| error.code)?;
        let process = epoch
            .validate(&owner.socket_host, &owner.socket_agent)
            .map_err(policy)?;
        process_descends(&process, &service)?;
        let argv = codex_relay::process_arguments(&process).map_err(policy)?;
        if !native_owner_arguments(&argv, &record.argv) {
            return Err(ErrorCode::NotReady);
        }
        use std::os::unix::fs::MetadataExt;
        let cwd = std::fs::metadata(&record.cwd).map_err(policy)?;
        let actual = std::fs::metadata(format!("/proc/{}/cwd", process.pid)).map_err(policy)?;
        let wrapper = std::fs::metadata(&record.wrapper).map_err(policy)?;
        if (cwd.dev(), cwd.ino()) != (actual.dev(), actual.ino())
            || herdr::process_object(&process, &record.cwd).map_err(policy)?
                != (cwd.dev(), cwd.ino())
            || herdr::process_object(&process, &record.wrapper).map_err(policy)?
                != (wrapper.dev(), wrapper.ino())
        {
            return Err(ErrorCode::NotReady);
        }
        let current = unit_properties(&owner.unit).await?;
        if service_pid(&current)? != service.pid
            || herdr::process_evidence(service.pid).map_err(policy)? != service
            || epoch
                .validate(&owner.socket_host, &owner.socket_agent)
                .map_err(policy)?
                != process
        {
            return Err(ErrorCode::StaleSession);
        }
        Ok(())
    }
    async fn prove_owner(&self, owner: &CodexOwner, epoch: &OwnerEpoch) -> SecurityResult<()> {
        let record: OwnerRecord =
            codex_relay::private_read(&owner_record(owner)).map_err(policy)?;
        self.prove_owner_record(owner, &record, epoch).await
    }
    async fn settle_stage(
        &self,
        auth: &Authorized,
        action: &Uuid,
        outcome: CreationOutcome,
    ) -> SecurityResult<Receipt> {
        let proof = auth.proof.clone();
        let id = action.clone();
        let receipt = self
            .security
            .storage
            .call(move |s| s.creation_result(&proof, &id, outcome))
            .await
            .map_err(pairing::store_error)?;
        self.security.state.peer_event(
            &auth.proof,
            EventBody::ActionStatus(ActionStatusEvent {
                receipt: receipt.clone(),
            }),
        )?;
        Ok(receipt)
    }
    async fn owner_once(
        &self,
        auth: &Authorized,
        action: &Uuid,
        params: &SessionCreateParams,
        resources: &CreationResources,
        job_deadline: tokio::time::Instant,
    ) -> SecurityResult<(
        CodexOwner,
        Arc<Link>,
        tokio::sync::mpsc::Receiver<NativeEvent>,
        OwnerEpoch,
    )> {
        let _owner = self.owner_start.lock().await;
        let reporter = self.invalidation_reporter.clone();
        if tokio::time::Instant::now() >= job_deadline || self.stopping.load(Ordering::Acquire) {
            return Err(ErrorCode::Timeout);
        }
        self.creation_available()?;
        self.topology
            .validate_provider_stage(action, params, resources)?;
        let owner = self.topology.config.creation_codex_owner()?.clone();
        let path = owner_record(&owner);
        let existing = std::fs::symlink_metadata(&owner.socket_host);
        if existing.is_ok() {
            let mut record: OwnerRecord = match codex_relay::private_read(&path) {
                Ok(record) => record,
                Err(error) => {
                    if let Some(reporter) = &reporter {
                        reporter.prebinding_error(
                            "creation",
                            "owner_record_proof",
                            policy(&error),
                            &error,
                        );
                    }
                    return Err(policy(error));
                }
            };
            let epoch = match record.epoch.take().ok_or(ErrorCode::NotReady) {
                Ok(epoch) => epoch,
                Err(code) => {
                    if let Some(reporter) = &reporter {
                        reporter.prebinding("creation", "owner_record_proof", code, "none");
                    }
                    return Err(code);
                }
            };
            if let Err(code) = self.prove_owner(&owner, &epoch).await {
                if let Some(reporter) = &reporter {
                    reporter.prebinding("creation", "owner_proof", code, "none");
                }
                return Err(code);
            }
            let (link, events, _) = match Link::connect(&owner, &epoch).await {
                Ok(link) => link,
                Err(error) => {
                    if let Some(reporter) = &reporter {
                        reporter.prebinding_error(
                            "creation",
                            "native_initialize",
                            policy(&error),
                            &error,
                        );
                    }
                    return Err(policy(error));
                }
            };
            let known = resources.clone();
            let mut admitted = known.clone();
            admitted.owner_started = Some(false);
            let metadata = owner_metadata(&owner, &epoch);
            let admitted = self
                .security
                .start_boundary(
                    auth,
                    action.clone(),
                    (CreationStage::Owner, metadata.clone()),
                    || {
                        self.topology
                            .validate_provider_stage(action, params, &known)?;
                        epoch
                            .validate(&owner.socket_host, &owner.socket_agent)
                            .map_err(policy)?;
                        Ok(())
                    },
                    || {
                        Ok((
                            (),
                            stage(
                                admitted,
                                metadata,
                                StageState::Confirmed,
                                Some("Reused exact independently owned live app-server"),
                            ),
                        ))
                    },
                )
                .await;
            if let Err(code) = admitted {
                link.close().await;
                return Err(code);
            }
            return Ok((owner, link, events, epoch));
        } else if let Err(error) = existing
            && error.kind() != io::ErrorKind::NotFound
        {
            if let Some(reporter) = &reporter {
                reporter.prebinding_error("creation", "owner_socket_epoch", policy(&error), &error);
            }
            return Err(ErrorCode::NotReady);
        }
        if std::fs::symlink_metadata(&path).is_ok() {
            if let Some(reporter) = &reporter {
                reporter.prebinding(
                    "creation",
                    "owner_record_proof",
                    ErrorCode::NotReady,
                    "none",
                );
            }
            return Err(ErrorCode::NotReady);
        }
        let owner_properties = match unit_properties(&owner.unit).await {
            Ok(properties) => properties,
            Err(code) => {
                if let Some(reporter) = &reporter {
                    reporter.prebinding("creation", "owner_service_properties", code, "none");
                }
                return Err(code);
            }
        };
        if owner_properties.get("LoadState").map(String::as_str) != Some("not-found") {
            if let Some(reporter) = &reporter {
                reporter.prebinding(
                    "creation",
                    "owner_service_properties",
                    ErrorCode::NotReady,
                    "none",
                );
            }
            return Err(ErrorCode::NotReady);
        }
        let wrapper = self
            .topology
            .config
            .wrapper(Provider::Codex)
            .ok_or(ErrorCode::NotReady)?
            .to_owned();
        let cwd = self
            .topology
            .config
            .project(&params.project_id)?
            .path
            .clone();
        let variables = environment()?;
        let manager = manager_process()?;
        let systemd = installed_command("systemd-run")?;
        let launcher = std::env::current_exe().map_err(policy)?;
        config::executable(&launcher).map_err(policy)?;
        let mut record = OwnerRecord {
            v: 2,
            owner_id: owner.id.clone(),
            action_id: action.clone(),
            unit: owner.unit.clone(),
            launcher: launcher.clone(),
            wrapper: wrapper.clone(),
            cwd: cwd.clone(),
            socket_agent: owner.socket_agent.clone(),
            argv: vec![
                "app-server".into(),
                "--listen".into(),
                format!("unix://{}", owner.socket_agent.display()),
            ],
            environment_digest: environment_digest(&variables),
            manager: manager.clone(),
            service: None,
            epoch: None,
        };
        let mut args = vec![
            "--user".into(),
            "--expand-environment=no".into(),
            format!("--unit={}", owner.unit),
            "--property=Type=exec".into(),
            "--property=UMask=0077".into(),
            "--property=Restart=no".into(),
            format!("--property=WorkingDirectory={}", cwd.display()),
        ];
        for (key, value) in &variables {
            args.push(format!("--setenv={key}={value}"));
        }
        args.extend(owner_launcher_argv(&launcher, &path)?);
        let known = resources.clone();
        let admitted = known.clone();
        let intent = StageMetadata {
            unit_name: Some(owner.unit.clone()),
            socket_path: owner.socket_host.to_str().map(str::to_owned),
            ..StageMetadata::default()
        };
        let spawn_reporter = reporter.clone();
        if let Some(reporter) = &reporter {
            reporter.arm_owner_progress(action);
        }
        let child = match self
            .security
            .start_boundary(
                auth,
                action.clone(),
                (CreationStage::Owner, intent.clone()),
                || {
                    if tokio::time::Instant::now() >= job_deadline {
                        return Err(ErrorCode::Timeout);
                    }
                    self.topology
                        .validate_provider_stage(action, params, &known)?;
                    if manager_process()? != manager
                        || std::fs::symlink_metadata(&owner.socket_host).is_ok()
                        || std::fs::symlink_metadata(&path).is_ok()
                    {
                        return Err(ErrorCode::NotReady);
                    }
                    config::executable(&wrapper).map_err(policy)
                },
                || {
                    if let Err(error) = codex_relay::private_write(&path, &record, false) {
                        if let Some(reporter) = &spawn_reporter {
                            reporter.prebinding_error(
                                "creation",
                                "owner_persist",
                                policy(&error),
                                &error,
                            );
                        }
                        return Err(policy(error));
                    }
                    match std::process::Command::new(&systemd)
                        .args(&args)
                        .current_dir(&cwd)
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .spawn()
                    {
                        Ok(child) => Ok((
                            Some(child),
                            stage(
                                admitted,
                                intent,
                                StageState::Dispatching,
                                Some("Independent owner service admission process started"),
                            ),
                        )),
                        Err(error) => {
                            if let Some(reporter) = &spawn_reporter {
                                reporter.prebinding_error(
                                    "creation",
                                    "owner_spawn",
                                    policy(&error),
                                    &error,
                                );
                            }
                            Ok((None, stage(admitted, intent, StageState::Uncertain, None)))
                        }
                    }
                },
            )
            .await
        {
            Ok(child) => {
                if let Some(reporter) = &reporter {
                    reporter.owner_progress(action, "owner_start_settled");
                }
                child
            }
            Err(code) => {
                if let Some(reporter) = &reporter {
                    reporter.prebinding("creation", "owner_spawn", code, "none");
                }
                return Err(code);
            }
        };
        let Some(child) = child else {
            if let Some(reporter) = &reporter {
                reporter.prebinding("creation", "owner_spawn", ErrorCode::NotReady, "none");
            }
            return Err(ErrorCode::NotReady);
        };
        let child_status = child_status(child).await;
        if let Some(reporter) = &reporter {
            reporter.owner_progress(action, "owner_child_status_returned");
        }
        let child_running = match child_status {
            Ok(running) => running,
            Err(code) => {
                if let Some(reporter) = &reporter {
                    reporter.prebinding("creation", "owner_wait", code, "none");
                }
                return Err(code);
            }
        };
        if !child_running {
            if let Some(reporter) = &reporter {
                reporter.prebinding("creation", "owner_exit", ErrorCode::NotReady, "none");
            }
            self.settle_stage(
                auth,
                action,
                stage(
                    resources.clone(),
                    StageMetadata::default(),
                    StageState::Uncertain,
                    None,
                ),
            )
            .await?;
            return Err(ErrorCode::NotReady);
        }
        // Owner startup shares the original absolute creation budget: the verified wrapper may
        // take longer than ten seconds before its publisher process is observable.
        let deadline = job_deadline;
        let mut native = None;
        loop {
            if tokio::time::Instant::now() >= deadline {
                if let Some(reporter) = &reporter {
                    reporter.prebinding("creation", "owner_wait", ErrorCode::Timeout, "none");
                }
                return Err(ErrorCode::Timeout);
            }
            {
                let _guard = match self.security.native_guard(auth).await {
                    Ok(guard) => guard,
                    Err(code) => {
                        if let Some(reporter) = &reporter {
                            reporter.prebinding("creation", "owner_authorization", code, "none");
                        }
                        return Err(code);
                    }
                };
            }
            if let Err(code) = self
                .topology
                .validate_provider_stage(action, params, resources)
            {
                if let Some(reporter) = &reporter {
                    reporter.prebinding("creation", "owner_proof", code, "none");
                }
                return Err(code);
            }
            if record.service.is_none() {
                let properties = match unit_properties(&owner.unit).await {
                    Ok(properties) => properties,
                    Err(code) => {
                        if let Some(reporter) = &reporter {
                            reporter.prebinding(
                                "creation",
                                "owner_service_properties",
                                code,
                                "none",
                            );
                        }
                        return Err(code);
                    }
                };
                let pid = match service_pid(&properties) {
                    Ok(pid) => pid,
                    Err(code) => {
                        if let Some(reporter) = &reporter {
                            reporter.prebinding("creation", "owner_service_pid", code, "none");
                        }
                        return Err(code);
                    }
                };
                record.service = match herdr::process_evidence(pid) {
                    Ok(service) => Some(service),
                    Err(error) => {
                        if let Some(reporter) = &reporter {
                            reporter.prebinding_error(
                                "creation",
                                "owner_service_kernel",
                                policy(&error),
                                &error,
                            );
                        }
                        return Err(policy(error));
                    }
                };
            }
            let (service, properties) = match self.prove_service(&owner, &record).await {
                Ok(service) => service,
                Err(error) => {
                    if let Some(reporter) = &reporter {
                        reporter.prebinding("creation", error.guard, error.code, error.io);
                    }
                    return Err(error.code);
                }
            };
            if tokio::time::Instant::now() >= deadline {
                if let Some(reporter) = &reporter {
                    reporter.prebinding("creation", "owner_wait", ErrorCode::Timeout, "none");
                }
                return Err(ErrorCode::Timeout);
            }
            if native.is_none() {
                let proof_properties = properties.clone();
                let proof_service = service.clone();
                let proof_argv = record.argv.clone();
                if tokio::time::Instant::now() >= deadline {
                    if let Some(reporter) = &reporter {
                        reporter.prebinding("creation", "owner_wait", ErrorCode::Timeout, "none");
                    }
                    return Err(ErrorCode::Timeout);
                }
                // Distribution hashing is a pure proof; cancellation leaves no effect to replay.
                let proof = tokio::task::spawn_blocking(move || {
                    owned_native_process(&proof_properties, &proof_service, &proof_argv)
                });
                let proof = match tokio::time::timeout_at(deadline, proof).await {
                    Ok(proof) => proof,
                    Err(_) => {
                        if let Some(reporter) = &reporter {
                            reporter.prebinding("creation", "owner_wait", ErrorCode::Timeout, "none");
                        }
                        return Err(ErrorCode::Timeout);
                    }
                };
                if tokio::time::Instant::now() >= deadline {
                    if let Some(reporter) = &reporter {
                        reporter.prebinding("creation", "owner_wait", ErrorCode::Timeout, "none");
                    }
                    return Err(ErrorCode::Timeout);
                }
                let proof = match proof {
                    Ok(proof) => proof,
                    Err(_) => {
                        if let Some(reporter) = &reporter {
                            reporter.prebinding("creation", "owner_native_process", ErrorCode::NotReady, "none");
                        }
                        return Err(ErrorCode::NotReady);
                    }
                };
                match proof {
                    Ok(process) => native = Some(process),
                    Err(ErrorCode::NotReady) if tokio::time::Instant::now() < deadline => {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        continue;
                    }
                    Err(code) => {
                        if let Some(reporter) = &reporter {
                            reporter.prebinding("creation", "owner_native_process", code, "none");
                        }
                        return Err(code);
                    }
                }
            }
            let process = native.as_ref().ok_or(ErrorCode::NotReady)?;
            let current_process = match herdr::process_evidence(process.pid) {
                Ok(current) => current,
                Err(error) => {
                    if let Some(reporter) = &reporter {
                        reporter.prebinding_error(
                            "creation",
                            "owner_native_process",
                            policy(&error),
                            &error,
                        );
                    }
                    return Err(policy(error));
                }
            };
            if current_process != *process {
                if let Some(reporter) = &reporter {
                    reporter.prebinding(
                        "creation",
                        "owner_service_identity",
                        ErrorCode::StaleSession,
                        "none",
                    );
                }
                return Err(ErrorCode::StaleSession);
            }
            if record.epoch.is_none() {
                match OwnerEpoch::new(process, &owner.socket_host, &owner.socket_agent) {
                    Ok(epoch) => record.epoch = Some(epoch),
                    Err(error)
                        if error.kind() == io::ErrorKind::NotFound
                            && tokio::time::Instant::now() < deadline =>
                    {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        continue;
                    }
                    Err(error) => {
                        if let Some(reporter) = &reporter {
                            reporter.prebinding_error(
                                "creation",
                                "owner_socket_epoch",
                                policy(&error),
                                &error,
                            );
                        }
                        return Err(policy(error));
                    }
                }
            }
            let epoch = match record.epoch.as_ref().ok_or(ErrorCode::NotReady) {
                Ok(epoch) => epoch,
                Err(code) => {
                    if let Some(reporter) = &reporter {
                        reporter.prebinding("creation", "owner_socket_epoch", code, "none");
                    }
                    return Err(code);
                }
            };
            if let Err(code) = self.prove_owner_record(&owner, &record, epoch).await {
                if let Some(reporter) = &reporter {
                    reporter.prebinding("creation", "owner_record_proof", code, "none");
                }
                return Err(code);
            }
            if tokio::time::Instant::now() >= deadline {
                if let Some(reporter) = &reporter {
                    reporter.prebinding("creation", "owner_wait", ErrorCode::Timeout, "none");
                }
                return Err(ErrorCode::Timeout);
            }
            {
                let _guard = match self.security.native_guard(auth).await {
                    Ok(guard) => guard,
                    Err(code) => {
                        if let Some(reporter) = &reporter {
                            reporter.prebinding("creation", "owner_authorization", code, "none");
                        }
                        return Err(code);
                    }
                };
                if let Err(code) = self
                    .topology
                    .validate_provider_stage(action, params, resources)
                {
                    if let Some(reporter) = &reporter {
                        reporter.prebinding("creation", "owner_proof", code, "none");
                    }
                    return Err(code);
                }
            }
            if tokio::time::Instant::now() >= deadline {
                if let Some(reporter) = &reporter {
                    reporter.prebinding("creation", "owner_wait", ErrorCode::Timeout, "none");
                }
                return Err(ErrorCode::Timeout);
            }
            if let Some(reporter) = &reporter {
                reporter.owner_progress(action, "owner_native_initialize_entered");
            }
            match tokio::time::timeout_at(deadline, Link::connect(&owner, epoch)).await {
                Ok(Ok((link, events, _))) => {
                    if tokio::time::Instant::now() >= deadline {
                        link.close().await;
                        if let Some(reporter) = &reporter {
                            reporter.prebinding(
                                "creation",
                                "native_initialize",
                                ErrorCode::Timeout,
                                "none",
                            );
                        }
                        return Err(ErrorCode::Timeout);
                    }
                    let verified_reporter = reporter.clone();
                    let verified = async {
                        // Only successful native initialization completes the saved reusable proof.
                        if let Err(error) = codex_relay::private_write(&path, &record, true) {
                            if let Some(reporter) = &verified_reporter {
                                reporter.prebinding_error("creation", "owner_persist", policy(&error), &error);
                            }
                            return Err(policy(error));
                        }
                        let mut known = resources.clone();
                        known.owner_started = Some(true);
                        self.settle_stage(
                            auth, action,
                            stage(
                                known.clone(), owner_metadata(&owner, epoch), StageState::Dispatching,
                                Some("Actual connected owner process observed; readiness/provenance pending"),
                            ),
                        ).await?;
                        if let Err(code) = self.prove_owner(&owner, epoch).await {
                            if let Some(reporter) = &verified_reporter {
                                reporter.prebinding("creation", "owner_proof", code, "none");
                            }
                            return Err(code);
                        }
                        {
                            let _guard = match self.security.native_guard(auth).await {
                                Ok(guard) => guard,
                                Err(code) => {
                                    if let Some(reporter) = &verified_reporter {
                                        reporter.prebinding("creation", "owner_authorization", code, "none");
                                    }
                                    return Err(code);
                                }
                            };
                            if let Err(code) = self.topology.validate_provider_stage(action, params, resources) {
                                if let Some(reporter) = &verified_reporter {
                                    reporter.prebinding("creation", "owner_proof", code, "none");
                                }
                                return Err(code);
                            }
                        }
                        if tokio::time::Instant::now() >= deadline {
                            if let Some(reporter) = &verified_reporter {
                                reporter.prebinding(
                                    "creation",
                                    "owner_wait",
                                    ErrorCode::Timeout,
                                    "none",
                                );
                            }
                            return Err(ErrorCode::Timeout);
                        }
                        self.settle_stage(
                            auth, action,
                            stage(
                                known, owner_metadata(&owner, epoch), StageState::Confirmed,
                                Some("Exact owner service/kernel/socket/native initialize verified"),
                            ),
                        ).await?;
                        Ok::<_, ErrorCode>(())
                    }.await;
                    if let Err(code) = verified {
                        link.close().await;
                        return Err(code);
                    }
                    return Ok((
                        owner,
                        link,
                        events,
                        record.epoch.take().ok_or(ErrorCode::NotReady)?,
                    ));
                }
                Ok(Err(error))
                    if matches!(
                        error.kind(),
                        io::ErrorKind::NotFound
                            | io::ErrorKind::ConnectionRefused
                            | io::ErrorKind::TimedOut
                    ) && tokio::time::Instant::now() < deadline =>
                {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                Ok(Err(error)) => {
                    if let Some(reporter) = &reporter {
                        reporter.prebinding_error(
                            "creation",
                            "native_initialize",
                            policy(&error),
                            &error,
                        );
                    }
                    return Err(policy(error));
                }
                Err(_) => {
                    if let Some(reporter) = &reporter {
                        reporter.prebinding(
                            "creation",
                            "native_initialize",
                            ErrorCode::Timeout,
                            "none",
                        );
                    }
                    return Err(ErrorCode::Timeout);
                }
            }
        }
    }
    async fn creation_job(
        &self,
        auth: &Authorized,
        action: &Uuid,
        params: &SessionCreateParams,
        deadline: tokio::time::Instant,
    ) -> SecurityResult<()> {
        if tokio::time::Instant::now() >= deadline || self.stopping.load(Ordering::Acquire) {
            return Err(ErrorCode::Timeout);
        }
        let workspace = self
            .topology
            .workspace_once(&self.security, auth, action, params)
            .await?;
        if workspace.state != ReceiptState::Dispatching
            || workspace
                .creation
                .as_ref()
                .is_none_or(|p| p.stage_state != StageState::Confirmed)
        {
            return Ok(());
        }
        let probe = self
            .topology
            .shell_probe_once(&self.security, auth, action, params, deadline)
            .await?;
        if probe.state != ReceiptState::Dispatching
            || probe
                .creation
                .as_ref()
                .is_none_or(|p| p.stage_state != StageState::Confirmed)
        {
            return Ok(());
        }
        let mut resources = probe.creation.ok_or(ErrorCode::NotReady)?.resources;
        let (owner, link, mut events, epoch) = match self
            .owner_once(auth, action, params, &resources, deadline)
            .await
        {
            Ok(owner) => owner,
            Err(code) => {
                if let Some(reporter) = &self.invalidation_reporter {
                    reporter.prebinding("creation", "owner_or_native", code, "none");
                }
                return Err(code);
            }
        };
        let result=async{
            resources=self.receipt(auth,action).await?.creation.ok_or(ErrorCode::NotReady)?.resources;
            if tokio::time::Instant::now()>=deadline {
                if let Some(reporter)=&self.invalidation_reporter {
                    reporter.prebinding("creation","pre_thread_deadline_before_reproof",ErrorCode::Timeout,"none");
                }
                return Err(ErrorCode::Timeout)
            }
            if let Err(code)=self.prove_owner(&owner,&epoch).await {
                if let Some(reporter)=&self.invalidation_reporter {
                    reporter.prebinding("creation","pre_thread_owner_reproof",code,"none");
                }
                return Err(code)
            }
            if tokio::time::Instant::now()>=deadline {
                if let Some(reporter)=&self.invalidation_reporter {
                    reporter.prebinding("creation","pre_thread_deadline_after_reproof",ErrorCode::Timeout,"none");
                }
                return Err(ErrorCode::Timeout)
            }
            if self.stopping.load(Ordering::Acquire) {
                if let Some(reporter)=&self.invalidation_reporter {
                    reporter.prebinding("creation","pre_thread_stopping",ErrorCode::Timeout,"none");
                }
                return Err(ErrorCode::Timeout)
            }
            let thread_source = self.topology.source(&params.server_id)?;
            let fd=link.fd.as_fd().try_clone_to_owned().map_err(policy)?;let known=resources.clone();let metadata=owner_metadata(&owner,&epoch);
            let thread_start = self.invalidation_reporter.as_ref().map(|reporter| reporter.thread_start_observation());
            let thread_start_window = ThreadStartWindowGuard(thread_start.clone());
            let id=Uuid::random().0;
            let rx = if let Some(observation) = &thread_start {
                link.callback_with_thread_start(NativeRpcId::String(id.clone()), false, observation.clone())?
            } else {
                link.callback(NativeRpcId::String(id.clone()), false)?
            };
            let execution_directory=self.topology.execution_directory(action,params)?;
            let bytes=serde_json::to_vec(&json!({"id":id,"method":"thread/start","params":{"cwd":execution_directory,"serviceName":"goatr"}})).map_err(policy)?;
            let validator_resources = known.clone();
            let (native_owner, native_epoch, native_link, native_source) = (&owner, &epoch, &link, &thread_source);
            let reporter = self.invalidation_reporter.clone();
            self.security.thread_creation_boundary(
                auth, action.clone(), metadata.clone(), deadline, self.thread_stop.subscribe(),
                || {
                    self.topology.validate_provider_stage(action, params, &validator_resources)?;
                    let current = self.topology.source(&params.server_id)?;
                    if current.incarnation != thread_source.incarnation || current.evidence != thread_source.evidence {
                        return Err(ErrorCode::StaleSession);
                    }
                    let process=epoch.validate(&owner.socket_host, &owner.socket_agent).map_err(policy)?;
                    if params.checkout.is_some() {
                        let current=self.topology.execution_directory(action,params)?;
                        use std::os::unix::fs::MetadataExt;
                        let metadata=std::fs::metadata(&current).map_err(policy)?;
                        if current != execution_directory || herdr::process_object(&process,&current).map_err(policy)? != (metadata.dev(),metadata.ino()) {
                            return Err(ErrorCode::StaleSession);
                        }
                    }
                    if tokio::time::Instant::now() >= deadline || self.stopping.load(Ordering::Acquire) {
                        return Err(ErrorCode::Timeout);
                    }
                    Ok(())
                },
                fd,
                |checkpoint| async move {
                    let effect_entered = thread_start.as_ref().map(|_| std::time::Instant::now());
                    let mut effect = ThreadEffectDrop::new(
                        reporter.clone(),
                        native_link.reader_capture.clone(),
                        thread_start,
                    );
                    let outcome = async {
                    effect.phase("thread_effect_drop_start_send");
                    if let Err(code) = native_link.send(bytes).await {
                        if let Some(reporter) = &reporter { reporter.prebinding("thread", "thread_start_send", code, "none"); }
                        return thread_uncertain(known, metadata, "thread_start_send", code);
                    }
                    if let (Some(observation), Some(entered)) = (&effect.thread_start, effect_entered) {
                        observation.start_send_elapsed(entered.elapsed());
                    }
                    effect.phase("thread_effect_drop_start_reply");
                    let value = match rx.await {
                        Ok(Ok(value)) => value,
                        Ok(Err(ErrorCode::BackendError)) => return stage(
                            known, metadata, StageState::Failed, Some("Native thread/start rejected"),
                        ),
                        Ok(Err(code)) => { if let Some(reporter) = &reporter { reporter.prebinding("thread", "thread_start_reply", code, "none"); } return thread_uncertain(known, metadata, "thread_start_reply", code) },
                        Err(_) => { if let Some(reporter) = &reporter { reporter.prebinding("thread", "thread_start_callback_lost", ErrorCode::NotReady, "none"); } return thread_uncertain(known, metadata, "thread_start_callback_lost", ErrorCode::NotReady) },
                    };
                    let thread = match native_id(&value["thread"], "id") {
                        Ok(thread) => thread,
                        Err(code) => { if let Some(reporter) = &reporter { reporter.prebinding("thread", "thread_start_id", code, "none"); } return thread_uncertain(known, metadata, "thread_start_id", code) },
                    };
                    let mut known = known;
                    known.native_id = Some(thread.clone());
                    effect.phase("thread_effect_drop_acknowledge");
                    if let Err(code) = checkpoint.acknowledge(thread.clone()).await {
                        if let Some(reporter) = &reporter { reporter.prebinding("thread", "thread_start_acknowledge", code, "none"); }
                        return thread_uncertain(known, metadata, "thread_start_acknowledge", code);
                    }
                    if let Err(reason) = paginated_zero_turn_thread(&value["thread"], &thread) {
                        return stage(known, metadata, StageState::Failed, Some(reason));
                    }
                    let named = async {
                        effect.phase("thread_effect_drop_owner_proof");
                        self.prove_owner(native_owner, native_epoch).await?;
                        effect.phase("thread_effect_drop_checkpoint_guard");
                        let guard = checkpoint.guard().await?;
                        if tokio::time::Instant::now() >= deadline || self.stopping.load(Ordering::Acquire) {
                            return Err(ErrorCode::Timeout);
                        }
                        self.topology.validate_provider_stage(action, params, &known)?;
                        let current = self.topology.source(&params.server_id)?;
                        if current.incarnation != native_source.incarnation || current.evidence != native_source.evidence {
                            return Err(ErrorCode::StaleSession);
                        }
                        native_source.recheck().map_err(policy)?;
                        native_epoch.validate(&native_owner.socket_host, &native_owner.socket_agent).map_err(policy)?;
                        let id = Uuid::random().0;
                        let rx = native_link.callback(NativeRpcId::String(id.clone()), false)?;
                        let bytes = serde_json::to_vec(&json!({
                            "id": id, "method": "thread/name/set",
                            "params": {"threadId": thread.0, "name": params.name.0},
                        })).map_err(policy)?;
                        effect.phase("thread_effect_drop_title_send");
                        native_link.send(bytes).await?;
                        drop(guard);
                        effect.phase("thread_effect_drop_title_reply");
                        let result = rx.await.map_err(|_| ErrorCode::NotReady)??;
                        native_name_set_admission(&result)
                    }.await;
                    match named {
                        Ok(()) => {}
                        Err(code) => {
                            return if code == ErrorCode::BackendError {
                                stage(known, metadata, StageState::Failed,
                                    Some("Native original-thread title checkpoint rejected; no read or fallback"))
                            } else { thread_uncertain(known, metadata, "thread_title_checkpoint", code) };
                        }
                    }
                    let mut read_rejection = None;
                    let read = async {
                        effect.phase("thread_effect_drop_owner_proof");
                        self.prove_owner(native_owner, native_epoch).await?;
                        effect.phase("thread_effect_drop_checkpoint_guard");
                        let guard = checkpoint.guard().await?;
                        if tokio::time::Instant::now() >= deadline || self.stopping.load(Ordering::Acquire) {
                            return Err(ErrorCode::Timeout);
                        }
                        self.topology.validate_provider_stage(action, params, &known)?;
                        let current = self.topology.source(&params.server_id)?;
                        if current.incarnation != native_source.incarnation || current.evidence != native_source.evidence {
                            return Err(ErrorCode::StaleSession);
                        }
                        native_source.recheck().map_err(policy)?;
                        native_epoch.validate(&native_owner.socket_host, &native_owner.socket_agent).map_err(policy)?;
                        let id = Uuid::random().0;
                        let (rejection_tx, mut rejection_rx) = oneshot::channel();
                        let rx = native_link.callback_with_read_rejection(
                            NativeRpcId::String(id.clone()), false, Some((thread.clone(), rejection_tx)),
                        )?;
                        let bytes = serde_json::to_vec(&json!({
                            "id": id, "method": "thread/read",
                            "params": {"threadId": thread.0, "includeTurns": true},
                        })).map_err(policy)?;
                        effect.phase("thread_effect_drop_read_send");
                        native_link.send(bytes).await?;
                        drop(guard);
                        effect.phase("thread_effect_drop_read_reply");
                        let result = rx.await.map_err(|_| ErrorCode::NotReady)?;
                        read_rejection = rejection_rx.try_recv().ok();
                        result
                    }.await;
                    let value = match read {
                        Ok(value) => value,
                        Err(ErrorCode::BackendError) => {
                            let proof = read_rejection.map(|rejection| rejection.proof()).unwrap_or_else(|| {
                                "Native original-thread materializing read rejected; no fallback".into()
                            });
                            return stage(known, metadata, StageState::Failed, Some(&proof));
                        }
                        Err(code) => {
                            return thread_uncertain(known, metadata, "thread_materializing_read", code);
                        }
                    };
                    if let Err(reason) = paginated_zero_turn_thread(&value["thread"], &thread) {
                        return stage(known, metadata, StageState::Failed, Some(reason));
                    }
                    let current = async {
                        effect.phase("thread_effect_drop_final_proof");
                        self.prove_owner(native_owner, native_epoch).await?;
                        effect.phase("thread_effect_drop_checkpoint_guard");
                        let _guard = checkpoint.guard().await?;
                        self.topology.validate_provider_stage(action, params, &known)?;
                        let current = self.topology.source(&params.server_id)?;
                        if current.incarnation != native_source.incarnation || current.evidence != native_source.evidence {
                            return Err(ErrorCode::StaleSession);
                        }
                        native_source.recheck().map_err(policy)?;
                        native_epoch.validate(&native_owner.socket_host, &native_owner.socket_agent).map_err(policy)?;
                        Ok::<_, ErrorCode>(())
                    }.await;
                    if let Err(code) = current {
                        return thread_uncertain(known, metadata, "thread_final_scope", code);
                    }
                    stage(
                        known, metadata, StageState::Confirmed,
                        Some("Correlated original paginated thread/start, empty native title acknowledgement and same-link zero-turn read"),
                    )
                    }.await;
                    effect.disarm();
                    outcome
                },
            ).await?;
            drop(thread_start_window);
            resources=self.receipt(auth,action).await?.creation.ok_or(ErrorCode::NotReady)?.resources;
            let progress = self.receipt(auth, action).await?.creation.ok_or(ErrorCode::NotReady)?;
            if progress.stage != CreationStage::Thread || progress.stage_state != StageState::Confirmed {
                return if progress.stage_state == StageState::Failed { Err(ErrorCode::BackendError) } else { Ok(()) };
            }
            let thread=resources.native_id.clone().ok_or(ErrorCode::NotReady)?;
            if !link.loaded(&thread).await?{return Err(ErrorCode::NotReady)}
            let source=self.topology.source(&params.server_id)?;let proof=auth.proof.clone();let id=action.clone();let shell=self.security.storage.call(move|s|s.creation_probe(&proof,&id)).await.map_err(pairing::store_error)?.shell;
            let binding=Uuid::random();let (listen_host,listen_agent)={
                let mut selected=None;
                for _ in 0..8{let suffix=pairing::hex(&pairing::random32().map_err(policy)?[..6]);let host=self.topology.config.runtime_dir.join(format!("t{suffix}.sock"));let agent=self.topology.config.runtime_dir_agent.join(format!("t{suffix}.sock"));config::socket_path(&host).map_err(policy)?;config::socket_path(&agent).map_err(policy)?;match std::fs::symlink_metadata(&host){Err(e) if e.kind()==io::ErrorKind::NotFound=>{selected=Some((host,agent));break},Ok(_)=>{},Err(_)=>return Err(ErrorCode::NotReady)}}selected.ok_or(ErrorCode::ResourceLimit)?
            };
            config::socket_path(&listen_host).map_err(policy)?;config::socket_path(&listen_agent).map_err(policy)?;
            let manifest=Manifest{v:1,binding_id:binding.clone(),owner_id:owner.id.clone(),upstream_host:owner.socket_host.clone(),upstream_agent:owner.socket_agent.clone(),listen_host,listen_agent,owner_epoch:epoch.clone(),herdr:codex_relay::HerdrBinding{server_id:source.server_id.clone(),incarnation:source.incarnation.clone(),workspace_id:resources.workspace_id.clone().ok_or(ErrorCode::NotReady)?,terminal_id:resources.terminal_id.clone().ok_or(ErrorCode::NotReady)?,pane_id:resources.pane_id.clone().ok_or(ErrorCode::NotReady)?,socket_path:source.socket_path.clone(),evidence:source.evidence.clone(),shell},expected_thread_id:thread.clone(),created_at:Uint(pairing::now_ms())};
            let path=codex_relay::manifest_path(&owner.bindings_dir,&binding);manifest.validate().map_err(policy)?;let unit=format!("goatr-codex-relay-{}.service",binding.0);
            if unit_properties(&unit).await?.get("LoadState").map(String::as_str)!=Some("not-found"){return Err(ErrorCode::NotReady)}
            let systemd=installed_command("systemd-run")?;let binary=std::env::current_exe().map_err(policy)?;config::executable(&binary).map_err(policy)?;
            let known=resources.clone();let mut admitted=known.clone();admitted.binding_id=Some(binding.clone());
            let metadata=StageMetadata{unit_name:Some(unit.clone()),socket_path:manifest.listen_host.to_str().map(str::to_owned),..StageMetadata::default()};
            let mut args=vec!["--user".into(),"--collect".into(),"--expand-environment=no".into(),format!("--unit={unit}"),"--property=Type=exec".into(),"--property=UMask=0077".into(),"--property=Restart=no".into()];
            if self.invalidation_reporter.is_some() {
                args.push("--property=StandardError=journal".into());
                args.push(format!("--setenv=GOATR_OWNED_CODEX_TERMINAL_CAPTURE={}",binding.0));
            }
            args.extend([binary.to_str().ok_or(ErrorCode::NotReady)?.into(),"codex-relay".into(),"--binding".into(),path.to_str().ok_or(ErrorCode::NotReady)?.into()]);
            let child=self.security.start_boundary(auth,action.clone(),(CreationStage::Relay,metadata.clone()),||{if tokio::time::Instant::now()>=deadline{return Err(ErrorCode::Timeout)}self.topology.validate_provider_stage(action,params,&known)?;manifest.validate().map_err(policy)?;if std::fs::symlink_metadata(&manifest.listen_host).is_ok(){return Err(ErrorCode::NotReady)}Ok(())},||{
                codex_relay::private_write(&path,&manifest,false).map_err(policy)?;
                match std::process::Command::new(&systemd).args(&args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn(){
                    Ok(child)=>Ok((Some(child),stage(admitted,metadata,StageState::Dispatching,Some("Independent same-binary relay service admission started")))),Err(_)=>Ok((None,stage(admitted,metadata,StageState::Uncertain,None))),
                }
            }).await?;
            let Some(child)=child else{return Err(ErrorCode::NotReady)};
            match child_status(child).await {
                Ok(true) => {}
                Ok(false) => return Err(ErrorCode::NotReady),
                Err(ErrorCode::Timeout) => {
                    let mut known = resources.clone();
                    known.binding_id = Some(binding.clone());
                    let gate_proof = launch_failure_proof("relay_child_status_timeout", ErrorCode::Timeout, None);
                    // Diagnostic settlement must not replace the original timeout.
                    let _ = self.settle_stage(auth, action, stage(
                        known,
                        StageMetadata { unit_name: Some(unit.clone()), socket_path: manifest.listen_host.to_str().map(str::to_owned), ..StageMetadata::default() },
                        StageState::Dispatching,
                        Some(&gate_proof),
                    )).await;
                    return Err(ErrorCode::Timeout)
                }
                Err(code) => return Err(code),
            }
            resources=self.receipt(auth,action).await?.creation.ok_or(ErrorCode::NotReady)?.resources;
            let mut last_presence_failure = None;
            let ready = async {
                loop {
                    if tokio::time::Instant::now() >= deadline || self.stopping.load(Ordering::Acquire) {
                        return Ok(None);
                    }
                    {
                        let _guard = self.security.native_guard(auth).await?;
                        self.topology.validate_provider_stage(action, params, &resources)?;
                    }
                    if tokio::time::Instant::now() >= deadline || self.stopping.load(Ordering::Acquire) {
                        return Ok(None);
                    }
                    let presence = match manifest.presence_checked(&path) {
                        Ok(presence) => {
                            last_presence_failure = None;
                            Some(presence)
                        }
                        Err(failure) => {
                            last_presence_failure = Some(failure);
                            None
                        }
                    };
                    if let Some(p) = presence && p.state == codex_relay::PresenceState::Ready {
                        let properties = unit_properties(&unit).await?;
                        if tokio::time::Instant::now() >= deadline || self.stopping.load(Ordering::Acquire) {
                            return Ok(None);
                        }
                        {
                            let _guard = self.security.native_guard(auth).await?;
                            self.topology.validate_provider_stage(action, params, &resources)?;
                        }
                        if tokio::time::Instant::now() >= deadline || self.stopping.load(Ordering::Acquire) {
                            return Ok(None);
                        }
                        if service_pid(&properties)? != p.relay.pid {
                            return Err(ErrorCode::NotReady);
                        }
                        let exec_start = service_property(&unit, "ExecStart").await?;
                        if tokio::time::Instant::now() >= deadline || self.stopping.load(Ordering::Acquire) {
                            return Ok(None);
                        }
                        {
                            let _guard = self.security.native_guard(auth).await?;
                            self.topology.validate_provider_stage(action, params, &resources)?;
                        }
                        exact_service_argv(&exec_start, &[
                            binary.to_str().ok_or(ErrorCode::NotReady)?.into(),
                            "codex-relay".into(), "--binding".into(),
                            path.to_str().ok_or(ErrorCode::NotReady)?.into(),
                        ])?;
                        if tokio::time::Instant::now() >= deadline || self.stopping.load(Ordering::Acquire) {
                            return Ok(None);
                        }
                        return Ok::<_, ErrorCode>(Some(p));
                    }
                    while events.try_recv().is_ok() {
                        if tokio::time::Instant::now() >= deadline || self.stopping.load(Ordering::Acquire) {
                            return Ok(None);
                        }
                    }
                    tokio::time::sleep_until(
                        deadline.min(tokio::time::Instant::now() + Duration::from_millis(50)),
                    ).await;
                }
            }.await?;
            let Some(p) = ready else {
                let failure = last_presence_failure.as_ref();
                let detail = failure.map(|failure| presence_failure_detail(&failure.error));
                let mut gate_proof = launch_failure_proof("relay_initial_ready_timeout", ErrorCode::Timeout, detail);
                if let Some(failure) = failure {
                    gate_proof.push_str("; presence_phase=");
                    gate_proof.push_str(failure.phase.as_str());
                }
                let _ = self.settle_stage(auth, action, stage(
                    resources.clone(),
                    StageMetadata { unit_name: Some(unit.clone()), socket_path: manifest.listen_host.to_str().map(str::to_owned), ..StageMetadata::default() },
                    StageState::Dispatching,
                    Some(&gate_proof),
                )).await;
                return Err(ErrorCode::Timeout)
            };
            resources.relay_started = Some(true);
            self.settle_stage(auth, action, stage(
                resources.clone(),
                StageMetadata {
                    linux_boot_id: Some(p.relay.linux_boot_id.clone()),
                    pid: Some(p.relay.pid), pid_start_ticks: Some(p.relay.start_ticks),
                    unit_name: Some(unit.clone()),
                    socket_path: manifest.listen_host.to_str().map(str::to_owned),
                    ..StageMetadata::default()
                },
                StageState::Confirmed,
                Some("Exact independent relay ready; no TUI slot consumed"),
            )).await?;
            if tokio::time::Instant::now() >= deadline || self.stopping.load(Ordering::Acquire) {
                return Err(ErrorCode::Timeout);
            }
            {
                let _guard = self.security.native_guard(auth).await?;
                self.topology.validate_provider_stage(action, params, &resources)?;
            }
            if tokio::time::Instant::now() >= deadline || self.stopping.load(Ordering::Acquire) {
                return Err(ErrorCode::Timeout);
            }
            let launched=self.topology.launch_once(&self.security,auth,action,params,vec!["-c".into(),"check_for_update_on_startup=false".into(),"--remote".into(),format!("unix://{}",manifest.listen_agent.display()),"resume".into(),thread.0.clone()]).await?;
            if launched.state!=ReceiptState::Dispatching||launched.creation.as_ref().is_none_or(|p|p.stage!=CreationStage::Launch||p.stage_state!=StageState::Dispatching){return Ok(())}
            let mut pinned_connection: Option<Arc<Connection>> = None;
            let reporter = self.invalidation_reporter.clone();
            let mut last_pending = "unobserved";
            loop{
                let mut presence_diagnostic = None;
                let mut presence_phase = None;
                let admission = async {
                    if tokio::time::Instant::now()>=deadline{report_launch_pending(&reporter,false,last_pending);return Err((None, ErrorCode::Timeout))}
                    if self.stopping.load(Ordering::Acquire){report_launch_pending(&reporter,true,last_pending);return Err((None, ErrorCode::Timeout))}
                    {let _guard=self.security.native_guard(auth).await.map_err(|code| (Some("original_guard"), code))?;}
                    if tokio::time::Instant::now()>=deadline{report_launch_pending(&reporter,false,last_pending);return Err((None, ErrorCode::Timeout))}
                    if self.stopping.load(Ordering::Acquire){report_launch_pending(&reporter,true,last_pending);return Err((None, ErrorCode::Timeout))}
                    let source = match self.topology.source_readiness(&params.server_id)
                        .map_err(|code| (Some("topology_source"), code))?
                    {
                        Some(source) => source,
                        None => {
                            if let Some(connection) = &pinned_connection {
                                let _ = self.creation_connection_ready(connection)
                                    .map_err(|code| (Some("connection_validate"), code))?;
                            }
                            last_pending="source_unavailable";return Ok(None)
                        }
                    };
                    if source.incarnation!=manifest.herdr.incarnation||source.evidence!=manifest.herdr.evidence{return Err((Some("topology_source"), ErrorCode::StaleSession))}
                    let p={
                        let _observation=pinned_connection.as_ref().map(|connection| connection.observation.lock());
                        let observed = manifest.validated_presence(&path);
                        let observed = match observed {
                        Ok(presence) if pinned_connection.is_none() => {
                            strict_unpinned_presence(presence)
                                .map_err(|code| (Some("manifest_presence"), code))?
                        }
                        Ok(presence) => presence,
                        Err(failure) if presence_publication_pending(&failure) => {
                            if let Some(connection) = &pinned_connection {
                                self.security.state.observation_pending(&connection.target, &connection.epoch)
                                    .map_err(|code| (Some("connection_validate"), code))?;
                            }
                            last_pending="publication_pending";return Ok(None);
                        }
                        Err(failure) => {
                            presence_diagnostic = Some(presence_failure_detail(&failure.error));
                            presence_phase = Some(failure.phase);
                            return Err((Some("manifest_presence"), policy(failure.error)));
                        }
                        };
                        if let Some(connection) = &pinned_connection {
                            let decision = connection.observe_verified_presence(
                                &self.security.state, &manifest, Some(&observed)
                            ).map_err(|code| (Some("connection_validate"), code))?;
                            if unavailable_resume_rejected(&observed, &thread) {
                                drop(_observation);
                                self.settle_stage(auth,action,stage(resources.clone(),owner_metadata(&owner,&epoch),StageState::Failed,Some("Stock exact TUI resume rejected; zero-turn rollout may be unmaterialized; no warm-up/substitute"))).await.map_err(|code| (None, code))?;
                                return Err((None, ErrorCode::BackendError))
                            }
                            if !decision.final_freshness(pairing::now_ms())
                                .map_err(|failure| (Some("manifest_presence"), policy(failure.error)))?
                            {
                                if decision.is_fresh() {
                                    self.security.state.observation_pending(&connection.target, &connection.epoch)
                                        .map_err(|code| (Some("connection_validate"), code))?;
                                }
                                last_pending="presence_unavailable";return Ok(None)
                            }
                        }
                        match observed {
                            codex_relay::ValidatedPresence::Fresh(presence) => presence,
                            codex_relay::ValidatedPresence::Past(_) => {last_pending="presence_past";return Ok(None)},
                            codex_relay::ValidatedPresence::Unavailable(_) => {last_pending="presence_unavailable";return Ok(None)},
                        }
                    };
                    if p.resume_rejected_thread.as_ref()==Some(&thread){
                        self.settle_stage(auth,action,stage(resources.clone(),owner_metadata(&owner,&epoch),StageState::Failed,Some("Stock exact TUI resume rejected; zero-turn rollout may be unmaterialized; no warm-up/substitute"))).await.map_err(|code| (None, code))?;
                        return Err((None, ErrorCode::BackendError))
                    }
                    let connected = p.state==codex_relay::PresenceState::Connected;
                    if !connected {last_pending="relay_not_connected";}
                    if connected
                        && {let associated=p.resumed_thread_ids.contains(&thread)&&p.associated_thread_ids.contains(&thread);if !associated{last_pending="resume_association_missing";}associated}
                        && {let terminal=manifest.terminal(p.tui.as_ref()).await.map_err(|error| (Some("terminal_proof"), policy(error)))?;if !terminal{last_pending="terminal_proof_false";}terminal}
                        && {let loaded=link.loaded(&thread).await.map_err(|code| (Some("loaded_membership_read"), code))?;if !loaded{last_pending="loaded_membership_false";}loaded}
                    {
                        if tokio::time::Instant::now()>=deadline{report_launch_pending(&reporter,false,last_pending);return Err((None, ErrorCode::Timeout))}
                        if self.stopping.load(Ordering::Acquire){report_launch_pending(&reporter,true,last_pending);return Err((None, ErrorCode::Timeout))}
                        let connection = match &pinned_connection {
                            Some(connection) => Some(connection.clone()),
                            None => self.connections.lock().values().find(|c|
                                c.manifest.binding_id==binding
                                &&c.presence.connection_epoch==p.connection_epoch
                                &&c.key.native_conversation_id==thread
                            ).cloned(),
                        };
                        if let Some(connection)=connection{
                            if connection.presence.connection_epoch!=p.connection_epoch {
                                return Err((Some("connection_validate"), ErrorCode::StaleSession))
                            }
                            if pinned_connection.is_none() {
                                pinned_connection=Some(connection.clone());
                            }
                            if self.creation_connection_ready(&connection)
                                .map_err(|code| (Some("connection_validate"), code))?.is_none()
                            {
                                last_pending="connection_readiness_pending";return Ok(None)
                            }
                            let proof=auth.proof.clone();let id=action.clone();
                            let probe=self.security.storage.call(move|s|s.creation_probe(&proof,&id)).await.map_err(|error| (Some("creation_probe"), pairing::store_error(error)))?;
                            if tokio::time::Instant::now()>=deadline{report_launch_pending(&reporter,false,last_pending);return Err((None, ErrorCode::Timeout))}
                            if self.stopping.load(Ordering::Acquire){report_launch_pending(&reporter,true,last_pending);return Err((None, ErrorCode::Timeout))}
                            let _guard=self.security.native_guard(auth).await.map_err(|code| (Some("original_guard"), code))?;
                            if tokio::time::Instant::now()>=deadline{report_launch_pending(&reporter,false,last_pending);return Err((None, ErrorCode::Timeout))}
                            if self.stopping.load(Ordering::Acquire){report_launch_pending(&reporter,true,last_pending);return Err((None, ErrorCode::Timeout))}
                            let Some(source)=self.creation_connection_ready(&connection)
                                .map_err(|code| (Some("connection_validate"), code))?
                            else { last_pending="connection_readiness_pending";return Ok(None) };
                            let root=source.snapshot.pane(&manifest.herdr.pane_id).map_err(|error| (Some("root_pane"), policy(error)))?;
                            if root.focused||source.snapshot.focused_workspace_id.as_ref()!=Some(&probe.focused_workspace_id){return Err((Some("focus_continuity"), ErrorCode::NotReady))}
                            if !self.topology.created_binding_readiness(action,params,&resources).map_err(|code| (Some("created_binding"), code))?{last_pending="created_binding_pending";return Ok(None)}
                            if !source.recheck_readiness().map_err(|error| (Some("topology_source"), policy(error)))?{last_pending="source_recheck_pending";return Ok(None)}
                            last_pending="unobserved";
                            if tokio::time::Instant::now()>=deadline{report_launch_pending(&reporter,false,last_pending);return Err((None, ErrorCode::Timeout))}
                            if self.stopping.load(Ordering::Acquire){report_launch_pending(&reporter,true,last_pending);return Err((None, ErrorCode::Timeout))}
                            return Ok(Some((connection, probe.focused_workspace_id)))
                        } else {
                            last_pending="connection_absent";
                        }
                    }
                    if let Some(connection)=&pinned_connection {
                        let _ = self.creation_connection_ready(connection)
                            .map_err(|code| (Some("connection_validate"), code))?;
                        return Err((Some("native_binding"), ErrorCode::NotReady))
                    }
                    Ok::<_, (Option<&'static str>, ErrorCode)>(None)
                }.await;
                let connection = match admission {
                    Ok(connection) => connection,
                    Err((gate, code)) => {
                        if let Some(gate) = gate {
                            let proof=auth.proof.clone();let id=action.clone();
                            let known=resources.clone();let metadata=owner_metadata(&owner,&epoch);
                            let mut gate_proof=launch_failure_proof(gate, code, presence_diagnostic);
                            if let Some(phase) = presence_phase {
                                gate_proof.push_str("; presence_phase=");
                                gate_proof.push_str(phase.as_str());
                            }
                            // Diagnostic failure/revoke must never mask the original admission error.
                            let _ = self.security.storage.call(move |s| {
                                s.creation_result(&proof, &id, stage(
                                    known, metadata, StageState::Dispatching, Some(&gate_proof),
                                ))?;
                                s.transition(&proof, &id, ReceiptTransition {
                                    state: ReceiptState::Uncertain,
                                    proof: TransitionProof::LostOutcome,
                                    result: None,
                                    error: Some(crate::transport::wire_error(code)),
                                    at: pairing::now_ms(),
                                }).map(|_| ())
                            }).await;
                        }
                        return Err(code)
                    }
                };
                if let Some((connection, focused_workspace))=connection{
                        if tokio::time::Instant::now()>=deadline||self.stopping.load(Ordering::Acquire){return Err(ErrorCode::Timeout)}
                        resources.native_id=Some(thread.clone());let metadata=owner_metadata(&owner,&epoch);self.settle_stage(auth,action,stage(resources.clone(),metadata.clone(),StageState::Confirmed,Some("Exact TUI-correlated successful resume of original zero-turn thread"))).await?;
                        if tokio::time::Instant::now()>=deadline||self.stopping.load(Ordering::Acquire){return Err(ErrorCode::Timeout)}
                        resources.session_id=Some(connection.target.session_id.clone());resources.generation=Some(connection.target.generation.clone());
                        let proof=auth.proof.clone();let id=action.clone();let intent=metadata.clone();
                        self.security.storage.call(move|s|{
                            if tokio::time::Instant::now()>=deadline{return Err(crate::storage::StorageError::Policy(ErrorCode::Timeout))}
                            s.creation_intent(&proof,&id,CreationStage::Ready,intent,pairing::now_ms())
                        }).await.map_err(pairing::store_error)?;
                        // Stage writes above happen once. A source refresh only waits for this
                        // exact installed connection; it never restarts stages or picks a new Target.
                        loop {
                            if tokio::time::Instant::now()>=deadline||self.stopping.load(Ordering::Acquire){return Err(ErrorCode::Timeout)}
                            let _guard=self.security.native_guard(auth).await?;
                            if tokio::time::Instant::now()>=deadline||self.stopping.load(Ordering::Acquire){return Err(ErrorCode::Timeout)}
                            if let Some(source)=self.creation_connection_ready(&connection)? {
                                let pane=source.snapshot.pane(&manifest.herdr.pane_id).map_err(policy)?;
                                if pane.focused||source.snapshot.focused_workspace_id.as_ref()!=Some(&focused_workspace){return Err(ErrorCode::NotReady)}
                                if self.topology.created_binding_readiness(action,params,&resources)?
                                    &&source.recheck_readiness().map_err(policy)?
                                {
                                    if tokio::time::Instant::now()>=deadline||self.stopping.load(Ordering::Acquire){return Err(ErrorCode::Timeout)}
                                    break
                                }
                            }
                            drop(_guard);
                            while events.try_recv().is_ok(){
                                if tokio::time::Instant::now()>=deadline||self.stopping.load(Ordering::Acquire){return Err(ErrorCode::Timeout)}
                            }
                            tokio::time::sleep_until(deadline.min(tokio::time::Instant::now()+Duration::from_millis(100))).await;
                        }
                        let proof=auth.proof.clone();let id=action.clone();let known=resources.clone();let target=connection.target.clone();
                        self.security.storage.call(move|s|{
                            if tokio::time::Instant::now()>=deadline{return Err(crate::storage::StorageError::Policy(ErrorCode::Timeout))}
                            s.creation_result(&proof,&id,stage(known.clone(),metadata,StageState::Confirmed,Some("Exact stock owner/thread/TUI/native snapshot ready")))?;
                            s.transition(&proof,&id,ReceiptTransition{state:ReceiptState::Succeeded,proof:TransitionProof::CorrelatedAcknowledgement(Id("Exact native desktop binding ready".into())),result:Some(ActionResult::Created(Box::new(ActionResultCreated{target,workspace_id:known.workspace_id.ok_or(crate::storage::StorageError::Policy(ErrorCode::NotReady))?,terminal_id:known.terminal_id.ok_or(crate::storage::StorageError::Policy(ErrorCode::NotReady))?,pane_id:known.pane_id.ok_or(crate::storage::StorageError::Policy(ErrorCode::NotReady))?,native_id:thread,binding_id:Some(binding)}))),error:None,at:pairing::now_ms()})
                        }).await.map_err(pairing::store_error)?;
                        return Ok(())
                    }
                // The original subscription keeps the zero-turn thread loaded. Drain, never replay.
                while events.try_recv().is_ok(){
                    if tokio::time::Instant::now()>=deadline{report_launch_pending(&reporter,false,"drain");return Err(ErrorCode::Timeout)}
                    if self.stopping.load(Ordering::Acquire){report_launch_pending(&reporter,true,"drain");return Err(ErrorCode::Timeout)}
                }
                tokio::time::sleep_until(deadline.min(tokio::time::Instant::now()+Duration::from_millis(100))).await;
            }
        }.await;
        link.close().await;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    #[test]
    fn create_receipt_reserved_and_existing_return_the_same_committed_receipt() {
        let receipt = Receipt {
                    shell_target: None,
            action_id: Uuid::random(),
            method: ActionMethod::SessionCreate,
            boot_id: Uuid::random(),
            target: None,
            created_at: Uint(0),
            updated_at: Uint(0),
            state: ReceiptState::Reserved,
            error: None,
            result: None,
            creation: None,
        };
        let Some(reserved) = reservation_reply(true, Reservation::Reserved(receipt.clone())) else {
            panic!("SessionCreate must return its committed reservation")
        };
        let Some(existing) = reservation_reply(true, Reservation::Existing(receipt.clone())) else {
            panic!("matching SessionCreate must return its existing receipt")
        };
        assert_eq!(reserved, receipt);
        assert_eq!(existing, receipt);
        assert!(reservation_reply(false, Reservation::Reserved(receipt)).is_none());
    }
    #[test]
    fn uncertain_thread_proof_keeps_known_native_id_without_disclosing_it() {
        let mut resources = herdr::empty_resources();
        resources.native_id = Some(Id("private-original-thread".into()));
        let outcome = thread_uncertain(
            resources,
            StageMetadata::default(),
            "thread_materializing_read",
            ErrorCode::Timeout,
        );
        assert_eq!(outcome.state, StageState::Uncertain);
        assert_eq!(
            outcome.resources.native_id,
            Some(Id("private-original-thread".into()))
        );
        let proof = outcome.definitive.unwrap();
        proof.validate().unwrap();
        assert!(
            proof
                .0
                .contains("gate=thread_materializing_read; error=Timeout")
        );
        assert!(!proof.0.contains("private-original-thread"));
    }
    fn flat_test_page(first: usize, len: usize, next: Option<&str>, turn: &str) -> Value {
        json!({"data":(first..first+len).map(|index|json!({"turnId":turn,"startedAtMs":index,"completedAtMs":if index%3==0{Value::Null}else{json!(index)},"item":{"id":format!("flat-{index}"),"type":"agentMessage","text":"fixture"}})).collect::<Vec<_>>(),"nextCursor":next})
    }
    #[test]
    fn flat_history_pages_more_than_retention_without_turn_seek_or_cursor_arithmetic() {
        let thread = Id("original-thread".into());
        let mut position = Some(FlatPosition::head());
        let mut complete = 0;
        let mut previous = UINT_MAX + 1;
        for page in 0..25 {
            let p = position.take().unwrap();
            let request = p.request(&thread);
            assert_eq!(request["threadId"], "original-thread");
            assert!(request.get("turnId").is_none());
            assert_eq!(request["limit"], 100);
            assert_eq!(request["sortDirection"], "desc");
            let token = format!("opaque:{}:never-parse", 25 - page);
            let next = (page < 24).then_some(token.as_str());
            let (entries, successor) = p
                .accept(&flat_test_page(page * 100, 100, next, "one-large-turn"))
                .unwrap();
            for record in &entries {
                assert!(record.order < previous);
                previous = record.order;
                complete += usize::from(record.complete);
            }
            position = successor;
        }
        assert!(complete > 1000);
        assert!(position.is_none());
    }
    #[test]
    fn flat_native_fixture_crosses_live_window_through_browse_owner() {
        use crate::state::BrowseLookup;
        use crate::storage::{PairedDevice, Storage};
        let (state, target, epoch) = publication_observation_state();
        let mut storage = Storage::memory([47; 32], 0).unwrap();
        let device = PairedDevice {
            device_id: Uuid::random(),
            endpoint_id: EndpointId(iroh::SecretKey::from_bytes(&[48; 32]).public().to_string()),
            device_name: Label::new("native browse").unwrap(),
            paired_at: Uint(0),
            last_seen_at: None,
        };
        let authorization = storage.pair_device(&device).unwrap();
        let peer = storage
            .authenticated_hello(&device.device_id, authorization)
            .unwrap();
        let ticket = state
            .begin_browse(
                &peer,
                &target,
                &epoch,
                BrowseSource::Flat(FlatPosition::head()).position().unwrap(),
                100,
                0,
            )
            .unwrap();
        let view = ticket.view_id().clone();
        let mut cursor = ticket.cursor();
        let mut current = Some(ticket);
        let mut completed = 0;
        for step in 0..16 {
            let ticket = current.take().unwrap_or_else(|| {
                let BrowseLookup::Fetch(ticket) = state
                    .browse_page(&peer, &target, &view, &cursor, 100, 0)
                    .unwrap()
                else {
                    panic!("uncaptured Codex page must use the original browse owner")
                };
                ticket
            });
            let BrowseSource::Flat(position) = ticket.position::<BrowseSource>().unwrap().clone()
            else {
                panic!("fixture must preserve the flat native reader")
            };
            let next = (step < 15).then(|| format!("opaque-{step}"));
            let (entries, successor) = position
                .accept(&flat_test_page(
                    step * 100,
                    100,
                    next.as_deref(),
                    "one-large-turn",
                ))
                .unwrap();
            let records = entries
                .into_iter()
                .filter(|entry| entry.complete)
                .map(|entry| {
                    project_safe_item(
                        &state,
                        &target,
                        &entry.turn,
                        &entry.item,
                        entry.order,
                        RecordSource::NativePage,
                        (entry.complete, entry.timestamp),
                    )
                })
                .collect::<SecurityResult<Vec<_>>>()
                .unwrap();
            let page = state
                .finish_browse(
                    ticket,
                    records,
                    successor
                        .map(BrowseSource::Flat)
                        .map(BrowseSource::position)
                        .transpose()
                        .unwrap(),
                    0,
                )
                .unwrap();
            completed += page.records.len();
            let BrowseLookup::Cached(repeated) = state
                .browse_page(&peer, &target, &view, &cursor, 100, 1)
                .unwrap()
            else {
                panic!("same Codex browse cursor must be memoized")
            };
            assert_eq!(page, repeated);
            if step < 15 {
                cursor = page.next_cursor.unwrap();
            } else {
                assert!(page.next_cursor.is_none());
            }
        }
        assert!(completed > 1000);
    }
    #[test]
    fn flat_history_empty_progress_preserves_native_positions_and_frozen_membership() {
        let mut page = flat_test_page(0, 1, Some("z opaque token"), "turn");
        let (entries, next) = FlatPosition::head().accept(&page).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(!entries[0].complete);
        page["data"][0]["completedAtMs"] = json!(1);
        assert!(
            !entries[0].complete,
            "captured page cannot be changed by later completion"
        );
        let (entries, next) = next
            .unwrap()
            .accept(&flat_test_page(
                1,
                1,
                Some("a opaque token"),
                "another-turn",
            ))
            .unwrap();
        assert!(entries[0].complete);
        assert!(
            next.unwrap()
                .accept(&flat_test_page(2, 1, Some("z opaque token"), "turn-3"))
                .is_err()
        );
        assert!(
            FlatPosition::head().accept(&page).unwrap().0[0].complete,
            "explicit new view sees later completion"
        );
    }
    #[test]
    fn flat_history_rejects_duplicate_ids_bad_lifecycle_and_unproven_mode() {
        let (_, next) = FlatPosition::head()
            .accept(&flat_test_page(1, 1, Some("next"), "turn"))
            .unwrap();
        assert!(
            next.unwrap()
                .accept(&flat_test_page(1, 1, None, "wrong-turn"))
                .is_err()
        );
        let mut page = flat_test_page(2, 1, None, "turn");
        page["data"][0]["completedAtMs"] = json!("invalid-time");
        assert!(FlatPosition::head().accept(&page).is_err());
        let expected = Id("original".into());
        assert_eq!(
            HistoryMode::from_thread(&json!({"id":"original","historyMode":"legacy"}), &expected)
                .unwrap(),
            HistoryMode::Legacy
        );
        assert_eq!(
            HistoryMode::from_thread(
                &json!({"id":"original","historyMode":"paginated"}),
                &expected
            )
            .unwrap(),
            HistoryMode::Paginated
        );
        for mode in [Value::Null, json!("unknown")] {
            assert!(
                HistoryMode::from_thread(&json!({"id":"original","historyMode":mode}), &expected)
                    .is_err()
            );
        }
        assert!(
            HistoryMode::from_thread(&json!({"id":"other","historyMode":"paginated"}), &expected)
                .is_err()
        );
    }
    fn publication_observation_state() -> (State, Target, Uuid) {
        let state = State::default();
        let display: SessionState = serde_json::from_value(json!({
            "summary":{"sessionId":Uuid::random(),"generation":Uuid::random(),
                "serverId":Uuid::random(),"workspaceId":"workspace","terminalId":"terminal",
                "paneId":"pane","provider":"codex","nativeId":"thread","title":"test",
                "status":"idle","connectivity":"ready",
                "capabilities":{"history":true,"prompt":true,"cancel":true,"respond":true,
                    "promptMode":"submit_or_steer","cancelMode":"native_turn"},
                "historyCoverage":{"scope":"thread","source":"native_pages",
                    "olderAvailable":false,"sourceTruncated":false,"desktopBranchKnown":false}},
            "desktopSelection":"bound_launch_thread","activity":{"state":"idle"}
        }))
        .unwrap();
        let target = Target {
            session_id: display.summary.session_id.clone(),
            generation: display.summary.generation.clone(),
        };
        let epoch = state.begin_install(display.clone()).unwrap();
        state
            .finish_install(
                &epoch,
                ProviderSnapshot {
                    state: display,
                    pending: vec![],
                    history: vec![],
                    older_available: false,
                    native_older: None,
                },
            )
            .unwrap();
        (state, target, epoch)
    }

    fn original_observation_presence() -> Presence {
        let relay = ProcessEvidence {
            linux_boot_id: Uuid::random(),
            pid: 42,
            start_ticks: 7,
            namespaces: herdr::ProcessNamespaces {
                pid: (1, 2),
                mount: (3, 4),
                user: (5, 6),
                nspids: vec![42],
            },
        };
        let socket_source: codex_relay::SocketSource = serde_json::from_value(json!({
            "root":["0","0"],"hostParent":["0","0"],"hostEntry":["0","0"],
            "agentParent":["0","0"],"alias":["0","0"],"physicalParent":["0","0"]
        }))
        .unwrap();
        Presence {
            v: 1,
            binding_id: Uuid::random(),
            owner_epoch: OwnerEpoch {
                linux_boot_id: relay.linux_boot_id.clone(),
                pid: relay.pid,
                start_ticks: "7".into(),
                socket_device: "0".into(),
                socket_inode: "0".into(),
                executable_sha256: "fixture".into(),
                socket_source,
            },
            relay: relay.clone(),
            tui: Some(relay),
            connection_epoch: Uuid::random(),
            state: codex_relay::PresenceState::Connected,
            terminal_proof: codex_relay::TerminalProofAvailability::Available,
            associated_thread_ids: vec![Id("thread".into())],
            resumed_thread_ids: vec![Id("thread".into())],
            resume_rejected_thread: None,
            collaboration_mode: None,
            collaboration_default_plan: None,
            updated_at: Uint(1),
            error: None,
        }
    }

    #[test]
    fn original_presence_state_error_guard_keeps_only_closed_relay_literals() {
        let known = [
            (
                "Relay identity, transport or bounded admission failed; explicit reconciliation required",
                "original_presence_state_relay_bounded_admission_failed",
            ),
            (
                "Stock TUI rejected exact thread/resume; zero-turn rollout may be unmaterialized. No warm-up or substitute was attempted",
                "original_presence_state_stock_tui_resume_rejected",
            ),
            (
                "TUI transport read failed",
                "original_presence_state_tui_read",
            ),
            (
                "TUI data frame invalid",
                "original_presence_state_tui_frame",
            ),
            (
                "TUI observation failed",
                "original_presence_state_tui_observation",
            ),
            (
                "Owner transport read failed",
                "original_presence_state_owner_read",
            ),
            (
                "Owner data frame invalid",
                "original_presence_state_owner_frame",
            ),
            (
                "Owner observation failed",
                "original_presence_state_owner_observation",
            ),
            (
                "Owner/manifest source proof failed",
                "original_presence_state_manifest_proof",
            ),
            (
                "Relay publication source changed",
                "original_presence_state_publication_changed",
            ),
            (
                "Native observation publication failed",
                "original_presence_state_observation_publish",
            ),
            (
                "Relay presence publication failed",
                "original_presence_state_presence_publish",
            ),
            (
                "Owner-bound relay queue failed",
                "original_presence_state_owner_queue",
            ),
            (
                "TUI-bound relay queue failed",
                "original_presence_state_tui_queue",
            ),
            (
                "Owner write timed out",
                "original_presence_state_owner_write_timeout",
            ),
            ("Owner write failed", "original_presence_state_owner_write"),
            (
                "TUI write timed out",
                "original_presence_state_tui_write_timeout",
            ),
            ("TUI write failed", "original_presence_state_tui_write"),
            (
                "Herdr pane.process_info rejected: pane_not_found",
                "original_presence_state_pane_not_found",
            ),
            (
                "Herdr terminal source proof failed",
                "original_presence_state_terminal_proof",
            ),
            (
                "Bound terminal closed",
                "original_presence_state_terminal_closed",
            ),
            (
                "Relay workers absent",
                "original_presence_state_relay_workers_absent",
            ),
            (
                "Relay worker failed",
                "original_presence_state_relay_worker_failed",
            ),
            (
                "Native relay forwarding closed",
                "original_presence_state_native_relay_closed",
            ),
        ];
        for (literal, guard) in known {
            assert_eq!(original_presence_state_error_guard(Some(literal)), guard);
            assert_eq!(
                original_presence_state_error_guard(Some(&format!("{literal} suffix"))),
                "original_presence_state"
            );
        }
        assert_eq!(
            original_presence_state_error_guard(Some("private relay failure")),
            "original_presence_state"
        );
        assert_eq!(
            original_presence_state_error_guard(None),
            "original_presence_state"
        );
    }

    #[test]
    fn original_presence_terminal_capture_tui_write_guards_are_closed() {
        for (phase, guard) in [
            (
                "tui_write_unpolled_not_pending",
                "original_presence_state_terminal_tui_write_unpolled_not_pending",
            ),
            (
                "tui_write_ready_not_pending",
                "original_presence_state_terminal_tui_write_ready_not_pending",
            ),
            (
                "tui_write_ready_pending",
                "original_presence_state_terminal_tui_write_ready_pending",
            ),
            (
                "tui_write_start_send_not_pending",
                "original_presence_state_terminal_tui_write_start_send_not_pending",
            ),
            (
                "tui_write_flush_not_pending",
                "original_presence_state_terminal_tui_write_flush_not_pending",
            ),
            (
                "tui_write_flush_pending",
                "original_presence_state_terminal_tui_write_flush_pending",
            ),
        ] {
            assert_eq!(
                original_presence_terminal_capture_guard(codex_relay::TerminalCapture {
                    phase,
                    io: "timed_out",
                    errno: 0,
                    response_timeout: false,
                }),
                guard
            );
        }
    }

    #[test]
    fn original_presence_scope_keeps_the_first_closed_mismatch() {
        let original = original_observation_presence();
        let thread = Id("thread".into());
        let mut changed = original.clone();
        changed.state = codex_relay::PresenceState::Disconnected;
        changed.binding_id = Uuid::random();
        assert_eq!(
            original_presence_scope(&changed, &original, &thread),
            Err(OriginalPresenceScopeFailure::State)
        );

        let changes: &[(OriginalPresenceScopeFailure, fn(&mut Presence))] = &[
            (OriginalPresenceScopeFailure::Binding, |presence| {
                presence.binding_id = Uuid::random();
            }),
            (OriginalPresenceScopeFailure::OwnerEpoch, |presence| {
                presence.owner_epoch.start_ticks = "replacement".into();
            }),
            (OriginalPresenceScopeFailure::Relay, |presence| {
                presence.relay.start_ticks += 1;
            }),
            (OriginalPresenceScopeFailure::ConnectionEpoch, |presence| {
                presence.connection_epoch = Uuid::random();
            }),
            (OriginalPresenceScopeFailure::Tui, |presence| {
                presence.tui.as_mut().expect("fixture TUI").start_ticks += 1;
            }),
            (OriginalPresenceScopeFailure::Associated, |presence| {
                presence.associated_thread_ids.clear();
            }),
            (OriginalPresenceScopeFailure::Resumed, |presence| {
                presence.resumed_thread_ids.clear();
            }),
        ];
        for (expected, change) in changes {
            let mut changed = original.clone();
            change(&mut changed);
            assert_eq!(
                original_presence_scope(&changed, &original, &thread),
                Err(*expected)
            );
        }
    }

    #[test]
    fn past_original_presence_gates_mutation_until_fresh_complete_observation() {
        use codex_relay::ValidatedPresence;
        let (state, target, epoch) = publication_observation_state();
        let original = original_observation_presence();
        let thread = Id("thread".into());
        let serial = Mutex::new(());
        let request = Request {
            id: Uuid::random(),
            body: RequestBody::PromptSubmit(PromptSubmitParams {
                boot_id: Uuid::random(),
                target: target.clone(),
                text: PromptText::new("test").unwrap(),
            }),
        };
        assert_eq!(
            revalidate_observation(
                &state,
                &target,
                &epoch,
                &serial,
                || original_presence_decision(
                    &state,
                    &target,
                    &epoch,
                    Some(&ValidatedPresence::Past(original.clone())),
                    &original,
                    &thread
                )
                .map(OriginalPresenceDecision::is_fresh),
                || Ok(Some(()))
            ),
            Ok(None)
        );
        assert_eq!(
            state.observation_ready(&target, &epoch),
            Err(ErrorCode::NotReady)
        );
        assert_eq!(
            state
                .session_state(&target, &epoch)
                .unwrap()
                .summary
                .connectivity,
            SessionConnectivity::Synchronizing
        );
        assert_eq!(
            state.validate_session_mutation_checked(&request, Some(&epoch)),
            Err(("core_observation_pending", ErrorCode::NotReady))
        );

        for changed in [
            {
                let mut p = original.clone();
                p.relay.start_ticks += 1;
                p
            },
            {
                let mut p = original.clone();
                p.resumed_thread_ids.clear();
                p
            },
            {
                let mut p = original.clone();
                p.connection_epoch = Uuid::random();
                p
            },
        ] {
            assert_eq!(
                original_presence_decision(
                    &state,
                    &target,
                    &epoch,
                    Some(&ValidatedPresence::Past(changed)),
                    &original,
                    &thread
                ),
                Err(ErrorCode::StaleSession)
            );
            assert_eq!(
                state.validate_session_mutation_checked(&request, Some(&epoch)),
                Err(("core_observation_pending", ErrorCode::NotReady))
            );
        }
        assert_eq!(
            revalidate_observation(
                &state,
                &target,
                &epoch,
                &serial,
                || original_presence_decision(
                    &state,
                    &target,
                    &epoch,
                    Some(&ValidatedPresence::Fresh(original.clone())),
                    &original,
                    &thread
                )
                .map(OriginalPresenceDecision::is_fresh),
                || Ok(None::<()>)
            ),
            Ok(None)
        );
        assert_eq!(
            state.observation_ready(&target, &epoch),
            Err(ErrorCode::NotReady)
        );
        assert_eq!(
            revalidate_observation(
                &state,
                &target,
                &epoch,
                &serial,
                || original_presence_decision(
                    &state,
                    &target,
                    &epoch,
                    Some(&ValidatedPresence::Fresh(original.clone())),
                    &original,
                    &thread
                )
                .map(OriginalPresenceDecision::is_fresh),
                || Ok(Some(()))
            ),
            Ok(Some(()))
        );
        assert!(
            state
                .validate_session_mutation_checked(&request, Some(&epoch))
                .is_ok()
        );
    }

    #[test]
    fn terminal_proof_unavailable_keeps_original_identity_and_blocks_mutation() {
        use codex_relay::{TerminalProofAvailability, ValidatedPresence};
        let (state, target, epoch) = publication_observation_state();
        let original = original_observation_presence();
        let thread = Id("thread".into());
        let serial = Mutex::new(());
        let mut unavailable = original.clone();
        unavailable.terminal_proof = TerminalProofAvailability::Unavailable;
        assert_eq!(
            original_presence_decision(
                &state,
                &target,
                &epoch,
                Some(&ValidatedPresence::Unavailable(unavailable.clone())),
                &original,
                &thread,
            ),
            Ok(OriginalPresenceDecision::Unavailable(
                unavailable.updated_at.0
            ))
        );
        let mut disconnected = unavailable.clone();
        disconnected.state = codex_relay::PresenceState::Disconnected;
        assert_eq!(
            original_presence_decision(
                &state,
                &target,
                &epoch,
                Some(&ValidatedPresence::Unavailable(disconnected)),
                &original,
                &thread,
            ),
            Err(ErrorCode::StaleSession)
        );
        let mut replaced = unavailable.clone();
        replaced.connection_epoch = Uuid::random();
        assert_eq!(
            original_presence_decision(
                &state,
                &target,
                &epoch,
                Some(&ValidatedPresence::Unavailable(replaced)),
                &original,
                &thread,
            ),
            Err(ErrorCode::StaleSession)
        );
        let mut rejected = unavailable.clone();
        rejected.resume_rejected_thread = Some(thread.clone());
        assert!(unavailable_resume_rejected(
            &ValidatedPresence::Unavailable(rejected),
            &thread
        ));
        let now = pairing::now_ms();
        assert!(
            OriginalPresenceDecision::Unavailable(now.saturating_add(1_001))
                .final_freshness(now)
                .is_err()
        );
        assert_eq!(
            revalidate_observation(
                &state,
                &target,
                &epoch,
                &serial,
                || {
                    original_presence_decision(
                        &state,
                        &target,
                        &epoch,
                        Some(&ValidatedPresence::Unavailable(unavailable.clone())),
                        &original,
                        &thread,
                    )
                    .map(OriginalPresenceDecision::is_fresh)
                },
                || Ok(Some(())),
            ),
            Ok(None)
        );
        assert_eq!(
            state.observation_ready(&target, &epoch),
            Err(ErrorCode::NotReady)
        );
        assert_eq!(
            revalidate_observation(
                &state,
                &target,
                &epoch,
                &serial,
                || {
                    original_presence_decision(
                        &state,
                        &target,
                        &epoch,
                        Some(&ValidatedPresence::Fresh(original.clone())),
                        &original,
                        &thread,
                    )
                    .map(OriginalPresenceDecision::is_fresh)
                },
                || Ok(Some(())),
            ),
            Ok(Some(()))
        );
    }

    #[test]
    fn unpinned_past_presence_remains_strict() {
        use codex_relay::ValidatedPresence;
        let presence = original_observation_presence();
        assert!(matches!(
            strict_unpinned_presence(ValidatedPresence::Past(presence.clone())),
            Err(ErrorCode::NotReady)
        ));
        let mut unavailable = presence.clone();
        unavailable.terminal_proof = codex_relay::TerminalProofAvailability::Unavailable;
        assert!(matches!(
            strict_unpinned_presence(ValidatedPresence::Unavailable(unavailable)),
            Err(ErrorCode::NotReady)
        ));
        assert!(matches!(
            strict_unpinned_presence(ValidatedPresence::Fresh(presence)),
            Ok(ValidatedPresence::Fresh(_))
        ));
    }

    #[test]
    fn fresh_record_aging_during_topology_cannot_clear_original_gate() {
        let (state, target, epoch) = publication_observation_state();
        let serial = Mutex::new(());
        assert_eq!(
            revalidate_observation_with_final(
                &state,
                &target,
                &epoch,
                &serial,
                || Ok(true),
                || Ok(Some(())),
                || codex_relay::freshness_at_completion(5_000, 10_001)
                    .map_err(|failure| policy(failure.error)),
            ),
            Ok(None)
        );
        assert_eq!(
            state.observation_ready(&target, &epoch),
            Err(ErrorCode::NotReady)
        );
        assert_eq!(
            state
                .session_state(&target, &epoch)
                .unwrap()
                .summary
                .connectivity,
            SessionConnectivity::Synchronizing
        );
        assert_eq!(
            revalidate_observation_with_final(
                &state,
                &target,
                &epoch,
                &serial,
                || Ok(true),
                || Ok(Some(())),
                || codex_relay::freshness_at_completion(12_000, 10_000)
                    .map_err(|failure| policy(failure.error)),
            ),
            Err(ErrorCode::NotReady)
        );
        assert_eq!(
            state.observation_ready(&target, &epoch),
            Err(ErrorCode::NotReady)
        );
        assert_eq!(
            revalidate_observation_with_final(
                &state,
                &target,
                &epoch,
                &serial,
                || Ok(true),
                || Ok(Some(())),
                || codex_relay::freshness_at_completion(5_000, 10_000)
                    .map_err(|failure| policy(failure.error)),
            ),
            Ok(Some(()))
        );
        assert!(state.observation_ready(&target, &epoch).is_ok());
    }

    #[test]
    fn past_original_presence_rechecks_future_at_final_decision() {
        let (state, target, epoch) = publication_observation_state();
        let original = original_observation_presence();
        let mut changed_clock = original.clone();
        changed_clock.updated_at = Uint(12_000);
        let thread = Id("thread".into());
        let serial = Mutex::new(());
        let decision = std::cell::Cell::new(OriginalPresenceDecision::Unavailable(0));
        assert_eq!(
            revalidate_observation_with_final(
                &state,
                &target,
                &epoch,
                &serial,
                || {
                    let observed = original_presence_decision(
                        &state,
                        &target,
                        &epoch,
                        Some(&codex_relay::ValidatedPresence::Past(changed_clock)),
                        &original,
                        &thread,
                    )?;
                    decision.set(observed);
                    Ok(observed.is_fresh())
                },
                || Ok(Some(())),
                || decision
                    .get()
                    .final_freshness(10_000)
                    .map_err(|failure| policy(failure.error)),
            ),
            Err(ErrorCode::NotReady)
        );
        assert_eq!(decision.get(), OriginalPresenceDecision::Past(12_000));
        assert_eq!(
            state.observation_ready(&target, &epoch),
            Err(ErrorCode::NotReady)
        );
        assert!(matches!(
            OriginalPresenceDecision::Past(5_000).final_freshness(5_000),
            Ok(false)
        ));
    }

    #[test]
    fn publication_pending_gates_original_source_until_complete_revalidation_and_keeps_loss_terminal()
     {
        use codex_relay::{PresenceFailure, PresencePhase, PublicationUnstable};
        let (state, target, epoch) = publication_observation_state();
        let serial = Mutex::new(());
        let pending = || {
            Err(PresenceFailure {
                phase: PresencePhase::PresenceRecord,
                error: io::Error::new(io::ErrorKind::InvalidData, PublicationUnstable),
            })
        };
        assert!(
            presence_observation(&state, &target, &epoch, pending())
                .unwrap()
                .is_none()
        );
        assert_eq!(
            state.observation_ready(&target, &epoch),
            Err(ErrorCode::NotReady)
        );
        let request = Request {
            id: Uuid::random(),
            body: RequestBody::PromptSubmit(PromptSubmitParams {
                boot_id: Uuid::random(),
                target: target.clone(),
                text: PromptText::new("test").unwrap(),
            }),
        };
        assert_eq!(
            state.validate_session_mutation_checked(&request, Some(&epoch)),
            Err(("core_observation_pending", ErrorCode::NotReady))
        );
        state.validate_source(&target, &epoch).unwrap();
        let mut display = state.session_state(&target, &epoch).unwrap();
        display.summary.connectivity = SessionConnectivity::Ready;
        state
            .update_from_source(&target, &epoch, SessionUpdate::State(display))
            .unwrap();
        assert_eq!(
            state
                .session_state(&target, &epoch)
                .unwrap()
                .summary
                .connectivity,
            SessionConnectivity::Synchronizing
        );
        assert_eq!(
            revalidate_observation(
                &state,
                &target,
                &epoch,
                &serial,
                || Ok(false),
                || Ok(Some(()))
            ),
            Ok(None)
        );
        assert_eq!(
            revalidate_observation::<()>(
                &state,
                &target,
                &epoch,
                &serial,
                || Ok(true),
                || Ok(None)
            ),
            Ok(None)
        );
        assert_eq!(
            state.observation_ready(&target, &epoch),
            Err(ErrorCode::NotReady)
        );
        assert_eq!(
            revalidate_observation(
                &state,
                &target,
                &epoch,
                &serial,
                || Ok(true),
                || Ok(Some(()))
            ),
            Ok(Some(()))
        );
        state.observation_ready(&target, &epoch).unwrap();
        assert!(
            state
                .validate_session_mutation_checked(&request, Some(&epoch))
                .is_ok()
        );
        assert!(
            presence_observation(&state, &target, &epoch, pending())
                .unwrap()
                .is_none()
        );
        for (phase, error) in [
            (
                PresencePhase::PresenceRecord,
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "private Codex publication did not stabilize",
                ),
            ),
            (
                PresencePhase::OwnerProof,
                io::Error::new(io::ErrorKind::InvalidData, PublicationUnstable),
            ),
            (
                PresencePhase::PresenceRecord,
                io::Error::new(io::ErrorKind::PermissionDenied, "unsafe private path"),
            ),
            (
                PresencePhase::TuiProcess,
                io::Error::from_raw_os_error(libc::ENOENT),
            ),
        ] {
            assert!(matches!(
                presence_observation(
                    &state,
                    &target,
                    &epoch,
                    Err(PresenceFailure { phase, error })
                ),
                Err(ErrorCode::NotReady)
            ));
            assert_eq!(
                state.observation_ready(&target, &epoch),
                Err(ErrorCode::NotReady)
            );
        }
        assert_eq!(
            revalidate_observation::<()>(
                &state,
                &target,
                &epoch,
                &serial,
                || Err(ErrorCode::StaleSession),
                || Ok(Some(()))
            ),
            Err(ErrorCode::StaleSession)
        );
        assert_eq!(
            revalidate_observation::<()>(
                &state,
                &target,
                &epoch,
                &serial,
                || Ok(false),
                || Err(ErrorCode::StaleSession)
            ),
            Err(ErrorCode::StaleSession)
        );
        state.begin_source_install(&target).unwrap();
        assert!(matches!(
            presence_observation(&state, &target, &epoch, pending()),
            Err(ErrorCode::StaleSession)
        ));
        assert_eq!(
            revalidate_observation(
                &state,
                &target,
                &epoch,
                &serial,
                || Ok(true),
                || Ok(Some(()))
            ),
            Err(ErrorCode::StaleSession)
        );
    }

    #[test]
    fn publication_observation_serializes_fresh_clear_before_later_pending() {
        let (state, target, epoch) = publication_observation_state();
        state.observation_pending(&target, &epoch).unwrap();
        let serial = Mutex::new(());
        let (entered, observed) = std::sync::mpsc::channel();
        let (release, resume) = std::sync::mpsc::channel();
        let (attempted, waiting) = std::sync::mpsc::channel();
        std::thread::scope(|threads| {
            let (state, target, epoch, serial) = (&state, &target, &epoch, &serial);
            let fresh = threads.spawn(move || {
                revalidate_observation(
                    state,
                    target,
                    epoch,
                    serial,
                    || {
                        entered.send(()).unwrap();
                        resume.recv().unwrap();
                        Ok(true)
                    },
                    || Ok(Some(())),
                )
            });
            observed.recv().unwrap();
            let serialized = serial.try_lock().is_none();
            let pending = threads.spawn(move || {
                attempted.send(()).unwrap();
                revalidate_observation(
                    state,
                    target,
                    epoch,
                    serial,
                    || {
                        state.observation_pending(target, epoch)?;
                        Ok(false)
                    },
                    || Ok(Some(())),
                )
            });
            waiting.recv().unwrap();
            release.send(()).unwrap();
            assert_eq!(fresh.join().unwrap(), Ok(Some(())));
            assert_eq!(pending.join().unwrap(), Ok(None));
            assert!(
                serialized,
                "fresh check holds the lock through gate clearing"
            );
        });
        assert_eq!(
            state.observation_ready(&target, &epoch),
            Err(ErrorCode::NotReady)
        );
        state.validate_source(&target, &epoch).unwrap();
    }
    #[test]
    fn active_native_items_and_nullable_command_output_stay_on_original_target() {
        use crate::storage::{PairedDevice, Storage};

        let state = State::default();
        let target = Target {
            session_id: Uuid::random(),
            generation: Uuid::random(),
        };
        let display: SessionState = serde_json::from_value(json!({
            "summary":{"sessionId":target.session_id,"generation":target.generation,
                "serverId":Uuid::random(),"workspaceId":"workspace","terminalId":"terminal",
                "paneId":"pane","provider":"codex","nativeId":"T","title":"native lifecycle",
                "status":"working","connectivity":"ready",
                "capabilities":{"history":true,"prompt":true,"cancel":true,"respond":true,
                    "promptMode":"submit_or_steer","cancelMode":"native_turn"},
                "historyCoverage":{"scope":"thread","source":"native_pages",
                    "olderAvailable":true,"sourceTruncated":false,"desktopBranchKnown":false}},
            "nativeTurnId":"active-turn","desktopSelection":"bound_launch_thread",
            "activity":{"state":"working"}
        }))
        .unwrap();
        let epoch = state.begin_install(display.clone()).unwrap();
        let mut cache = History::default();
        // Pinned thread/items/list lifecycle fields distinguish an active item's cache
        // from a completed display record without inventing an end-of-replay marker.
        let page = json!({"data":[
            {"turnId":"past-turn","startedAtMs":1000,"completedAtMs":1001,
                "item":{"type":"userMessage","id":"past","content":[{"type":"text","text":"prior"}]}},
            {"turnId":"active-turn","startedAtMs":1002,"completedAtMs":null,
                "item":{"type":"agentMessage","id":"active","text":""}}
        ],"nextCursor":"opaque-original-owner-page"});
        let entries = page["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| {
                let item = safe_native(&entry["item"]);
                NativeRecord {
                    turn: native_id(entry, "turnId").unwrap(),
                    bytes: native_bytes(&item),
                    item,
                    order: 0,
                    complete: entry.get("completedAtMs").is_some_and(|v| !v.is_null()),
                    timestamp: native_timestamp(&entry["startedAtMs"]),
                    source: RecordSource::NativePage,
                    cached: true,
                    live_base: false,
                    fingerprint: None,
                    gap: None,
                }
            })
            .collect();
        let (records, older) = cache
            .page(&state, &target, entries, next_cursor(&page).unwrap(), true)
            .unwrap();
        assert!(older);
        assert_eq!(cache.older.as_deref(), Some("opaque-original-owner-page"));
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].record().id.0, "past");
        assert!(records[0].record().complete);
        let active = Id("active".into());
        let turn = Id("active-turn".into());
        let order = cache.records[&active].order;
        assert!(cache.records[&active].cached && cache.records[&active].live_base);
        assert!(!cache.records[&active].complete);
        state
            .finish_install(
                &epoch,
                ProviderSnapshot {
                    state: display,
                    pending: vec![],
                    history: records,
                    older_available: older,
                    native_older: None,
                },
            )
            .unwrap();

        let mut storage = Storage::memory([1; 32], 0).unwrap();
        let device = PairedDevice {
            device_id: Uuid::random(),
            endpoint_id: EndpointId(iroh::SecretKey::from_bytes(&[61; 32]).public().to_string()),
            device_name: Label::new("test phone").unwrap(),
            paired_at: Uint(0),
            last_seen_at: None,
        };
        let authorization = storage.pair_device(&device).unwrap();
        let peer = storage
            .authenticated_hello(&device.device_id, authorization)
            .unwrap();
        let started = json!({"type":"agentMessage","id":"active","text":""});
        assert!(matches!(
            project_item(
                &state,
                &target,
                &turn,
                &started,
                order,
                RecordSource::Live,
                false
            ),
            Err(ErrorCode::InvalidRequest)
        ));
        let update = cache
            .live_item(&state, &target, &turn, &started, (false, Some(Uint(1002))))
            .unwrap();
        assert!(matches!(&update, SessionUpdate::Activity(_)));
        state.update_from_source(&target, &epoch, update).unwrap();
        let delta =
            json!({"threadId":"T","turnId":"active-turn","itemId":"active","delta":"partial"});
        let update = cache
            .delta(&state, &target, "item/agentMessage/delta", &delta)
            .unwrap();
        assert!(matches!(&update, SessionUpdate::Activity(_)));
        state.update_from_source(&target, &epoch, update).unwrap();
        let working = state.session_state(&target, &epoch).unwrap();
        assert_eq!(working.activity.state, ActivityState::Working);
        assert!(matches!(&working.activity.text,
            Some(Content::Text(ContentText { text })) if text.0 == "partial"));
        let before = state
            .history(&peer, &target, None, 100, pairing::now_ms())
            .unwrap();
        assert_eq!(before.result.records.len(), 1);
        assert_eq!(before.result.records[0].id.0, "past");
        assert_eq!(cache.records[&active].item["text"], "partial");
        assert_eq!(cache.records[&active].order, order);
        assert!(!cache.records[&active].complete);

        let completed = json!({"type":"agentMessage","id":"active","text":"partial final"});
        // Both item/completed and a later turn/completed snapshot upsert the same native ID.
        for _ in 0..2 {
            let SessionUpdate::History(records) = cache
                .live_item(&state, &target, &turn, &completed, (true, None))
                .unwrap()
            else {
                panic!("completed item must enter history")
            };
            assert_eq!(records.len(), 1);
            assert!(records[0].record().complete);
            state.history_from_source(&target, &epoch, records).unwrap();
        }
        let history = state
            .history(&peer, &target, None, 100, pairing::now_ms())
            .unwrap();
        let final_records: Vec<_> = history
            .result
            .records
            .iter()
            .filter(|r| r.id == active)
            .collect();
        assert_eq!(final_records.len(), 1);
        assert!(matches!(&final_records[0].text,
            Some(Content::Text(ContentText { text })) if text.0 == "partial final"));
        assert_eq!(final_records[0].order, Uint(order));
        assert!(cache.records[&active].complete && !cache.records[&active].cached);

        // The publisher permits an active command item's aggregatedOutput to be
        // null. Its first item/commandExecution/outputDelta must initialize that
        // slot without changing the already-bound source or Target.
        let command = Id("command-null-output".into());
        let command_started = json!({
            "method":"item/started",
            "params":{
                "threadId":"T","turnId":"active-turn","startedAtMs":1003,
                "item":{"type":"commandExecution","id":"command-null-output",
                    "command":"printf command-output","cwd":"/workspace",
                    "status":"inProgress","commandActions":[],"aggregatedOutput":null}
            }
        });
        let update = cache
            .live_item(
                &state,
                &target,
                &turn,
                &command_started["params"]["item"],
                (
                    false,
                    native_timestamp(&command_started["params"]["startedAtMs"]),
                ),
            )
            .unwrap();
        state.update_from_source(&target, &epoch, update).unwrap();
        let command_delta = json!({
            "method":"item/commandExecution/outputDelta",
            "params":{"threadId":"T","turnId":"active-turn",
                "itemId":"command-null-output","delta":"command output"}
        });
        let update = cache
            .delta(
                &state,
                &target,
                command_delta["method"].as_str().unwrap(),
                &command_delta["params"],
            )
            .unwrap();
        state.update_from_source(&target, &epoch, update).unwrap();
        assert_eq!(
            cache.records[&command].item["aggregatedOutput"],
            "command output"
        );
        let mut malformed_started = command_started.clone();
        malformed_started["params"]["item"]["id"] = json!("command-invalid-output");
        malformed_started["params"]["item"]["aggregatedOutput"] = json!({"unexpected":true});
        let update = cache
            .live_item(
                &state,
                &target,
                &turn,
                &malformed_started["params"]["item"],
                (
                    false,
                    native_timestamp(&malformed_started["params"]["startedAtMs"]),
                ),
            )
            .unwrap();
        state.update_from_source(&target, &epoch, update).unwrap();
        let mut malformed_delta = command_delta.clone();
        malformed_delta["params"]["itemId"] = json!("command-invalid-output");
        let Err(failure) = cache.delta(
            &state,
            &target,
            malformed_delta["method"].as_str().unwrap(),
            &malformed_delta["params"],
        ) else {
            panic!("non-string command output must fail closed")
        };
        assert_eq!(failure, ErrorCode::NotReady);
        let command_completed = json!({
            "method":"item/completed",
            "params":{
                "threadId":"T","turnId":"active-turn","startedAtMs":1003,
                "item":{"type":"commandExecution","id":"command-null-output",
                    "command":"printf command-output","cwd":"/workspace",
                    "status":"completed","commandActions":[],
                    "aggregatedOutput":"command output","exitCode":0,"durationMs":1}
            }
        });
        let SessionUpdate::History(records) = cache
            .live_item(
                &state,
                &target,
                &turn,
                &command_completed["params"]["item"],
                (
                    true,
                    native_timestamp(&command_completed["params"]["startedAtMs"]),
                ),
            )
            .unwrap()
        else {
            panic!("completed command must enter history")
        };
        state.history_from_source(&target, &epoch, records).unwrap();
        assert!(cache.records[&command].complete);
        let current = state.session_state(&target, &epoch).unwrap();
        assert_eq!(current.summary.session_id, target.session_id);
        assert_eq!(current.summary.generation, target.generation);
        assert_eq!(current.native_turn_id, Some(turn));
        assert_eq!(cache.older.as_deref(), Some("opaque-original-owner-page"));
    }
    #[test]
    fn presence_failure_proof_classifies_validation_without_retaining_messages() {
        use codex_relay::{PresenceFailure, PresencePhase};
        for (message, guard) in [
            ("private parent absent", "private_parent"),
            ("oversized or aliased private Codex file", "private_size"),
            ("invalid private Codex record", "private_record"),
            (
                "private Codex publication did not stabilize",
                "publication_unstable",
            ),
            ("unsafe private path", "unsafe_private_path"),
            ("path is not UTF-8", "path_encoding"),
            ("stream did not contain valid UTF-8", "io_utf8"),
            ("listen parent absent", "socket_parent"),
            ("unknown Codex binding version", "binding_schema"),
            (
                "relay host/provider parent object proof unavailable",
                "binding_parent",
            ),
            ("invalid boot identity", "owner_boot"),
            ("Codex owner process/source replaced", "owner_identity"),
            ("native process number overflow", "process_number"),
            (
                "No verified Codex publisher distribution for this platform",
                "distribution_platform",
            ),
            (
                "Codex process changed before executable proof",
                "distribution_process",
            ),
            (
                "Kernel-opened Codex executable/install/process-root objects disagree",
                "distribution_object",
            ),
            ("Codex native executable is not ELF", "distribution_elf"),
            (
                "Codex executable differs from verified source-linked publisher distribution",
                "distribution_digest",
            ),
            (
                "Codex executable/process changed during digest proof",
                "distribution_changed",
            ),
            ("invalid provider pathname", "native_pathname"),
            ("owner process root changed", "owner_root"),
            ("unsafe native socket directory", "socket_directory"),
            ("unsafe native rendezvous alias", "socket_alias"),
            (
                "native rendezvous target is not the exact stock hash",
                "socket_alias_target",
            ),
            ("unsafe Codex socket", "socket_type"),
            (
                "advertised host/provider alias objects disagree",
                "socket_mapping",
            ),
            ("native physical socket epoch replaced", "socket_epoch"),
            (
                "native alias/root/parent source epoch replaced",
                "socket_source",
            ),
            (
                "visible process PID disagrees with proc namespace view",
                "process_namespace",
            ),
            ("process start ticks unavailable", "process_stat"),
            ("actual caller PID unavailable", "process_pid"),
            ("process UID mismatch", "process_uid"),
            ("Linux boot identity unavailable", "process_boot"),
            (
                "process changed or procfs PID view unavailable during evidence capture",
                "process_changed",
            ),
            (
                "absolute control-free path required: /private/key\nsecret",
                "path_policy",
            ),
            (
                "Unix socket path exceeds 107 UTF-8 bytes: /private/key",
                "socket_path",
            ),
            (
                "unsafe installed executable: /private/key",
                "installed_object",
            ),
            ("unsafe executable parent: /private/key", "installed_object"),
            (
                "installed object changed during trust proof",
                "installed_object",
            ),
            (
                "private Codex publication did not stabilize /private/key",
                "unknown",
            ),
            (
                "private native message https://private.invalid/token",
                "unknown",
            ),
        ] {
            let failure = PresenceFailure {
                phase: PresencePhase::PresenceRecord,
                error: io::Error::new(io::ErrorKind::InvalidData, message),
            };
            assert_eq!(
                presence_failure_detail(&failure.error),
                (guard, io::ErrorKind::InvalidData, None)
            );
            let proof = launch_failure_proof(
                "manifest_presence",
                ErrorCode::NotReady,
                Some(presence_failure_detail(&failure.error)),
            );
            assert!(!proof.contains(message));
            assert!(!proof.contains("/private/") && !proof.contains("secret"));
        }
        // Exercise the real bounded native parser, not a fabricated parser error string.
        for bytes in [br#"{"duplicate":1,"duplicate":2}"#.as_slice(), &[0xff], b""] {
            let error = native_json::parse(bytes, native_json::REGISTRY).unwrap_err();
            assert_eq!(
                presence_failure_detail(&error),
                ("native_json", io::ErrorKind::InvalidData, None)
            );
        }
        for phase in [
            PresencePhase::Guard,
            PresencePhase::PresenceRecord,
            PresencePhase::RelayProcess,
            PresencePhase::TuiProcess,
            PresencePhase::TuiDistribution,
            PresencePhase::TuiAgentSocket,
            PresencePhase::HostListenSocket,
            PresencePhase::ListenParent,
            PresencePhase::OwnerProof,
            PresencePhase::HostParent,
            PresencePhase::TerminalProcess,
            PresencePhase::TerminalParentObject,
        ] {
            let expected = phase.as_str();
            let failure = PresenceFailure {
                phase,
                error: io::Error::from_raw_os_error(libc::ENOENT),
            };
            assert_eq!(failure.phase.as_str(), expected);
            assert_eq!(
                presence_failure_detail(&failure.error),
                ("unknown", io::ErrorKind::NotFound, Some(libc::ENOENT))
            );
        }
    }

    #[test]
    fn presence_uncertainty_proof_retains_only_exact_category_kind_and_errno() {
        for (message, category) in [
            (
                "Codex relay presence older than five seconds",
                "presence_past_5s",
            ),
            (
                "Codex relay presence more than one second ahead",
                "presence_future_1s",
            ),
            ("TUI process replaced", "tui_replaced"),
            (
                "Codex relay presence older than five seconds /private/native-T",
                "unknown",
            ),
            (
                "private path /private/native-T and native payload",
                "unknown",
            ),
        ] {
            let error = io::Error::new(io::ErrorKind::InvalidData, message);
            let detail = presence_failure_detail(&error);
            assert_eq!(detail, (category, io::ErrorKind::InvalidData, None));
            let code = policy(&error);
            assert_eq!(code, ErrorCode::NotReady);
            let proof = Id(launch_failure_proof(
                "manifest_presence",
                code,
                Some(detail),
            ));
            proof.validate().unwrap();
            assert!(proof.0.contains(&format!(
                "presence={category}; io_kind=InvalidData; errno=None"
            )));
            assert!(!proof.0.contains(message));
            assert!(!proof.0.contains("/private/") && !proof.0.contains("native-T"));
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        }
        let error = io::Error::from_raw_os_error(libc::ENOENT);
        let detail = presence_failure_detail(&error);
        assert_eq!(detail, ("unknown", error.kind(), Some(libc::ENOENT)));
        let proof = Id(launch_failure_proof(
            "manifest_presence",
            policy(&error),
            Some(detail),
        ));
        proof.validate().unwrap();
        assert!(
            proof
                .0
                .contains("presence=unknown; io_kind=NotFound; errno=Some(")
        );
        assert!(!proof.0.contains(&error.to_string()));
        assert_eq!(error.raw_os_error(), Some(libc::ENOENT));
    }
    #[test]
    fn public_creation_read_requires_observed_paginated_original_thread_and_zero_turns() {
        let expected = Id("T".into());
        let value = native_json::parse(
            br#"{"thread":{"id":"T","historyMode":"paginated","turns":[]}}"#,
            NATIVE,
        )
        .unwrap();
        assert_eq!(
            paginated_zero_turn_thread(&value["thread"], &expected),
            Ok(())
        );
        let mut legacy = value["thread"].clone();
        legacy["historyMode"] = json!("legacy");
        assert!(paginated_zero_turn_thread(&legacy, &expected).is_err());
        legacy["historyMode"] = Value::Null;
        assert!(paginated_zero_turn_thread(&legacy, &expected).is_err());
        legacy.as_object_mut().unwrap().remove("historyMode");
        assert!(paginated_zero_turn_thread(&legacy, &expected).is_err());
        let mut substitute = value["thread"].clone();
        substitute["id"] = json!("other");
        assert!(paginated_zero_turn_thread(&substitute, &expected).is_err());
        let mut nonzero = value["thread"].clone();
        nonzero["turns"] = json!([{"id":"turn"}]);
        assert!(paginated_zero_turn_thread(&nonzero, &expected).is_err());
    }
    #[test]
    fn native_owner_candidate_preserves_wrapper_prefix_and_requires_exact_app_server_tail() {
        let expected = vec![
            "app-server".to_owned(),
            "--listen".to_owned(),
            "unix:///owned/c.sock".to_owned(),
        ];
        let mut argv = vec!["/installed/native/codex".to_owned()];
        argv.extend(expected.iter().cloned());
        assert!(native_owner_arguments(&argv, &expected));
        let mut prefixed = vec![
            "/installed/native/codex".to_owned(),
            "-c".to_owned(),
            "sandbox_mode=\"danger-full-access\"".to_owned(),
        ];
        prefixed.extend(expected.iter().cloned());
        assert!(native_owner_arguments(&prefixed, &expected));
        let mut extra = argv.clone();
        extra.push("--another-mode".to_owned());
        assert!(!native_owner_arguments(&extra, &expected));
        let mut other_owner = argv;
        other_owner[3] = "unix:///foreign/c.sock".to_owned();
        assert!(!native_owner_arguments(&other_owner, &expected));
        assert!(!native_owner_arguments(&expected, &expected));
    }
    #[test]
    fn manager_native_socket_permissions_admit_creation_without_weakening_codex_sockets() {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
        struct Root(std::path::PathBuf);
        impl Drop for Root {
            fn drop(&mut self) {
                std::fs::remove_dir_all(&self.0).unwrap();
            }
        }
        let root = Root(std::env::temp_dir().join(format!("gm-{}", &Uuid::random().0[..8])));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&root.0)
            .unwrap();
        let parent = root.0.join("systemd");
        std::fs::DirBuilder::new()
            .mode(0o755)
            .create(&parent)
            .unwrap();
        let path = parent.join("private");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let metadata = std::fs::symlink_metadata(&path).unwrap();
        // Native systemd binds under umask0077; this is not a Codex0600 socket.
        assert_eq!(
            manager_socket_object(&path).unwrap(),
            (metadata.dev(), metadata.ino())
        );
        assert!(codex_relay::socket_object(&path).is_err());
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(manager_socket_object(&path).is_err());
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(manager_socket_object(&path).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let alias = parent.join("alias");
        std::os::unix::fs::symlink(&path, &alias).unwrap();
        assert!(manager_socket_object(&alias).is_err());
        let parent_alias = root.0.join("alias");
        std::os::unix::fs::symlink(&parent, &parent_alias).unwrap();
        assert!(manager_socket_object(&parent_alias.join("private")).is_err());
        drop(listener);
        std::fs::remove_file(&path).unwrap();
        std::fs::write(&path, []).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(manager_socket_object(&path).is_err());
    }
    #[tokio::test]
    async fn malformed_first_response_consumes_typed_callback_and_cannot_later_acknowledge() {
        let (tx, mut rx) = oneshot::channel();
        let callbacks = Mutex::new(vec![Callback {
            response_limit: 16 * 1024 * 1024,
            response_bytes: None,
            id: NativeRpcId::from_raw("-123456789012345678901234567890").unwrap(),
            reply: tx,
            snapshot: true,
            read_rejection: None,
            thread_start: None,
        }]);
        let boundary = AtomicU64::new(0);
        let wrong=native_json::parse(br#"{"id":"-123456789012345678901234567890","result":{"turn":{"id":"T","items":[],"status":"inProgress"}}}"#,NATIVE).unwrap();
        complete_callback(&callbacks, &wrong, 1, &boundary).unwrap();
        assert!(matches!(
            rx.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        let malformed = native_json::parse(
            br#"{"id":-123456789012345678901234567890,"result":null}"#,
            NATIVE,
        )
        .unwrap();
        complete_callback(&callbacks, &malformed, 2, &boundary).unwrap();
        let reply = rx.await.unwrap().unwrap();
        assert!(turn_admission(&reply).is_err());
        assert!(callbacks.lock().is_empty());
        assert_eq!(boundary.load(Ordering::Acquire), 2);
        let late=native_json::parse(br#"{"id":-123456789012345678901234567890,"result":{"turn":{"id":"T","items":[],"status":"inProgress"}}}"#,NATIVE).unwrap();
        complete_callback(&callbacks, &late, 3, &boundary).unwrap();
        assert_eq!(boundary.load(Ordering::Acquire), 2);
    }
    #[tokio::test]
    async fn malformed_title_acknowledgement_cannot_admit_read_or_be_repaired_by_late_reply() {
        for result in [
            Value::Null,
            json!([]),
            json!({"name":"unexpected"}),
            json!({"threadId":"other-T"}),
        ] {
            let (reply, mut rx) = oneshot::channel();
            let callbacks = Mutex::new(vec![Callback {
                response_limit: 16 * 1024 * 1024,
                response_bytes: None,
                id: NativeRpcId::from_raw("-71").unwrap(),
                reply,
                snapshot: false,
                read_rejection: None,
                thread_start: None,
            }]);
            let boundary = AtomicU64::new(0);
            complete_callback(&callbacks, &json!({"id":"-71","result":{}}), 1, &boundary).unwrap();
            assert!(matches!(
                rx.try_recv(),
                Err(oneshot::error::TryRecvError::Empty)
            ));
            complete_callback(&callbacks, &json!({"id":-71,"result":result}), 2, &boundary)
                .unwrap();
            let acknowledged = rx.await.unwrap().unwrap();
            assert_eq!(
                native_name_set_admission(&acknowledged),
                Err(ErrorCode::NotReady)
            );
            assert!(callbacks.lock().is_empty());
            // The original title response is consumed; a late empty result cannot authorize read.
            complete_callback(&callbacks, &json!({"id":-71,"result":{}}), 3, &boundary).unwrap();
            assert!(callbacks.lock().is_empty());
            assert_eq!(boundary.load(Ordering::Acquire), 0);
        }
        let (reply, rx) = oneshot::channel();
        let callbacks = Mutex::new(vec![Callback {
            response_limit: 16 * 1024 * 1024,
            response_bytes: None,
            id: NativeRpcId::String("original-title".into()),
            reply,
            snapshot: false,
            read_rejection: None,
            thread_start: None,
        }]);
        complete_callback(
            &callbacks,
            &json!({"id":"original-title","result":{}}),
            1,
            &AtomicU64::new(0),
        )
        .unwrap();
        assert_eq!(
            native_name_set_admission(&rx.await.unwrap().unwrap()),
            Ok(())
        );
    }
    #[tokio::test]
    async fn original_creation_read_rejection_preserves_only_correlated_numeric_static_proof() {
        let expected = Id("private-original-native-T".into());
        for (code, message, category) in [
            (
                -32600,
                "ephemeral threads do not support includeTurns".into(),
                "ephemeral_include_turns",
            ),
            (
                -32600,
                format!("thread not loaded: {}", expected.0),
                "thread_not_loaded",
            ),
            (
                -32600,
                format!(
                    "thread {} is not materialized yet; includeTurns is unavailable before first user message",
                    expected.0
                ),
                "include_turns_unmaterialized",
            ),
            (
                -32600,
                format!(
                    "thread {} is not materialized yet; thread/turns/list is unavailable before first user message",
                    expected.0
                ),
                "full_turns_unmaterialized",
            ),
            (
                -32600,
                "thread not loaded: different-native-T".into(),
                "unclassified_rejection",
            ),
            (
                -32602,
                "unknown params with private native details".into(),
                "unclassified_rejection",
            ),
            (
                -32601,
                "unknown method with private native details".into(),
                "unclassified_rejection",
            ),
        ] {
            let (reply, mut rx) = oneshot::channel();
            let (rejection_tx, mut rejection_rx) = oneshot::channel();
            let callbacks = Mutex::new(vec![Callback {
                response_limit: 16 * 1024 * 1024,
                response_bytes: None,
                id: NativeRpcId::from_raw("-42").unwrap(),
                reply,
                snapshot: false,
                read_rejection: Some((expected.clone(), rejection_tx)),
                thread_start: None,
            }]);
            let boundary = AtomicU64::new(0);
            let error = json!({"code": code, "message": message, "data": {"private": "native-secret-data"}});
            complete_callback(
                &callbacks,
                &json!({"id": "-42", "error": error}),
                1,
                &boundary,
            )
            .unwrap();
            assert!(matches!(
                rx.try_recv(),
                Err(oneshot::error::TryRecvError::Empty)
            ));
            assert!(matches!(
                rejection_rx.try_recv(),
                Err(oneshot::error::TryRecvError::Empty)
            ));
            complete_callback(
                &callbacks,
                &json!({"id": -42, "error": error}),
                2,
                &boundary,
            )
            .unwrap();
            assert_eq!(rx.await.unwrap(), Err(ErrorCode::BackendError));
            let rejection = rejection_rx.await.unwrap();
            assert_eq!(rejection, CreationReadRejection { code, category });
            let proof = rejection.proof();
            assert_eq!(
                proof,
                format!(
                    "Native original-thread materializing read rejected; code={code}; category={category}; no fallback",
                )
            );
            for private in [
                expected.0.as_str(),
                message.as_str(),
                "different-native-T",
                "native-secret-data",
                "-42",
            ] {
                assert!(!proof.contains(private));
            }
            assert!(callbacks.lock().is_empty());
            assert_eq!(boundary.load(Ordering::Acquire), 0);
            complete_callback(
                &callbacks,
                &json!({"id": -42, "result": {"thread": {"id": "late-T"}}}),
                3,
                &boundary,
            )
            .unwrap();
            assert_eq!(boundary.load(Ordering::Acquire), 0);
        }
        for error in [
            Value::Null,
            json!({"code": false, "message": "ephemeral threads do not support includeTurns"}),
            json!({"code": -32603, "message": "ephemeral threads do not support includeTurns"}),
            json!({"code": 123, "message": "ephemeral threads do not support includeTurns"}),
            json!({"code": -32600}),
        ] {
            let (reply, rx) = oneshot::channel();
            let (rejection_tx, rejection_rx) = oneshot::channel();
            let callbacks = Mutex::new(vec![Callback {
                response_limit: 16 * 1024 * 1024,
                response_bytes: None,
                id: NativeRpcId::String("original-read".into()),
                reply,
                snapshot: false,
                read_rejection: Some((expected.clone(), rejection_tx)),
                thread_start: None,
            }]);
            assert!(
                complete_callback(
                    &callbacks,
                    &json!({"id": "original-read", "error": error}),
                    1,
                    &AtomicU64::new(0)
                )
                .is_err()
            );
            assert_eq!(rx.await.unwrap(), Err(ErrorCode::NotReady));
            assert!(rejection_rx.await.is_err());
            assert!(callbacks.lock().is_empty());
        }
    }
    #[tokio::test]
    async fn malformed_or_internal_error_is_unknown_not_definitive_no_effect() {
        for error in [
            Value::Null,
            json!({"code":false,"message":"bad"}),
            json!({"code":-32603,"message":"internal after possible admission"}),
        ] {
            let (tx, rx) = oneshot::channel();
            let callbacks = Mutex::new(vec![Callback {
                response_limit: 16 * 1024 * 1024,
                response_bytes: None,
                id: NativeRpcId::String("original".into()),
                reply: tx,
                snapshot: false,
                read_rejection: None,
                thread_start: None,
            }]);
            let frame = json!({"id":"original","error":error});
            assert!(complete_callback(&callbacks, &frame, 1, &AtomicU64::new(0)).is_err());
            assert_eq!(rx.await.unwrap(), Err(ErrorCode::NotReady));
            assert!(callbacks.lock().is_empty());
        }
        assert_eq!(
            ordinary_response(
                &json!({"error":{"code":-32602,"message":"pre-admission params rejection"}})
            ),
            Err(ErrorCode::BackendError)
        );
    }
    #[test]
    fn continuation_eviction_never_aliases_original_boundary_to_successor() {
        let mut history = History::default();
        let value = |before| Continuation::seed(Some(Id(format!("anchor-{before}"))), before);
        let first = history.continuation(value(100)).unwrap();
        assert_eq!(history.continuation(value(100)).unwrap(), first);
        for before in 101..164 {
            history.continuation(value(before)).unwrap();
        }
        assert_eq!(history.continuations.len(), 64);
        assert_eq!(history.continuation(value(100)).unwrap(), first);
        let successor = history.continuation(value(164)).unwrap();
        assert_eq!(history.continuations.len(), 1);
        assert!(!history.continuations.contains_key(&first));
        assert_ne!(successor, first);
        // Recreating the same boundary creates an unrelated provider ticket; the
        // State-pinned original token still references the explicitly expired ID.
        let recreated = history.continuation(value(100)).unwrap();
        assert_ne!(recreated, first);
        assert!(!history.continuations.contains_key(&first));
    }
    fn walk_turn(id: &str, items: &[&str]) -> Value {
        json!({"id":id,"status":"completed","itemsView":"full","items":items.iter()
            .map(|id| json!({"id":id,"type":"agentMessage","text":"fixture"})).collect::<Vec<_>>()})
    }
    fn walk_ids(walk: &Continuation) -> Vec<&str> {
        walk.remaining
            .iter()
            .map(|r| r.item["id"].as_str().unwrap())
            .collect()
    }
    #[test]
    fn history_walk_bounds_seek_and_fallback_reads_preserving_native_order() {
        let thread = Id("thread".into());
        let mut walk = Continuation::seed(Some(Id("anchor".into())), 100);
        assert_eq!(walk.request(&thread).0, "thread/turns/list");
        walk.accept_page(&json!({"data":[walk_turn("new", &["new-item"])],"nextCursor":"turns-2"}))
            .unwrap();
        assert!(walk.remaining.is_empty() && walk.has_more());
        assert_eq!(walk.request(&thread).1["cursor"], "turns-2");
        // The turns response consumes this request's only page. Hydration starts
        // only on the next request, ahead of the older full turn in this page.
        walk.accept_page(
            &json!({"data":[{"id":"fallback","itemsView":"summary","status":"completed"},
            walk_turn("old", &["oldest", "older"])],"nextCursor":"turns-3"}),
        )
        .unwrap();
        assert!(walk.remaining.is_empty());
        let (method, params) = walk.request(&thread);
        assert_eq!(method, "thread/items/list");
        assert_eq!(params["turnId"], "fallback");
        assert_eq!(params["sortDirection"], "desc");
        walk.accept_page(&json!({"data":[{"turnId":"fallback","item":{"id":"newer","type":"agentMessage","text":"fixture"},
            "startedAtMs":1,"completedAtMs":2}],"nextCursor":"items-2"})).unwrap();
        assert!(walk.remaining.is_empty());
        assert_eq!(walk.request(&thread).1["cursor"], "items-2");
        walk.accept_page(&json!({"data":[
            {"turnId":"fallback","item":{"id":"anchor","type":"agentMessage","text":"fixture"},"completedAtMs":3},
            {"turnId":"fallback","item":{"id":"before-anchor","type":"agentMessage","text":"fixture"},"startedAtMs":1,"completedAtMs":2}
        ],"nextCursor":null})).unwrap();
        assert_eq!(walk_ids(&walk), ["before-anchor", "older", "oldest"]);
        assert!(walk.anchor.is_none());
        assert_eq!(walk.remaining[0].timestamp, Some(Uint(1)));
        assert!(
            walk.remaining
                .iter()
                .all(|r| r.complete && r.source == RecordSource::NativePage)
        );
        // A display slice is ascending; its next native read resumes the opaque
        // turns cursor, never restarts at newest or substitutes a cached page.
        assert_eq!(
            walk.remaining
                .iter()
                .rev()
                .map(|r| r.item["id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["oldest", "older", "before-anchor"]
        );
        walk.offset = walk.remaining.len();
        assert_eq!(walk.request(&thread).0, "thread/turns/list");
        assert_eq!(walk.request(&thread).1["cursor"], "turns-3");
    }
    #[test]
    fn history_walk_rejects_cross_page_cycles_wrong_turn_and_missing_anchor() {
        let mut walk = Continuation::seed(Some(Id("anchor".into())), 100);
        walk.accept_page(&json!({"data":[],"nextCursor":"a"}))
            .unwrap();
        let original = walk.clone();
        walk.accept_page(&json!({"data":[],"nextCursor":"b"}))
            .unwrap();
        assert_eq!(
            walk.accept_page(&json!({"data":[],"nextCursor":"a"})),
            Err(ErrorCode::NotReady)
        );
        assert_eq!(original.cursor.as_deref(), Some("a"));
        assert_eq!(original.seen_turns.len(), 1);
        let mut lost = original.clone();
        assert_eq!(
            lost.accept_page(&json!({"data":[],"nextCursor":null})),
            Err(ErrorCode::CursorExpired)
        );
        let mut items = Continuation::seed(None, 100);
        items
            .accept_page(&json!({"data":[{"id":"turn","itemsView":"summary"}],"nextCursor":null}))
            .unwrap();
        assert_eq!(
            items
                .clone()
                .accept_page(&json!({"data":[{"turnId":"foreign","item":{"id":"item"}}]})),
            Err(ErrorCode::NotReady)
        );
        items
            .accept_page(&json!({"data":[],"nextCursor":"items-a"}))
            .unwrap();
        items
            .accept_page(&json!({"data":[],"nextCursor":"items-b"}))
            .unwrap();
        assert_eq!(
            items.accept_page(&json!({"data":[],"nextCursor":"items-a"})),
            Err(ErrorCode::NotReady)
        );
    }
    #[test]
    fn history_walk_budget_counts_hydration_and_cycle_metadata_without_orphan() {
        let mut history = History::default();
        let seed = Continuation::seed(Some(Id("anchor".into())), 100);
        let first = history.continuation(seed.clone()).unwrap();
        let mut walk = seed;
        walk.accept_page(&json!({"data":[{"id":"fallback","itemsView":"summary"},walk_turn("old", &["old-item"])],
            "nextCursor":"native-cursor"})).unwrap();
        let size = walk.storage_bytes(
            &mut HashSet::new(),
            &mut HashSet::new(),
            &mut HashSet::new(),
        );
        assert!(size > 256 + "native-cursor".len());
        assert_eq!(
            history.continuation_with_budget(walk.clone(), size - 1),
            Err(ErrorCode::ResourceLimit)
        );
        assert_eq!(history.continuations.len(), 1);
        assert!(history.continuations.contains_key(&first));
        let next = history.continuation_with_budget(walk, size).unwrap();
        assert_eq!(history.continuations.len(), 1);
        assert_ne!(first, next);
        assert!(!history.continuations.contains_key(&first));
    }
    #[test]
    fn history_walk_empty_successor_is_stable_peer_scoped_and_expires() {
        use crate::state::{CONTENT_TTL_MS, NativePageLookup};
        use crate::storage::{PairedDevice, Storage};
        let (state, target, epoch) = publication_observation_state();
        let mut storage = Storage::memory([1; 32], 0).unwrap();
        let device = PairedDevice {
            device_id: Uuid::random(),
            endpoint_id: EndpointId(iroh::SecretKey::from_bytes(&[61; 32]).public().to_string()),
            device_name: Label::new("test phone").unwrap(),
            paired_at: Uint(0),
            last_seen_at: None,
        };
        let authorization = storage.pair_device(&device).unwrap();
        let peer = storage
            .authenticated_hello(&device.device_id, authorization)
            .unwrap();
        let other_device = PairedDevice {
            device_id: Uuid::random(),
            endpoint_id: EndpointId(iroh::SecretKey::from_bytes(&[62; 32]).public().to_string()),
            device_name: Label::new("other phone").unwrap(),
            paired_at: Uint(0),
            last_seen_at: None,
        };
        let other_authorization = storage.pair_device(&other_device).unwrap();
        let other = storage
            .authenticated_hello(&other_device.device_id, other_authorization)
            .unwrap();
        let mut history = History::default();
        let seed = history
            .continuation(Continuation::seed(Some(Id("anchor".into())), 100))
            .unwrap();
        let cursor = state
            .native_history_cursor(&peer, &target, &epoch, seed, None, 0)
            .unwrap();
        assert!(matches!(
            state.native_history_page(&other, &target, &cursor, 100, 0),
            Err(ErrorCode::CursorExpired)
        ));
        let NativePageLookup::Fetch(ticket) = state
            .native_history_page(&peer, &target, &cursor, 100, 0)
            .unwrap()
        else {
            panic!("native fetch");
        };
        let mut walk = history.continuations[ticket.native_cursor()].clone();
        walk.accept_page(
            &json!({"data":[walk_turn("new", &["new-item"])],"nextCursor":"turns-next"}),
        )
        .unwrap();
        let native_next = history.continuation(walk).unwrap();
        let page = state
            .finish_native_history_page(ticket, vec![], Some(native_next.clone()), 0)
            .unwrap();
        assert!(page.records.is_empty());
        let successor = page.next_cursor.clone().unwrap();
        assert_ne!(successor, cursor);
        let NativePageLookup::Cached(repeat) = state
            .native_history_page(&peer, &target, &cursor, 100, 1)
            .unwrap()
        else {
            panic!("repeat must not refetch");
        };
        assert_eq!(page, repeat);
        let NativePageLookup::Fetch(next) = state
            .native_history_page(&peer, &target, &successor, 100, 1)
            .unwrap()
        else {
            panic!("next native read");
        };
        assert_eq!(next.native_cursor(), &native_next);
        assert_eq!(
            history.continuations[next.native_cursor()]
                .cursor
                .as_deref(),
            Some("turns-next")
        );
        assert!(matches!(
            state.native_history_page(&peer, &target, &cursor, 100, CONTENT_TTL_MS),
            Err(ErrorCode::CursorExpired)
        ));
        state
            .binding_offline(&target, &epoch, "fixture source loss")
            .unwrap();
        assert!(matches!(
            state.native_history_page(&peer, &target, &successor, 100, 2),
            Err(ErrorCode::CursorExpired)
        ));
    }
    #[test]
    fn completed_cache_keeps_identity_and_exact_refetch_fingerprint_not_another_history_body() {
        let mut cache = History::default();
        let body = json!({"id":"item","type":"agentMessage","text":"completed native body"});
        cache.retain(
            Id("turn".into()),
            body.clone(),
            42,
            true,
            RecordSource::Live,
        );
        let record = cache.records.get(&Id("item".into())).unwrap();
        assert_eq!(record.order, 42);
        assert!(!record.cached);
        assert!(record.item.get("text").is_none());
        assert_eq!(
            record.fingerprint,
            Some(native_fingerprint(&safe_native(&body)))
        );
        cache.retain(
            Id("turn".into()),
            json!({"id":"item","type":"agentMessage","text":"older page"}),
            1,
            true,
            RecordSource::NativePage,
        );
        assert_eq!(
            cache.records[&Id("item".into())].fingerprint,
            Some(native_fingerprint(&safe_native(&body)))
        );
    }
    #[test]
    fn kernel_service_arguments_are_exact_values_not_display_substring_matches() {
        let expected = vec![
            "/installed/outer wrapper".into(),
            "app-server".into(),
            "--listen".into(),
            "unix:///private/c.sock".into(),
        ];
        let property = json!({"type":"a(sasbttttuii)","data":[["/installed/outer wrapper",expected,false,0,0,0,0,12,0,0]]});
        exact_service_argv(&property, &expected).unwrap();
        let mut substituted = property.clone();
        substituted["data"][0][1][3] = json!("unix:///foreign/c.sock");
        assert!(exact_service_argv(&substituted, &expected).is_err());
        let mut multiple = property.clone();
        multiple["data"]
            .as_array_mut()
            .unwrap()
            .push(property["data"][0].clone());
        assert!(exact_service_argv(&multiple, &expected).is_err());
        let mut truncated = property.clone();
        truncated["data"][0].as_array_mut().unwrap().pop();
        assert!(exact_service_argv(&truncated, &expected).is_err());
        let nested = json!({"type":"a(sasbttttuii)","data":[property["data"]]});
        assert!(exact_service_argv(&nested, &expected).is_err());
        let environment = json!({"type":"as","data":["PATH=first","HOME=/owned"]});
        assert_eq!(
            service_environment(&environment).unwrap(),
            vec![
                ("HOME".to_owned(), "/owned".to_owned()),
                ("PATH".to_owned(), "first".to_owned())
            ],
        );
        let duplicate = json!({"type":"as","data":["PATH=first","PATH=second"]});
        assert!(service_environment(&duplicate).is_err());
        let nested = json!({"type":"as","data":[["PATH=first"]]});
        assert!(service_environment(&nested).is_err());
    }
    #[test]
    fn owner_launcher_public_ancestry_accepts_child_and_rejects_replacement() {
        const CHILD: &str = "GOATR_TEST_OWNER_ANCESTRY";
        if let Ok(payload) = std::env::var(CHILD) {
            let (manager, unrelated): (ProcessEvidence, ProcessEvidence) =
                serde_json::from_str(&payload).unwrap();
            crate::codex_relay::drop_inherited_capabilities().unwrap();
            let own = herdr::process_evidence(std::process::id()).unwrap();
            let peer = (manager.pid, pairing::uid());
            owner_manager_scope(&own, peer, &manager).unwrap();
            let mut changed = manager.clone();
            changed.start_ticks += 1;
            assert!(owner_manager_scope(&own, peer, &changed).is_err());
            changed = manager.clone();
            changed.linux_boot_id = Uuid::random();
            assert!(owner_manager_scope(&own, peer, &changed).is_err());
            assert!(
                owner_manager_scope(&own, (manager.pid, pairing::uid() + 1), &manager).is_err()
            );
            assert!(
                owner_manager_scope(&own, (manager.pid + 1, pairing::uid()), &manager).is_err()
            );
            changed = manager.clone();
            changed.namespaces.pid.1 += 1;
            assert!(owner_manager_scope(&own, peer, &changed).is_err());
            assert!(
                owner_manager_scope(&own, (unrelated.pid, pairing::uid()), &unrelated).is_err()
            );
            return;
        }
        // The sibling stays alive on a private stdin pipe, without invoking a provider.
        let mut sibling = std::process::Command::new("/bin/sh")
            .args(["-c", "read -r unused"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let manager = herdr::process_evidence(std::process::id()).unwrap();
        let unrelated = herdr::process_evidence(sibling.id()).unwrap();
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "backends::codex::tests::owner_launcher_public_ancestry_accepts_child_and_rejects_replacement", "--nocapture"])
            .env(CHILD, serde_json::to_string(&(manager, unrelated)).unwrap())
            .output();
        drop(sibling.stdin.take());
        sibling.wait().unwrap();
        let result = result.unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    #[test]
    fn owner_launcher_service_arguments_require_the_exact_record_path() {
        let launcher = Path::new("/nix/store/owner-launcher/bin/goatr");
        let record = Path::new("/owned/runtime/b/owner-00000000-0000-4000-8000-000000000000.owner");
        let expected = owner_launcher_argv(launcher, record).unwrap();
        assert_eq!(
            expected,
            vec![
                "/nix/store/owner-launcher/bin/goatr",
                "codex-owner",
                "--record",
                "/owned/runtime/b/owner-00000000-0000-4000-8000-000000000000.owner",
            ]
        );
        let property =
            json!({"type":"a(sasbttttuii)","data":[[expected[0],expected,false,0,0,0,0,12,0,0]]});
        exact_service_argv(&property, &owner_launcher_argv(launcher, record).unwrap()).unwrap();
        let mut substituted = property;
        substituted["data"][0][1][3] = json!("/owned/runtime/b/owner-foreign.owner");
        assert!(
            exact_service_argv(
                &substituted,
                &owner_launcher_argv(launcher, record).unwrap()
            )
            .is_err()
        );
    }
    #[test]
    fn owner_launcher_record_shape_rejects_old_and_substituted_fields() {
        fn record() -> OwnerRecord {
            OwnerRecord {
                v: 2,
                owner_id: Uuid::new("00000000-0000-4000-8000-000000000000").unwrap(),
                action_id: Uuid::random(),
                unit: "goatr-owner.service".into(),
                launcher: PathBuf::from("/nix/store/owner/bin/goatr"),
                wrapper: PathBuf::from("/nix/store/wrapper/bin/wrapper"),
                cwd: PathBuf::from("/owned/project"),
                socket_agent: PathBuf::from("/owned/c.sock"),
                argv: vec![
                    "app-server".into(),
                    "--listen".into(),
                    "unix:///owned/c.sock".into(),
                ],
                environment_digest: "digest".into(),
                manager: ProcessEvidence {
                    linux_boot_id: Uuid::random(),
                    pid: 7,
                    start_ticks: 8,
                    namespaces: herdr::ProcessNamespaces {
                        pid: (1, 2),
                        mount: (3, 4),
                        user: (5, 6),
                        nspids: vec![7],
                    },
                },
                service: None,
                epoch: None,
            }
        }
        let path = Path::new("/owned/owner-00000000-0000-4000-8000-000000000000.owner");
        let current = Path::new("/nix/store/owner/bin/goatr");
        assert!(owner_exec_record_shape(&record(), path, current).is_ok());
        for change in [
            |r: &mut OwnerRecord| r.v = 1,
            |r: &mut OwnerRecord| r.launcher = PathBuf::from("/nix/store/foreign/bin/goatr"),
            |r: &mut OwnerRecord| r.socket_agent = PathBuf::from("relative.sock"),
            |r: &mut OwnerRecord| r.argv.push("--foreign".into()),
            |r: &mut OwnerRecord| r.unit = "bad unit".into(),
        ] {
            let mut changed = record();
            change(&mut changed);
            assert!(owner_exec_record_shape(&changed, path, current).is_err());
        }
    }
    #[test]
    fn provided_native_millisecond_times_do_not_round_or_invent_missing_dates() {
        for (raw, expected) in [
            ("1700000000123", Some(Uint(1700000000123))),
            ("-0", Some(Uint(0))),
            ("-1", None),
            ("1700000000123.0", None),
            ("true", None),
            ("null", None),
            ("9007199254740992", None),
        ] {
            let value = native_json::parse(raw.as_bytes(), NATIVE).unwrap();
            assert_eq!(native_timestamp(&value), expected);
        }
    }
}
