// SPDX-License-Identifier: GPL-3.0-or-later
//! Closed Goatr v2 boundary. Decode bytes before dispatch; never deserialize a frame directly.
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    io::{Read, Write},
};

#[path = "protocol_decimal.rs"]
mod decimal;
#[path = "protocol_model.rs"]
mod model;
pub use decimal::{Decimal, NativeRpcId};
pub use model::*;
#[path = "protocol_forms.rs"]
mod forms;
use forms::*;
pub use forms::{
    valid_format, valid_uri, validate_answer, validate_content_read, validate_field_value,
    validate_turn_cancel,
};

pub const FRAME_MAX: usize = 1_048_576;
pub const JSON_DEPTH_MAX: usize = 32;
pub const JSON_MEMBERS_MAX: usize = 16_384;
pub const UINT_MAX: u64 = 9_007_199_254_740_991;
pub const ANSWER_MAX: usize = 262_144;
pub const CONTENT_INLINE_MAX: usize = 16_384;
pub const CONTENT_MAX: u64 = 16_777_216;
pub type ValidationResult<T> = Result<T, ProtocolFailure>;
pub type Validation = ValidationResult<()>;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    InvalidRequest,
    UnsupportedVersion,
    UnsupportedMethod,
    Incompatible,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolFailure {
    pub kind: FailureKind,
    pub request_id: Option<Uuid>,
    pub close: bool,
}
impl std::fmt::Display for ProtocolFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.kind)
    }
}
impl std::error::Error for ProtocolFailure {}
fn invalid() -> ProtocolFailure {
    ProtocolFailure {
        kind: FailureKind::InvalidRequest,
        request_id: None,
        close: true,
    }
}
fn ensure(ok: bool) -> Validation {
    if ok { Ok(()) } else { Err(invalid()) }
}
pub trait WireValidate {
    fn validate(&self) -> Validation;
}
impl WireValidate for bool {
    fn validate(&self) -> Validation {
        Ok(())
    }
}
impl WireValidate for String {
    fn validate(&self) -> Validation {
        ensure(self.len() <= FRAME_MAX)
    }
}
impl<T: WireValidate> WireValidate for Vec<T> {
    fn validate(&self) -> Validation {
        for x in self {
            x.validate()?;
        }
        Ok(())
    }
}
impl<T: WireValidate> WireValidate for Option<T> {
    fn validate(&self) -> Validation {
        if let Some(x) = self {
            x.validate()?;
        }
        Ok(())
    }
}
impl<T: WireValidate> WireValidate for Box<T> {
    fn validate(&self) -> Validation {
        self.as_ref().validate()
    }
}
fn optional_non_null<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}
pub type Text = String;

/// Count actual escaped JSON bytes without allocating a second catalogue/view buffer.
fn encoded_limit(value: &impl Serialize, limit: usize) -> Validation {
    encoded_size(value, limit).map(|_| ())
}
pub(crate) fn encoded_size(value: &impl Serialize, limit: usize) -> Result<usize, ProtocolFailure> {
    struct Counter {
        remaining: usize,
    }
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.remaining = self
                .remaining
                .checked_sub(bytes.len())
                .ok_or(std::io::ErrorKind::OutOfMemory)?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter { remaining: limit };
    serde_json::to_writer(&mut counter, value).map_err(|_| invalid())?;
    Ok(limit - counter.remaining)
}

macro_rules! string_type {
    ($name:ident, $check:expr) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(pub String);
        impl $name {
            pub fn new(s: impl Into<String>) -> ValidationResult<Self> {
                let result = Self(s.into());
                result.validate()?;
                Ok(result)
            }
        }
        impl WireValidate for $name {
            fn validate(&self) -> Validation {
                ensure(($check)(&self.0))
            }
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                Self::new(String::deserialize(d)?).map_err(serde::de::Error::custom)
            }
        }
    };
}
string_type!(Uuid, |s: &String| uuid::Uuid::parse_str(s).is_ok_and(|u| u
    .hyphenated()
    .to_string()
    == *s
    && u.get_variant() == uuid::Variant::RFC4122));
impl Uuid {
    pub fn random() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
}
string_type!(EndpointId, |s: &String| s
    .parse::<iroh::EndpointId>()
    .is_ok_and(|id| id.to_string() == *s));
string_type!(Id, |s: &String| !s.is_empty() && s.len() <= 512);
string_type!(AgentName, |s: &String| !s.is_empty()
    && s.len() <= 32
    && s.as_bytes()[0].is_ascii_lowercase()
    && s.bytes().all(|b| b.is_ascii_lowercase()
        || b.is_ascii_digit()
        || b == b'_'
        || b == b'-'));
string_type!(Label, |s: &String| !s.is_empty()
    && s.len() <= 512
    && s.chars().count() <= 128
    && !s.chars().any(char::is_control));
