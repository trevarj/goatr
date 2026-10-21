// SPDX-License-Identifier: GPL-3.0-or-later
//! Native-only JSON: literal maps and exact numeric tokens, never the phone Uint grammar.
use crate::protocol::{Decimal, NativeRpcId};
use serde_json::{Map, Number, Value};
use std::io;

#[derive(Clone, Copy)]
pub struct Limits {
    pub bytes: usize,
    pub depth: usize,
    pub members: usize,
}
pub const NATIVE: Limits = Limits {
    bytes: 8 * 1024 * 1024,
    depth: 128,
    members: 131_072,
};
pub const REGISTRY: Limits = Limits {
    bytes: 65_536,
    depth: 32,
    members: 16_384,
};
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid or oversized native JSON",
    )
}

pub fn parse(bytes: &[u8], limits: Limits) -> io::Result<Value> {
    if bytes.is_empty() || bytes.len() > limits.bytes || std::str::from_utf8(bytes).is_err() {
        return Err(invalid());
    }
    let mut parser = Parser {
        bytes,
        at: 0,
        members: 0,
        limits,
    };
    let value = parser.value(1)?;
    parser.space();
    if parser.at != bytes.len() {
        return Err(invalid());
    }
    Ok(value)
}
pub fn rpc_id(value: &Value) -> io::Result<NativeRpcId> {
    let raw = match value {
        Value::String(s) => serde_json::to_string(s).map_err(|_| invalid())?,
        Value::Number(n) => n.to_string(),
        _ => return Err(invalid()),
    };
    NativeRpcId::from_raw(&raw).map_err(|_| invalid())
}
pub fn decimal(value: &Value) -> io::Result<Decimal> {
    match value {
        Value::Number(n) => Decimal::from_native_token(&n.to_string()).map_err(|_| invalid()),
        _ => Err(invalid()),
    }
}
struct Parser<'a> {
    bytes: &'a [u8],
    at: usize,
    members: usize,
    limits: Limits,
}
impl Parser<'_> {
    fn space(&mut self) {
        while self
            .bytes
            .get(self.at)
            .is_some_and(|b| matches!(b, b' ' | b'\n' | b'\r' | b'\t'))
        {
            self.at += 1;
        }
    }
    fn take(&mut self, b: u8) -> io::Result<()> {
        self.space();
        if self.bytes.get(self.at) != Some(&b) {
            return Err(invalid());
        }
        self.at += 1;
        Ok(())
    }
    fn member(&mut self) -> io::Result<()> {
        self.members += 1;
        if self.members > self.limits.members {
            Err(invalid())
        } else {
            Ok(())
        }
    }
    fn string(&mut self) -> io::Result<String> {
        self.space();
        let start = self.at;
        self.take(b'"')?;
        loop {
            let b = *self.bytes.get(self.at).ok_or_else(invalid)?;
            self.at += 1;
            if b == b'"' {
                break;
            }
            if b < 32 {
                return Err(invalid());
            }
            if b == b'\\' {
                if self.at >= self.bytes.len() {
                    return Err(invalid());
                }
                self.at += 1;
            }
        }
        serde_json::from_slice(&self.bytes[start..self.at]).map_err(|_| invalid())
    }
    fn value(&mut self, depth: usize) -> io::Result<Value> {
        if depth > self.limits.depth {
            return Err(invalid());
        }
        self.space();
        match self.bytes.get(self.at).copied().ok_or_else(invalid)? {
            b'{' => {
                self.at += 1;
                let mut map = Map::new();
                self.space();
                if self.bytes.get(self.at) == Some(&b'}') {
                    self.at += 1;
                    return Ok(Value::Object(map));
                }
                loop {
                    self.member()?;
                    let key = self.string()?;
                    if map.contains_key(&key) {
                        return Err(invalid());
                    }
                    self.take(b':')?;
                    let value = self.value(depth + 1)?;
                    map.insert(key, value);
                    self.space();
                    if self.bytes.get(self.at) == Some(&b'}') {
                        self.at += 1;
                        break;
                    }
                    self.take(b',')?;
                }
                Ok(Value::Object(map))
            }
            b'[' => {
                self.at += 1;
                let mut values = Vec::new();
                self.space();
                if self.bytes.get(self.at) == Some(&b']') {
                    self.at += 1;
                    return Ok(Value::Array(values));
                }
                loop {
                    self.member()?;
                    values.push(self.value(depth + 1)?);
                    self.space();
                    if self.bytes.get(self.at) == Some(&b']') {
                        self.at += 1;
                        break;
                    }
                    self.take(b',')?;
                }
                Ok(Value::Array(values))
            }
            b'"' => self.string().map(Value::String),
            b't' => {
                self.literal(b"true")?;
                Ok(Value::Bool(true))
            }
            b'f' => {
                self.literal(b"false")?;
                Ok(Value::Bool(false))
            }
            b'n' => {
                self.literal(b"null")?;
                Ok(Value::Null)
            }
            b'-' | b'0'..=b'9' => {
                let start = self.at;
                if self.bytes[self.at] == b'-' {
                    self.at += 1;
                }
                match self.bytes.get(self.at) {
                    Some(b'0') => self.at += 1,
                    Some(b'1'..=b'9') => self.digits(),
                    _ => return Err(invalid()),
                }
                if self.bytes.get(self.at) == Some(&b'.') {
                    self.at += 1;
                    self.required_digits()?;
                }
                if self
                    .bytes
                    .get(self.at)
                    .is_some_and(|b| matches!(b, b'e' | b'E'))
                {
                    self.at += 1;
                    if self
                        .bytes
                        .get(self.at)
                        .is_some_and(|b| matches!(b, b'+' | b'-'))
                    {
                        self.at += 1;
                    }
                    self.required_digits()?;
                }
                let token =
                    std::str::from_utf8(&self.bytes[start..self.at]).map_err(|_| invalid())?;
                // Pinned serde_json's FromStr normalizes signed integer -0. The lexer above
                // has already validated the complete JSON number grammar; store its token.
                Ok(Value::Number(Number::from_string_unchecked(
                    token.to_owned(),
                )))
            }
            _ => Err(invalid()),
        }
    }
    fn literal(&mut self, s: &[u8]) -> io::Result<()> {
        if self.bytes.get(self.at..self.at + s.len()) != Some(s) {
            return Err(invalid());
        }
        self.at += s.len();
        Ok(())
    }
    fn digits(&mut self) {
        while self.bytes.get(self.at).is_some_and(u8::is_ascii_digit) {
            self.at += 1;
        }
    }
    fn required_digits(&mut self) -> io::Result<()> {
        let at = self.at;
        self.digits();
        if at == self.at {
            Err(invalid())
        } else {
            Ok(())
        }
    }
}
