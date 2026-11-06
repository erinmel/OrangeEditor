//! Strict manifest JSON and its canonical form (04 "Package format and SDK").
//!
//! A manifest must mean the same thing to every reader, so decoding rejects what readers
//! disagree on: duplicate object keys (first-wins and last-wins parsers would see different
//! manifests) and numbers other than integers within ±(2^53 − 1) (float and big-integer
//! handling differs across languages). The canonical form is the RFC 8785 (JCS) encoding of
//! such a value: no whitespace, object members sorted by the UTF-16 code units of their keys,
//! strings with only `"`, `\` and U+0000–U+001F escaped (`\b \t \n \f \r` short, the rest
//! `\u00xx` lowercase), integers in plain decimal.

use std::cmp::Ordering;
use std::fmt;

use serde::Deserialize;
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};

/// Largest integer magnitude a manifest may hold: every language's JSON reader agrees on it.
const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

/// A rule a decoded manifest value breaks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Problem {
    /// An object repeats a key; the key as written.
    DuplicateKey(String),
    /// A number is fractional, has an exponent or lies outside ±(2^53 − 1).
    Number,
}

/// A decoded value together with the first [`Problem`] found in it.
struct Checked {
    value: Value,
    problem: Option<Problem>,
}

/// Decodes `bytes` as one JSON value under the strict rules. The outer error is a syntax
/// failure; the inner one is the first strictness violation.
pub(crate) fn parse(bytes: &[u8]) -> Result<Result<Value, Problem>, serde_json::Error> {
    let checked: Checked = serde_json::from_slice(bytes)?;
    Ok(match checked.problem {
        Some(problem) => Err(problem),
        None => Ok(checked.value),
    })
}

/// Appends the canonical encoding of `value` to `out`.
pub(crate) fn write_canonical(value: &Value, out: &mut Vec<u8>) {
    match value {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(true) => out.extend_from_slice(b"true"),
        Value::Bool(false) => out.extend_from_slice(b"false"),
        Value::Number(number) => out.extend_from_slice(number.to_string().as_bytes()),
        Value::String(text) => write_string(text, out),
        Value::Array(items) => {
            out.push(b'[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(b',');
                }
                write_canonical(item, out);
            }
            out.push(b']');
        }
        Value::Object(members) => {
            let mut sorted: Vec<(&String, &Value)> = members.iter().collect();
            sorted.sort_by(|(a, _), (b, _)| utf16_order(a, b));
            out.push(b'{');
            for (index, (key, item)) in sorted.into_iter().enumerate() {
                if index > 0 {
                    out.push(b',');
                }
                write_string(key, out);
                out.push(b':');
                write_canonical(item, out);
            }
            out.push(b'}');
        }
    }
}

fn utf16_order(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

fn write_string(text: &str, out: &mut Vec<u8>) {
    out.push(b'"');
    for ch in text.chars() {
        match ch {
            '"' => out.extend_from_slice(b"\\\""),
            '\\' => out.extend_from_slice(b"\\\\"),
            '\u{8}' => out.extend_from_slice(b"\\b"),
            '\t' => out.extend_from_slice(b"\\t"),
            '\n' => out.extend_from_slice(b"\\n"),
            '\u{c}' => out.extend_from_slice(b"\\f"),
            '\r' => out.extend_from_slice(b"\\r"),
            control if u32::from(control) < 0x20 => {
                const HEX: &[u8; 16] = b"0123456789abcdef";
                let code = u32::from(control) as usize;
                out.extend_from_slice(b"\\u00");
                out.push(HEX[code >> 4]);
                out.push(HEX[code & 0xf]);
            }
            other => {
                let mut buffer = [0; 4];
                out.extend_from_slice(other.encode_utf8(&mut buffer).as_bytes());
            }
        }
    }
    out.push(b'"');
}

impl<'de> Deserialize<'de> for Checked {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(CheckedVisitor)
    }
}

struct CheckedVisitor;

impl CheckedVisitor {
    fn clean(value: Value) -> Checked {
        Checked {
            value,
            problem: None,
        }
    }