string_type!(SafeMessage, |s: &String| s.len() <= 512);
string_type!(InlineText, |s: &String| s.len() <= CONTENT_INLINE_MAX);
string_type!(PromptText, |s: &String| s.len() <= ANSWER_MAX);
string_type!(Key32, |s: &String| s.len() == 43
    && URL_SAFE_NO_PAD
        .decode(s)
        .is_ok_and(|b| b.len() == 32 && URL_SAFE_NO_PAD.encode(b) == *s));
string_type!(Base64Bytes, |s: &String| STANDARD
    .decode(s)
    .is_ok_and(|b| b.len() <= 65_536 && STANDARD.encode(b) == *s));
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Uint(pub u64);
impl Uint {
    pub fn new(n: u64) -> ValidationResult<Self> {
        ensure(n <= UINT_MAX)?;
        Ok(Self(n))
    }
}
impl WireValidate for Uint {
    fn validate(&self) -> Validation {
        ensure(self.0 <= UINT_MAX)
    }
}
impl<'de> Deserialize<'de> for Uint {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Self::new(u64::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}
macro_rules! constant_type {
    ($name:ident, $ty:ty, $value:expr) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize)]
        #[serde(transparent)]
        pub struct $name(pub $ty);
        impl WireValidate for $name {
            fn validate(&self) -> Validation {
                ensure(self.0 == $value)
            }
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let result = Self(<$ty>::deserialize(d)?);
                result.validate().map_err(serde::de::Error::custom)?;
                Ok(result)
            }
        }
    };
}
constant_type!(TrueValue, bool, true);
constant_type!(OneValue, u64, 1);
constant_type!(ProtocolVersion, u64, 2);
constant_type!(ZeroValue, u64, 0);
constant_type!(UnknownValue, String, "unknown");

