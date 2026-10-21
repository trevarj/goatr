// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;

/// Bounded, normalized decimal text; precision is limited only by the wire's 128 bytes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct Decimal(pub String);
impl Decimal {
    pub fn new(text: &str) -> ValidationResult<Self> {
        ensure(!text.is_empty() && text.len() <= 128)?;
        let unsigned = text.strip_prefix('-').unwrap_or(text);
        let mut parts = unsigned.split('.');
        let whole = parts.next().unwrap();
        let fraction = parts.next();
        ensure(parts.next().is_none() && valid_whole(whole))?;
        if let Some(f) = fraction {
            ensure(!f.is_empty() && f.bytes().all(|b| b.is_ascii_digit()))?;
        }
        let fraction = fraction.unwrap_or("").trim_end_matches('0');
        let zero = whole == "0" && fraction.is_empty();
        let mut normalized = String::with_capacity(text.len());
        if text.starts_with('-') && !zero {
            normalized.push('-');
        }
        normalized.push_str(whole);
        if !fraction.is_empty() {
            normalized.push('.');
            normalized.push_str(fraction);
        }
        Ok(Self(normalized))
    }
    pub fn is_integer(&self) -> bool {
        !self.0.contains('.')
    }
    /// Convert a native JSON numeric token, including exponent notation, exactly.
    pub fn from_native_token(token: &str) -> ValidationResult<Self> {
        ensure(!token.is_empty() && token.len() <= FRAME_MAX)?;
        let split = token.find(['e', 'E']);
        let (mantissa, exponent) = if let Some(i) = split {
            let exponent = &token[i + 1..];
            let digits = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
            ensure(!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))?;
            (&token[..i], exponent)
        } else {
            (token, "0")
        };
        let unsigned = mantissa.strip_prefix('-').unwrap_or(mantissa);
        let mut parts = unsigned.split('.');
        let whole = parts.next().unwrap();
        let fraction = parts.next().unwrap_or("");
        ensure(
            valid_whole(whole)
                && parts.next().is_none()
                && fraction.bytes().all(|b| b.is_ascii_digit())
                && (!mantissa.contains('.') || !fraction.is_empty()),
        )?;
        let digits = format!("{whole}{fraction}");
        if digits.bytes().all(|b| b == b'0') {
            return Self::new("0");
        }
        let exponent = exponent.parse::<i32>().map_err(|_| invalid())?;
        let first = digits.find(|c| c != '0').unwrap();
        let last = digits.rfind(|c| c != '0').unwrap() + 1;
        let significant = &digits[first..last];
        let point = i64::try_from(whole.len()).map_err(|_| invalid())? + i64::from(exponent)
            - i64::try_from(first).map_err(|_| invalid())?;
        let significant_len = i64::try_from(significant.len()).map_err(|_| invalid())?;
        let body_len = if point <= 0 {
            2 - point + significant_len
        } else if point >= significant_len {
            point
        } else {
            significant_len + 1
        };
        ensure(body_len + i64::from(token.starts_with('-')) <= 128)?;
        let mut result = String::with_capacity(128);
        if token.starts_with('-') {
            result.push('-');
        }
        if point <= 0 {
            result.push_str("0.");
            result.extend(std::iter::repeat_n('0', (-point) as usize));
            result.push_str(significant);
        } else if point >= significant_len {
            result.push_str(significant);
            result.extend(std::iter::repeat_n('0', (point - significant_len) as usize));
        } else {
            result.push_str(&significant[..point as usize]);
            result.push('.');
            result.push_str(&significant[point as usize..]);
        }
        Self::new(&result)
    }
    pub fn native_number(&self) -> ValidationResult<Box<serde_json::value::RawValue>> {
        self.validate()?;
        serde_json::value::RawValue::from_string(self.0.clone()).map_err(|_| invalid())
    }
}
fn valid_whole(value: &str) -> bool {
    value == "0"
        || (!value.is_empty()
            && !value.starts_with('0')
            && value.bytes().all(|b| b.is_ascii_digit()))
}
impl Ord for Decimal {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let negative = self.0.starts_with('-');
        let other_negative = other.0.starts_with('-');
        if negative != other_negative {
            return if negative {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Greater
            };
        }
        fn parts(s: &str) -> (&str, &str) {
            let s = s.strip_prefix('-').unwrap_or(s);
            s.split_once('.').unwrap_or((s, ""))
        }
        let (a, af) = parts(&self.0);
        let (b, bf) = parts(&other.0);
        let mut order = a.len().cmp(&b.len()).then_with(|| a.cmp(b));
        if order.is_eq() {
            for i in 0..af.len().max(bf.len()) {
                order = af
                    .as_bytes()
                    .get(i)
                    .unwrap_or(&b'0')
                    .cmp(bf.as_bytes().get(i).unwrap_or(&b'0'));
                if !order.is_eq() {
                    break;
                }
            }
        }
        if negative { order.reverse() } else { order }
    }
}
impl PartialOrd for Decimal {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl<'de> Deserialize<'de> for Decimal {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Self::new(&s).map_err(serde::de::Error::custom)
    }
}
impl WireValidate for Decimal {
    fn validate(&self) -> Validation {
        ensure(Self::new(&self.0)?.0 == self.0)
    }
}

/// Native IDs never pass through Uint, f64 or an Android request key.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum NativeRpcId {
    String(String),
    Number(Box<serde_json::value::RawValue>),
}
impl PartialEq for NativeRpcId {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::String(a), Self::String(b)) => a == b,
            (Self::Number(a), Self::Number(b)) => a.get() == b.get(),
            _ => false,
        }
    }
}
impl Eq for NativeRpcId {}
impl NativeRpcId {
    pub fn from_raw(raw: &str) -> ValidationResult<Self> {
        if raw.starts_with('"') {
            return serde_json::from_str::<String>(raw)
                .map(Self::String)
                .map_err(|_| invalid());
        }
        ensure(valid_whole(raw.strip_prefix('-').unwrap_or(raw)))?;
        serde_json::value::RawValue::from_string(raw.to_owned())
            .map(Self::Number)
            .map_err(|_| invalid())
    }
}
impl<'de> Deserialize<'de> for NativeRpcId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = Box::<serde_json::value::RawValue>::deserialize(d)?;
        Self::from_raw(raw.get()).map_err(serde::de::Error::custom)
    }
}