    fn integer(number: Number, magnitude: u64) -> Checked {
        Checked {
            value: Value::Number(number),
            problem: (magnitude > MAX_SAFE_INTEGER).then_some(Problem::Number),
        }
    }
}

impl<'de> Visitor<'de> for CheckedVisitor {
    type Value = Checked;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON value")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Checked, E> {
        Ok(Self::clean(Value::Bool(value)))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Checked, E> {
        Ok(Self::integer(value.into(), value.unsigned_abs()))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Checked, E> {
        Ok(Self::integer(value.into(), value))
    }

    fn visit_f64<E>(self, _value: f64) -> Result<Checked, E> {
        Ok(Checked {
            value: Value::Null,
            problem: Some(Problem::Number),
        })
    }

    fn visit_str<E>(self, value: &str) -> Result<Checked, E> {
        Ok(Self::clean(Value::String(value.to_owned())))
    }

    fn visit_string<E>(self, value: String) -> Result<Checked, E> {
        Ok(Self::clean(Value::String(value)))
    }

    fn visit_unit<E>(self) -> Result<Checked, E> {
        Ok(Self::clean(Value::Null))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Checked, A::Error> {
        let mut items = Vec::new();
        let mut problem = None;
        while let Some(item) = seq.next_element::<Checked>()? {
            problem = problem.or(item.problem);
            items.push(item.value);
        }
        Ok(Checked {
            value: Value::Array(items),
            problem,
        })
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Checked, A::Error> {
        let mut members = Map::new();
        let mut problem = None;
        while let Some(key) = map.next_key::<String>()? {
            let item: Checked = map.next_value()?;
            problem = problem.or(item.problem);
            if members.contains_key(&key) {
                problem = problem.or(Some(Problem::DuplicateKey(key)));
            } else {
                members.insert(key, item.value);
            }
        }
        Ok(Checked {
            value: Value::Object(members),
            problem,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canonical(text: &str) -> String {
        let value = parse(text.as_bytes()).unwrap().unwrap();
        let mut out = Vec::new();
        write_canonical(&value, &mut out);
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn rejects_duplicate_keys_at_any_depth() {
        let problem = parse(br#"{"a":1,"b":{"c":1,"c":2}}"#).unwrap().unwrap_err();
        assert_eq!(problem, Problem::DuplicateKey("c".into()));
        let problem = parse(br#"[{"x":1},{"x":1,"x":1}]"#).unwrap().unwrap_err();
        assert_eq!(problem, Problem::DuplicateKey("x".into()));
    }

    #[test]
    fn rejects_numbers_readers_disagree_on() {
        // `-0` is a float to serde_json and an integer elsewhere.
        for text in [
            "1.0",
            "1e2",
            "-0.5",
            "-0",
            "9007199254740992",
            "-9007199254740992",
        ] {
            assert_eq!(
                parse(text.as_bytes()).unwrap().unwrap_err(),
                Problem::Number,
                "{text}"
            );
        }
        for text in ["0", "-1", "9007199254740991", "-9007199254740991"] {
            assert!(parse(text.as_bytes()).unwrap().is_ok(), "{text}");
        }
    }

    /// Byte vectors any implementation of the canonical form must reproduce.
    #[test]
    fn canonical_vectors() {
        assert_eq!(
            canonical(" { \"b\" : [ 1 , true , null ] , \"a\" : \"x\" } "),
            r#"{"a":"x","b":[1,true,null]}"#
        );
        // Keys sort by UTF-16 code units: U+E000 (one unit 0xE000) sorts after U+1F600
        // (surrogates 0xD83D 0xDE00), the reverse of UTF-8 byte order.
        assert_eq!(
            canonical("{\"\u{e000}\":1,\"\u{1f600}\":2,\"A\":3,\"a\":4}"),
            "{\"A\":3,\"a\":4,\"\u{1f600}\":2,\"\u{e000}\":1}"
        );
        assert_eq!(
            canonical(r#""q\" s\\ \/ \b\f\n\r\t \u0001 \u001f \u007f é  ""#),
            "\"q\\\" s\\\\ / \\b\\f\\n\\r\\t \\u0001 \\u001f \u{7f} \u{e9} \u{2028}\""
        );
    }
}