/// Raw pass retains numeric spelling and catches recursive duplicates before serde creates maps.
struct Scanner<'a> {
    bytes: &'a [u8],
    at: usize,
    members: usize,
    request_id: Option<Uuid>,
    invalid_structure: bool,
    ambiguous_id: bool,
}
impl Scanner<'_> {
    fn whitespace(&mut self) {
        while self
            .bytes
            .get(self.at)
            .is_some_and(|b| matches!(b, b' ' | b'\n' | b'\r' | b'\t'))
        {
            self.at += 1;
        }
    }
    fn take(&mut self, b: u8) -> Validation {
        self.whitespace();
        ensure(self.bytes.get(self.at) == Some(&b))?;
        self.at += 1;
        Ok(())
    }
    fn scan_string(&mut self) -> Validation {
        self.take(b'"')?;
        loop {
            let b = *self.bytes.get(self.at).ok_or_else(invalid)?;
            self.at += 1;
            if b == b'"' {
                break;
            }
            if b < 32 {
                self.invalid_structure = true;
            }
            if b == b'\\' {
                ensure(self.at < self.bytes.len())?;
                self.at += 1;
            }
        }
        Ok(())
    }
    fn string(&mut self) -> ValidationResult<String> {
        self.whitespace();
        let start = self.at;
        self.scan_string()?;
        serde_json::from_slice(&self.bytes[start..self.at]).map_err(|_| invalid())
    }
    fn member(&mut self) -> Validation {
        self.members += 1;
        ensure(self.members <= JSON_MEMBERS_MAX)
    }
    fn value(&mut self, depth: usize) -> Validation {
        self.whitespace();
        match self.bytes.get(self.at).copied().ok_or_else(invalid)? {
            b'{' => {
                ensure(depth <= JSON_DEPTH_MAX)?;
                self.at += 1;
                self.whitespace();
                let mut keys = HashSet::new();
                if self.bytes.get(self.at) == Some(&b'}') {
                    self.at += 1;
                    return Ok(());
                }
                loop {
                    self.member()?;
                    let key = self.string()?;
                    let is_id = depth == 1 && key == "id";
                    if keys.contains(&key) {
                        self.invalid_structure = true;
                        if is_id {
                            self.ambiguous_id = true;
                            self.request_id = None;
                        }
                    }
                    keys.insert(key);
                    self.take(b':')?;
                    self.whitespace();
                    let start = self.at;
                    self.value(depth + 1)?;
                    if is_id && !self.ambiguous_id {
                        self.request_id =
                            serde_json::from_slice::<Uuid>(&self.bytes[start..self.at]).ok();
                    }
                    self.whitespace();
                    if self.bytes.get(self.at) == Some(&b'}') {
                        self.at += 1;
                        break;
                    }
                    self.take(b',')?;
                }
            }
            b'[' => {
                ensure(depth <= JSON_DEPTH_MAX)?;
                self.at += 1;
                self.whitespace();
                if self.bytes.get(self.at) == Some(&b']') {
                    self.at += 1;
                    return Ok(());
                }
                loop {
                    self.member()?;
                    self.value(depth + 1)?;
                    self.whitespace();
                    if self.bytes.get(self.at) == Some(&b']') {
                        self.at += 1;
                        break;
                    }
                    self.take(b',')?;
                }
            }
            b'"' => {
                self.scan_string()?;
            }
            b't' => {
                ensure(self.bytes.get(self.at..self.at + 4) == Some(b"true"))?;
                self.at += 4;
            }
            b'f' => {
                ensure(self.bytes.get(self.at..self.at + 5) == Some(b"false"))?;
                self.at += 5;
            }
            b'n' => {
                ensure(self.bytes.get(self.at..self.at + 4) == Some(b"null"))?;
                self.at += 4;
            }
            b'0'..=b'9' | b'-' => {
                let start = self.at;
                while self
                    .bytes
                    .get(self.at)
                    .is_some_and(|b| matches!(b, b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E'))
                {
                    self.at += 1;
                }
                let token =
                    std::str::from_utf8(&self.bytes[start..self.at]).map_err(|_| invalid())?;
                if !(token == "0"
                    || (!token.starts_with('0') && token.bytes().all(|b| b.is_ascii_digit())))
                    || !token.parse::<u64>().is_ok_and(|n| n <= UINT_MAX)
                {
                    self.invalid_structure = true;
                }
            }
            _ => return Err(invalid()),
        }
        Ok(())
    }
}
// Build literal JSON maps without serde_json::Value's feature-private wrapper handling.
struct LiteralJson(Value);
impl<'de> Deserialize<'de> for LiteralJson {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct LiteralVisitor;
        impl<'de> serde::de::Visitor<'de> for LiteralVisitor {
            type Value = Value;
            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("literal JSON with Goatr integers")
            }
            fn visit_bool<E: serde::de::Error>(self, value: bool) -> Result<Value, E> {
                Ok(Value::Bool(value))
            }
            fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Value, E> {
                if value > UINT_MAX {
                    return Err(E::custom("Uint out of range"));
                }
                Ok(Value::Number(value.into()))
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Value, E> {
                Ok(Value::Null)
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Value, E> {
                Ok(Value::String(value.to_owned()))
            }
            fn visit_string<E: serde::de::Error>(self, value: String) -> Result<Value, E> {
                Ok(Value::String(value))
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut sequence: A,
            ) -> Result<Value, A::Error> {
                let mut array = Vec::new();
                while let Some(value) = sequence.next_element::<LiteralJson>()? {
                    array.push(value.0);
                }
                Ok(Value::Array(array))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Value, A::Error> {
                let mut object = serde_json::Map::new();
                while let Some((key, value)) = map.next_entry::<String, LiteralJson>()? {
                    object.insert(key, value.0);
                }
                Ok(Value::Object(object))
            }
        }
        deserializer
            .deserialize_any(LiteralVisitor)
            .map(LiteralJson)
    }
}
pub fn parse_raw_json(bytes: &[u8]) -> ValidationResult<Value> {
    ensure(!bytes.is_empty() && bytes.len() <= FRAME_MAX)?;
    let valid_utf8 = std::str::from_utf8(bytes).is_ok();
    let mut scanner = Scanner {
        bytes,
        at: 0,
        members: 0,
        request_id: None,
        invalid_structure: false,
        ambiguous_id: false,
    };
    let checked = scanner.value(1).and_then(|()| {
        scanner.whitespace();
        ensure(valid_utf8 && scanner.at == bytes.len() && !scanner.invalid_structure)
    });
    if let Err(mut e) = checked {
        e.request_id = scanner.request_id;
        return Err(e);
    }
    serde_json::from_slice::<LiteralJson>(bytes)
        .map(|value| value.0)
        .map_err(|_| ProtocolFailure {
            request_id: scanner.request_id,
            ..invalid()
        })
}
fn typed<T: serde::de::DeserializeOwned + Serialize + WireValidate>(
    value: &Value,
    strict: bool,
) -> ValidationResult<T> {
    let result = T::deserialize(value).map_err(|_| invalid())?;
    result.validate()?;
    known_fields(
        value,
        &serde_json::to_value(&result).map_err(|_| invalid())?,
        strict,
    )?;
    Ok(result)
}
fn known_fields(input: &Value, typed: &Value, strict: bool) -> Validation {
    match (input, typed) {
        (Value::Object(input), Value::Object(typed)) => {
            for (key, value) in input {
                if let Some(known) = typed.get(key) {
                    known_fields(value, known, strict)?;
                } else {
                    ensure(!strict)?;
                }
            }
        }
        (Value::Array(input), Value::Array(typed)) => {
            ensure(input.len() == typed.len())?;
            for (a, b) in input.iter().zip(typed) {
                known_fields(a, b, strict)?;
            }
        }
        (Value::String(_), Value::String(_))
        | (Value::Bool(_), Value::Bool(_))
        | (Value::Number(_), Value::Number(_)) => {}
        _ => return Err(invalid()),
    }
    Ok(())
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub id: Uuid,
    pub body: RequestBody,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub id: Uuid,
    pub outcome: Result<ResultBody, WireError>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub subscription: Uuid,
    pub seq: Uint,
    pub body: EventBody,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Envelope {
    Request(Request),
    Response(Response),
    Event(Event),
}
fn required<'a>(
    object: &'a serde_json::Map<String, Value>,
    key: &str,
) -> ValidationResult<&'a Value> {
    object.get(key).ok_or_else(invalid)
}
fn only(object: &serde_json::Map<String, Value>, names: &[&str]) -> Validation {
    ensure(object.keys().all(|key| names.contains(&key.as_str())))
}
pub fn decode_payload(bytes: &[u8], response_method: Option<Method>) -> ValidationResult<Envelope> {
    let value = parse_raw_json(bytes)?;
    let object = value.as_object().ok_or_else(invalid)?;
    let id = object
        .get("id")
        .and_then(|v| serde_json::from_value::<Uuid>(v.clone()).ok());
    let bad_envelope = || ProtocolFailure {
        kind: FailureKind::InvalidRequest,
        request_id: id.clone(),
        close: true,
    };
    let version = required(object, "v")
        .map_err(|_| bad_envelope())?
        .as_u64()
        .ok_or_else(bad_envelope)?;
    if version != 2 {
        return Err(ProtocolFailure {
            kind: FailureKind::UnsupportedVersion,
            request_id: id,
            close: true,
        });
    }
    let kind = required(object, "type")
        .map_err(|_| bad_envelope())?
        .as_str()
        .ok_or_else(bad_envelope)?;
    let result = (|| match kind {
        "request" => {
            only(object, &["v", "type", "id", "method", "params"])?;
            let id: Uuid = typed(required(object, "id")?, true)?;
            ensure(required(object, "params")?.is_object())?;
            let method_value = required(object, "method")?;
            ensure(method_value.is_string())?;
            let method: Method =
                Method::deserialize(method_value).map_err(|_| ProtocolFailure {
                    kind: FailureKind::UnsupportedMethod,
                    request_id: Some(id.clone()),
                    close: false,
                })?;
            let body = decode_request_body(method, required(object, "params")?)?;
            Ok(Envelope::Request(Request { id, body }))
        }
        "response" => {
            let id = typed(required(object, "id")?, false)?;
            ensure(object.contains_key("result") != object.contains_key("error"))?;
            let outcome = if let Some(error) = object.get("error") {
                Err(typed(error, false)?)
            } else {
                Ok(decode_result_body(
                    response_method.ok_or_else(invalid)?,
                    required(object, "result")?,
                )?)
            };
            Ok(Envelope::Response(Response { id, outcome }))
        }
        "event" => {
            only(
                object,
                &["v", "type", "subscription", "seq", "kind", "data"],
            )?;
            let subscription = typed(required(object, "subscription")?, true)?;
            let seq = typed(required(object, "seq")?, true)?;
            let body = typed(
                &serde_json::json!({"kind":required(object,"kind")?,"data":required(object,"data")?}),
                true,
            )?;
            Ok(Envelope::Event(Event {
                subscription,
                seq,
                body,
            }))
        }
        _ => Err(invalid()),
    })();
    result.map_err(|mut e: ProtocolFailure| {
        if kind == "response" || kind == "event" {
            e.kind = FailureKind::Incompatible;
            e.close = true;
        }
        e.request_id = id;
        e
    })
}
pub fn frame_length(header: [u8; 4]) -> ValidationResult<usize> {
    let n = u32::from_be_bytes(header) as usize;
    ensure(n > 0 && n <= FRAME_MAX)?;
    Ok(n)
}
pub fn read_frame(reader: &mut impl Read) -> ValidationResult<Vec<u8>> {
    let mut header = [0; 4];
    reader.read_exact(&mut header).map_err(|_| invalid())?;
    let mut bytes = vec![0; frame_length(header)?];
    reader.read_exact(&mut bytes).map_err(|_| invalid())?;
    Ok(bytes)
}
pub fn write_frame(writer: &mut impl Write, bytes: &[u8]) -> Validation {
    ensure(!bytes.is_empty() && bytes.len() <= FRAME_MAX)?;
    writer
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .and_then(|()| writer.write_all(bytes))
        .map_err(|_| invalid())
}

/// Canonical scalar ordering equals lexicographic UTF-8 order for valid Unicode.
pub fn canonical_json(value: &Value) -> ValidationResult<Vec<u8>> {
    fn string(s: &str, out: &mut String) -> Validation {
        use std::fmt::Write;
        out.push('"');
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\u{0}'..='\u{1f}' => write!(out, "\\u{:04x}", c as u32).map_err(|_| invalid())?,
                _ => out.push(c),
            }
        }
        out.push('"');
        Ok(())
    }
    fn emit(value: &Value, out: &mut String) -> Validation {
        match value {
            Value::Null => return Err(invalid()),
            Value::Bool(x) => out.push_str(if *x { "true" } else { "false" }),
            Value::Number(x) => {
                use std::fmt::Write;
                let n = x.as_u64().filter(|n| *n <= UINT_MAX).ok_or_else(invalid)?;
                write!(out, "{n}").map_err(|_| invalid())?;
            }
            Value::String(s) => string(s, out)?,
            Value::Array(xs) => {
                out.push('[');
                for (i, x) in xs.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    emit(x, out)?;
                }
                out.push(']');
            }
            Value::Object(xs) => {
                out.push('{');
                let mut keys: Vec<_> = xs.keys().collect();
                keys.sort_unstable();
                for (i, key) in keys.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    string(key, out)?;
                    out.push(':');
                    emit(&xs[*key], out)?;
                }
                out.push('}');
            }
        }
        Ok(())
    }
    let mut out = String::with_capacity(canonical_json_size(value)?);
    emit(value, &mut out)?;
    Ok(out.into_bytes())
}

