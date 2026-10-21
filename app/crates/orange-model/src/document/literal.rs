//! Literal wire shape (12 §6.1). Only the top-level literal carries `type`; the nested
//! payload stays untyped JSON here. Typed decoding per core type arrives with `Value` (00b.4).

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::error::LiteralError;
use crate::id::TypeExpr;

/// The encoded payload: a JSON `value`, or the IEEE bit pattern of an f32/f64.
#[derive(Debug, Clone, PartialEq)]
pub enum LiteralPayload {
    Value(Json),
    /// 8 or 16 lowercase hex digits.
    Bits(String),
}

/// A typed constant: an input literal, a variable initializer or a port default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "LiteralWire", into = "LiteralWire")]
pub struct Literal {
    pub ty: TypeExpr,
    pub payload: LiteralPayload,
    /// Non-authoritative display text for float literals.
    pub display: Option<String>,
}

impl Literal {
    pub fn value(ty: TypeExpr, value: Json) -> Self {
        Self {
            ty,
            payload: LiteralPayload::Value(value),
            display: None,
        }
    }
}

/// A literal is a closed value encoding, not an extensible object: unknown keys are an
/// error rather than silently dropped.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LiteralWire {
    #[serde(rename = "type")]
    ty: TypeExpr,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    value: Option<Json>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bits: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    display: Option<String>,
}

impl TryFrom<LiteralWire> for Literal {
    type Error = LiteralError;

    fn try_from(wire: LiteralWire) -> Result<Self, LiteralError> {
        let payload = match (wire.value, wire.bits) {
            (Some(value), None) => LiteralPayload::Value(value),
            (None, Some(bits)) if is_bit_pattern(&bits) => LiteralPayload::Bits(bits),
            (None, Some(_)) => return Err(LiteralError::InvalidBits),
            _ => return Err(LiteralError::PayloadCount),
        };
        Ok(Self {
            ty: wire.ty,
            payload,
            display: wire.display,
        })
    }
}

impl From<Literal> for LiteralWire {
    fn from(literal: Literal) -> Self {
        let (value, bits) = match literal.payload {
            LiteralPayload::Value(value) => (Some(value), None),
            LiteralPayload::Bits(bits) => (None, Some(bits)),
        };
        Self {
            ty: literal.ty,
            value,
            bits,
            display: literal.display,
        }
    }
}

fn is_bit_pattern(bits: &str) -> bool {
    matches!(bits.len(), 8 | 16)
        && bits
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(json: Json) -> Result<Literal, serde_json::Error> {
        serde_json::from_value(json)
    }

    #[test]
    fn value_and_bits_forms_round_trip() {
        for json in [
            json!({"type": "core/i64@1", "value": "100"}),
            json!({"type": "core/f64@1", "bits": "3ff0000000000000", "display": "1.0"}),
            json!({"type": "core/f32@1", "bits": "3f800000"}),
            json!({"type": "core/option@1<core/i64@1>", "value": {"none": true}}),
        ] {
            let literal = parse(json.clone()).unwrap();
            assert_eq!(serde_json::to_value(&literal).unwrap(), json);
        }
    }

    #[test]
    fn rejects_bad_payloads() {
        for json in [
            json!({"type": "core/i64@1"}),
            json!({"type": "core/i64@1", "value": null}),
            json!({"type": "core/f64@1", "value": "1", "bits": "3ff0000000000000"}),
            json!({"type": "core/f64@1", "bits": "3FF0000000000000"}),
            json!({"type": "core/f64@1", "bits": "3ff00"}),
            json!({"type": "core/i64@1", "value": "1", "unit": "ms"}),
            json!({"value": "1"}),
        ] {
            assert!(parse(json.clone()).is_err(), "{json}");
        }
    }
}