/// Canonical expansion is not the transmitted frame length; count without an answer buffer.
pub fn canonical_json_size(value: &Value) -> ValidationResult<usize> {
    fn string(s: &str) -> usize {
        2 + s
            .chars()
            .map(|c| match c {
                '"' | '\\' => 2,
                '\u{0}'..='\u{1f}' => 6,
                _ => c.len_utf8(),
            })
            .sum::<usize>()
    }
    Ok(match value {
        Value::Null => return Err(invalid()),
        Value::Bool(x) => {
            if *x {
                4
            } else {
                5
            }
        }
        Value::String(s) => string(s),
        Value::Number(n) => {
            let n = n.as_u64().filter(|n| *n <= UINT_MAX).ok_or_else(invalid)?;
            if n == 0 { 1 } else { n.ilog10() as usize + 1 }
        }
        Value::Array(xs) => {
            let mut n = 2 + xs.len().saturating_sub(1);
            for x in xs {
                n += canonical_json_size(x)?;
            }
            n
        }
        Value::Object(xs) => {
            let mut n = 2 + xs.len().saturating_sub(1);
            for (k, x) in xs {
                n += string(k) + 1 + canonical_json_size(x)?;
            }
            n
        }
    })
}
fn request_body_value(body: &RequestBody) -> ValidationResult<Value> {
    let mut value = serde_json::to_value(body).map_err(|_| invalid())?;
    if let RequestBody::TerminalGridSubscribe(params) = body {
        let mut fields = serde_json::Map::new();
        fields.insert(
            "target".into(),
            serde_json::to_value(&params.target).map_err(|_| invalid())?,
        );
        if params.ansi_observer {
            fields.insert("ansiObserver".into(), Value::Bool(true));
        }
        value["params"] = Value::Object(fields);
    }
    if let RequestBody::ShellGridSubscribe(params) = body {
        let mut fields = serde_json::Map::new();
        fields.insert(
            "target".into(),
            serde_json::to_value(&params.target).map_err(|_| invalid())?,
        );
        if params.history {
            fields.insert("history".into(), Value::Bool(true));
        }
        value["params"] = Value::Object(fields);
    }
    Ok(value)
}
pub fn canonical_request(request: &Request) -> ValidationResult<Vec<u8>> {
    request.id.validate()?;
    request.body.validate()?;
    let mut value = request_body_value(&request.body)?;
    if matches!(
        request.body.method(),
        Method::SessionHistory | Method::NotificationsSync | Method::SessionQueueList
    ) {
        value["params"]
            .as_object_mut()
            .ok_or_else(invalid)?
            .entry("limit")
            .or_insert(Value::from(100));
    }
    canonical_json(&value)
}
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}
pub fn hmac_sha256(key: &[u8], bytes: &[u8]) -> [u8; 32] {
    let mut mac =
        <Hmac<Sha256> as KeyInit>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(bytes);
    mac.finalize().into_bytes().into()
}
pub fn verify_hmac_sha256(key: &[u8], bytes: &[u8], expected: &[u8]) -> bool {
    let mut mac =
        <Hmac<Sha256> as KeyInit>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(bytes);
    mac.verify_slice(expected).is_ok()
}

pub fn decode_presentation(bytes: &[u8]) -> ValidationResult<Presentation> {
    typed(&parse_raw_json(bytes)?, true)
}
pub fn decode_answer(bytes: &[u8]) -> ValidationResult<Answer> {
    typed(&parse_raw_json(bytes)?, true)
}
pub fn decode_form_field(bytes: &[u8]) -> ValidationResult<FormField> {
    typed(&parse_raw_json(bytes)?, true)
}
fn json_bounds(value: &Value, depth: usize, members: &mut usize) -> bool {
    let count = match value {
        Value::Object(values) => values.len(),
        Value::Array(values) => values.len(),
        _ => return true,
    };
    if depth > JSON_DEPTH_MAX || count > JSON_MEMBERS_MAX - *members {
        return false;
    }
    *members += count;
    match value {
        Value::Object(values) => values
            .values()
            .all(|child| json_bounds(child, depth + 1, members)),
        Value::Array(values) => values
            .iter()
            .all(|child| json_bounds(child, depth + 1, members)),
        _ => unreachable!(),
    }
}

/// Complete validated graphs or an explicit finite display failure. Never hide malformed
/// relationships/associations behind the budget fallback or broaden phone framing.
pub(crate) fn host_inventory_payload(
    request: Uuid,
    result: HostSnapshotResult,
) -> ValidationResult<Vec<u8>> {
    request.validate()?;
    result.validate()?;
    let mut value = serde_json::json!({"v":2,"type":"response","id":request,"result":result});
    loop {
        if json_bounds(&value, 1, &mut 0) {
            let bytes = serde_json::to_vec(&value).map_err(|_| invalid())?;
            if bytes.len() <= FRAME_MAX {
                return Ok(bytes);
            }
        }
        let graphs = value["result"]["hierarchy"]
            .as_array_mut()
            .ok_or_else(invalid)?;
        // ponytail: remove whole records in reverse native-server publication order only on
        // observed overflow; at most64 graphs. No paging/ranking or second inventory authority.
        let graph = graphs
            .iter_mut()
            .rev()
            .find(|h| {
                ["workspaces", "tabs", "panes"]
                    .iter()
                    .any(|k| h[*k].as_array().is_some_and(|a| !a.is_empty()))
            })
            .ok_or_else(invalid)?;
        for key in ["workspaces", "tabs", "panes"] {
            graph[key] = Value::Array(Vec::new());
        }
        if graph["freshness"].as_str() == Some("current") {
            graph["reason"] = Value::from("too_large");
        }
        graph["freshness"] = Value::from("unavailable");
        let server = graph["serverId"].clone();
        // An omitted native graph cannot carry actionable shell authority.
        if let Some(shells) = value["result"]["shells"].as_array_mut() {
            for shell in shells
                .iter_mut()
                .filter(|shell| shell["serverId"] == server)
            {
                shell["connectivity"] = Value::from("offline");
                shell["capabilities"] =
                    serde_json::json!({"read":false,"create":false,"input":false,"close":false});
            }
        }
    }
}

pub fn encode_payload(envelope: &Envelope) -> ValidationResult<Vec<u8>> {
    fn value<T: Serialize + WireValidate>(x: &T) -> ValidationResult<Value> {
        x.validate()?;
        serde_json::to_value(x).map_err(|_| invalid())
    }
    let mut object = serde_json::Map::new();
    object.insert("v".into(), Value::from(2));
    match envelope {
        Envelope::Request(x) => {
            object.insert("type".into(), Value::from("request"));
            object.insert("id".into(), value(&x.id)?);
            x.body.validate()?;
            let Value::Object(body) = request_body_value(&x.body)? else {
                return Err(invalid());
            };
            object.extend(body);
        }
        Envelope::Response(x) => {
            object.insert("type".into(), Value::from("response"));
            object.insert("id".into(), value(&x.id)?);
            match &x.outcome {
                Err(error) => {
                    object.insert("error".into(), value(error)?);
                }
                Ok(result) => {
                    let result = match result {
                        ResultBody::SessionControlsRead(x) => value(x)?,
                        ResultBody::SessionControlsApply(x) => value(x)?,
                        ResultBody::SessionQueueList(x) => value(x)?,
                        ResultBody::SessionQueueUpdate(x) => value(x)?,
                        ResultBody::SessionQueueDelete(x) => value(x)?,
                        ResultBody::SessionQueueReorder(x) => value(x)?,
                        ResultBody::SessionQueueStart(x) => value(x)?,
                        ResultBody::PairConfirm(x) => value(x)?,
                        ResultBody::Hello(x) => value(x)?,
                        ResultBody::HostSnapshot(x) => value(x)?,
                        ResultBody::SessionOpen(x) => value(x)?,
                        ResultBody::SessionHistory(x) => value(x)?,
                        ResultBody::SessionUnsubscribe(x) => value(x)?,
                        ResultBody::SessionCreate(x) => value(x)?,
                        ResultBody::TerminalCreate(x) => value(x)?,
                        ResultBody::AttachmentsBegin(x) => value(x)?,
                        ResultBody::AttachmentsChunk(x) => value(x)?,
                        ResultBody::AttachmentsComplete(x) => value(x)?,
                        ResultBody::AttachmentsDiscard(x) => value(x)?,
                        ResultBody::CreationDirectories(x) => value(x)?,
                        ResultBody::AttachmentsStatus(x) => value(x)?,
                        ResultBody::AgentRename(x) => value(x)?,
                        ResultBody::WorkspaceRename(x) => value(x)?,
                        ResultBody::PromptSubmit(x) => value(x)?,
                        ResultBody::TurnCancel(x) => value(x)?,
                        ResultBody::RequestRespond(x) => value(x)?,
                        ResultBody::ActionStatus(x) => value(x)?,
                        ResultBody::ContentRead(x) => value(x)?,
                        ResultBody::TerminalGridSubscribe(x) => value(x)?,
                        ResultBody::ShellGridSubscribe(x) => value(x)?,
                        ResultBody::ShellInput(x) => value(x)?,
                        ResultBody::ShellResize(x) => value(x)?,
                        ResultBody::ShellScroll(x) => value(x)?,
                        ResultBody::ArtifactsList(x) => value(x)?,
                        ResultBody::ArtifactsOpen(x) => value(x)?,
                        ResultBody::ArtifactsRead(x) => value(x)?,
                        ResultBody::ArtifactsClose(x) => value(x)?,
                        ResultBody::WorktreesList(x) => value(x)?,
                        ResultBody::ChangesRead(x) => value(x)?,
                        ResultBody::SkillsList(x) => value(x)?,
                        ResultBody::SkillsPreviewOpen(x) => value(x)?,
                        ResultBody::SkillsPreviewRead(x) => value(x)?,
                        ResultBody::SkillsPreviewClose(x) => value(x)?,

                        ResultBody::NotificationsSync(x) => value(x)?,
                        ResultBody::NotificationsRead(x) => value(x)?,
                        ResultBody::PushRegister(x) => value(x)?,
                        ResultBody::PushProbeAck(x) => value(x)?,
                        ResultBody::PushUnregister(x) => value(x)?,
                        ResultBody::DeviceUnpair(x) => value(x)?,
                    };
                    object.insert("result".into(), result);
                }
            }
        }
        Envelope::Event(x) => {
            object.insert("type".into(), Value::from("event"));
            object.insert("subscription".into(), value(&x.subscription)?);
            object.insert("seq".into(), value(&x.seq)?);
            let Value::Object(body) = value(&x.body)? else {
                return Err(invalid());
            };
            object.extend(body);
        }
    }
    let value = Value::Object(object);
    ensure(json_bounds(&value, 1, &mut 0))?;
    let bytes = serde_json::to_vec(&value).map_err(|_| invalid())?;
    ensure(bytes.len() <= FRAME_MAX)?;
    Ok(bytes)
}

#[cfg(test)]
mod inventory_tests {
    use super::*;

    fn oversized_inventory() -> HostSnapshotResult {
        let corpus: Value =
            serde_json::from_str(include_str!("../../protocol/fixtures/v2.json")).unwrap();
        let case = corpus["frames"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| {
                v["name"]
                    == "native hierarchy shells unsupported empty order and exact associations"
            })
            .unwrap();
        let wire: Value = serde_json::from_str(case["wire"].as_str().unwrap()).unwrap();
        let mut result: HostSnapshotResult =
            serde_json::from_value(wire["result"].clone()).unwrap();
        for _ in 0..2 {
            let server = Uuid::random();
            result.servers.push(Server {
                server_id: server.clone(),
                name: Label::new("Native server").unwrap(),
                connectivity: ServerConnectivity::Online,
                can_create: CanCreate {
                    omp: false,
                    codex: false,
                },
                reason: None,
            });
            let label = Label::new("😀".repeat(128)).unwrap();
            let panes = (0..240)
                .map(|n| NativePane {
                    shell_target: None,
                    pane_id: Id(format!("pane-{n}")),
                    terminal_id: Id(format!("terminal-{n}")),
                    workspace_id: Id("workspace".into()),
                    tab_id: Id("tab".into()),
                    label: Some(label.clone()),
                    title: Some(label.clone()),
                    terminal_title: Some(label.clone()),
                    display_agent: Some(label.clone()),
                    agent: Some(Label::new("claude").unwrap()),
                    agent_name: None,
                    focused: false,
                    agent_status: NativeAgentStatus::Unknown,
                    restore_reason: None,
                    display_unavailable: None,
                    target: None,
                    unbound_row_id: None,
                })
                .collect();
            result.hierarchy.push(NativeHierarchy {
                server_id: server,
                server_incarnation: Uuid::random(),
                freshness: NativeFreshness::Current,
                reason: None,
                workspaces: vec![NativeWorkspace {
                    workspace_id: Id("workspace".into()),
                    number: Uint(1),
                    label: Some(Label::new("Workspace").unwrap()),
                    focused: false,
                    active_tab_id: Some(Id("tab".into())),
                    agent_status: NativeAgentStatus::Unknown,
                    display_unavailable: None,
                }],
                tabs: vec![NativeTab {
                    tab_id: Id("tab".into()),
                    workspace_id: Id("workspace".into()),
                    number: Uint(1),
                    label: Some(Label::new("Tab").unwrap()),
                    focused: false,
                    agent_status: NativeAgentStatus::Unknown,
                    display_unavailable: None,
                }],
                panes,
            });
        }
        result
    }

    #[test]
    fn hierarchy_budget_keeps_complete_other_servers_and_never_masks_invalid_associations() {
        let result = oversized_inventory();
        let original = result.hierarchy[0].clone();
        result.validate().unwrap();
        assert!(
            encode_payload(&Envelope::Response(Response {
                id: Uuid::random(),
                outcome: Ok(ResultBody::HostSnapshot(result.clone()))
            }))
            .is_err()
        );
        let mut invalid = result.clone();
        invalid.sessions[0].generation = Uuid::random();
        assert!(
            host_inventory_payload(Uuid::random(), invalid).is_err(),
            "an invalid binding is not a display overflow"
        );
        let bytes = host_inventory_payload(Uuid::random(), result).unwrap();
        let Envelope::Response(response) =
            decode_payload(&bytes, Some(Method::HostSnapshot)).unwrap()
        else {
            panic!("response required")
        };
        let Ok(ResultBody::HostSnapshot(snapshot)) = response.outcome else {
            panic!("inventory required")
        };
        assert_eq!(snapshot.hierarchy[0], original);
        assert_eq!(snapshot.hierarchy[1].freshness, NativeFreshness::Current);
        assert_eq!(snapshot.hierarchy[1].panes.len(), 240);
        assert_eq!(
            snapshot.hierarchy[2].freshness,
            NativeFreshness::Unavailable
        );
        assert_eq!(
            snapshot.hierarchy[2].reason,
            Some(NativeUnavailableReason::TooLarge)
        );
        assert!(
            snapshot.hierarchy[2].workspaces.is_empty()
                && snapshot.hierarchy[2].tabs.is_empty()
                && snapshot.hierarchy[2].panes.is_empty()
        );
    }

    #[test]
    fn hierarchy_budget_preserves_unavailable_source_when_collapsing_stale_graph() {
        let mut result = oversized_inventory();
        let mut graph = result.hierarchy.remove(1);
        let extra = result.hierarchy.pop().unwrap();
        graph
            .panes
            .extend(extra.panes.into_iter().enumerate().map(|(n, mut pane)| {
                pane.pane_id = Id(format!("extra-pane-{n}"));
                pane.terminal_id = Id(format!("extra-terminal-{n}"));
                pane
            }));
        graph.server_id = result.servers[0].server_id.clone();
        graph.server_incarnation = result.hierarchy[0].server_incarnation.clone();
        graph.freshness = NativeFreshness::Stale;
        graph.reason = Some(NativeUnavailableReason::SourceUnavailable);
        result.servers.truncate(1);
        result.unbound.clear();
        result.hierarchy = vec![graph];
        result.validate().unwrap();
        assert_eq!(result.hierarchy[0].panes.len(), 480);
        assert!(
            result.hierarchy[0]
                .panes
                .iter()
                .all(|p| { p.target.is_none() && p.unbound_row_id.is_none() })
        );
        assert!(
            encode_payload(&Envelope::Response(Response {
                id: Uuid::random(),
                outcome: Ok(ResultBody::HostSnapshot(result.clone()))
            }))
            .is_err()
        );

        let summaries = result.sessions.clone();
        let bytes = host_inventory_payload(Uuid::random(), result).unwrap();
        assert!(bytes.len() <= FRAME_MAX);
        let Envelope::Response(response) =
            decode_payload(&bytes, Some(Method::HostSnapshot)).unwrap()
        else {
            panic!("bounded response required")
        };
        let Ok(ResultBody::HostSnapshot(snapshot)) = response.outcome else {
            panic!("bounded inventory required")
        };
        assert_eq!(snapshot.servers[0].connectivity, ServerConnectivity::Online);
        assert_eq!(snapshot.sessions, summaries);
        let collapsed = &snapshot.hierarchy[0];
        assert_eq!(collapsed.freshness, NativeFreshness::Unavailable);
        assert_eq!(
            collapsed.reason,
            Some(NativeUnavailableReason::SourceUnavailable),
            "display overflow must not turn an invalidated source into current authority"
        );
        assert!(
            collapsed.workspaces.is_empty()
                && collapsed.tabs.is_empty()
                && collapsed.panes.is_empty()
        );

        let corpus: Value =
            serde_json::from_str(include_str!("../../protocol/fixtures/v2.json")).unwrap();
        let case = corpus["frames"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["name"] == "native hierarchy stale overflow remains source unavailable")
            .unwrap();
        let Envelope::Response(response) = decode_payload(
            case["wire"].as_str().unwrap().as_bytes(),
            Some(Method::HostSnapshot),
        )
        .unwrap() else {
            panic!("shared Kotlin consumer input required")
        };
        let Ok(ResultBody::HostSnapshot(expected)) = response.outcome else {
            panic!("shared inventory required")
        };
        assert_eq!(collapsed.freshness, expected.hierarchy[0].freshness);
        assert_eq!(collapsed.reason, expected.hierarchy[0].reason);
        assert_eq!(
            snapshot.servers[0].connectivity,
            expected.servers[0].connectivity
        );
        assert_eq!(
            snapshot.sessions[0].capabilities,
            expected.sessions[0].capabilities
        );
    }
}
