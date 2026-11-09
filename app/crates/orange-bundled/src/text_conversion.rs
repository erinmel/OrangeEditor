//! Core text converters: invariant parse/display for bool and the six numeric
//! scalars, and UTF-8 encode/decode between `string` and `bytes`.

use std::sync::Arc;

use orange_model::Value;
use orange_model::conversion::{ConversionClass, ConverterDefinition};

use crate::{BindingError, ConversionError};

/// Scalar types with a Core text form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Textual {
    Bool,
    I32,
    I64,
    U32,
    U64,
    F32,
    F64,
}

impl Textual {
    fn parse(ty: &str) -> Option<Self> {
        Some(match ty {
            "core/bool@1" => Self::Bool,
            "core/i32@1" => Self::I32,
            "core/i64@1" => Self::I64,
            "core/u32@1" => Self::U32,
            "core/u64@1" => Self::U64,
            "core/f32@1" => Self::F32,
            "core/f64@1" => Self::F64,
            _ => return None,
        })
    }
    fn name(self) -> &'static str {
        match self {
            Self::Bool => "bool",
            Self::I32 => "i32",
            Self::I64 => "i64",
            Self::U32 => "u32",
            Self::U64 => "u64",
            Self::F32 => "f32",
            Self::F64 => "f64",
        }
    }
    fn is_float(self) -> bool {
        matches!(self, Self::F32 | Self::F64)
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum TextConversion {
    Display(Textual),
    Parse(Textual),
    Encode,
    Decode { replace: bool },
}

/// Recognizes a Core text entry and checks its fixed contract. `Ok(None)` means the
/// entry is not a text entry; any mismatch with the fixed Core contract is unavailable.
pub(crate) fn bind(
    id: &str,
    definition: &ConverterDefinition,
    option: impl Fn(&str) -> Option<String>,
) -> Result<Option<(TextConversion, bool)>, BindingError> {
    let (from, to) = (definition.from.as_str(), definition.to.as_str());
    let (conversion, class) = match (from, to) {
        ("core/string@1", "core/bytes@1") => {
            (TextConversion::Encode, ConversionClass::LosslessTotal)
        }
        ("core/bytes@1", "core/string@1") if id == "core/bytes-to-string" => (
            TextConversion::Decode { replace: false },
            ConversionClass::CheckedPartial,
        ),
        ("core/bytes@1", "core/string@1") => {
            if option("invalid").as_deref() != Some("replace") {
                return Err(BindingError::InvalidConfig);
            }
            (
                TextConversion::Decode { replace: true },
                ConversionClass::Lossy,
            )
        }
        ("core/string@1", _) => match Textual::parse(to) {
            Some(target) => {
                if target.is_float() && option("rounding").as_deref() != Some("nearest-even") {
                    return Err(BindingError::InvalidConfig);
                }
                (
                    TextConversion::Parse(target),
                    ConversionClass::CheckedPartial,
                )
            }
            None => return Ok(None),
        },
        (_, "core/string@1") => match Textual::parse(from) {
            Some(source) => (
                TextConversion::Display(source),
                if source.is_float() {
                    ConversionClass::Lossy
                } else {
                    ConversionClass::LosslessTotal
                },
            ),
            None => return Ok(None),
        },
        _ => return Ok(None),
    };
    let expected_id = match conversion {
        TextConversion::Display(source) => format!("core/{}-to-string", source.name()),
        TextConversion::Parse(target) => format!("core/string-to-{}", target.name()),
        TextConversion::Encode => "core/string-to-bytes".into(),
        TextConversion::Decode { replace: false } => "core/bytes-to-string".into(),
        TextConversion::Decode { replace: true } => "core/bytes-to-string-lossy".into(),
    };
    if id != expected_id || definition.class != class {
        return Err(BindingError::Unavailable);
    }
    Ok(Some((conversion, class == ConversionClass::Lossy)))
}

impl TextConversion {
    /// Returns the result and whether the lossy entry lost information.
    pub(crate) fn evaluate(self, input: &Value) -> Result<(Value, bool), ConversionError> {
        Ok(match (self, input) {
            (Self::Encode, Value::Str(text)) => {
                (Value::Bytes(Arc::new(text.as_bytes().to_vec())), false)
            }
            (Self::Decode { replace }, Value::Bytes(bytes)) => match std::str::from_utf8(bytes) {
                Ok(text) => (Value::Str(Arc::new(text.to_owned())), false),
                Err(_) if replace => (
                    Value::Str(Arc::new(String::from_utf8_lossy(bytes).into_owned())),
                    true,
                ),
                Err(_) => return Err(ConversionError::InvalidUtf8),
            },
            (Self::Display(source), value) => {
                let (text, lost) = display(source, value)?;
                (Value::Str(Arc::new(text)), lost)
            }
            (Self::Parse(target), Value::Str(text)) => (parse(target, text)?, false),
            _ => return Err(ConversionError::InputContract),
        })
    }
}

fn display(source: Textual, value: &Value) -> Result<(String, bool), ConversionError> {
    Ok(match (source, value) {
        (Textual::Bool, Value::Bool(value)) => (value.to_string(), false),
        (Textual::I32, Value::I32(value)) => (value.to_string(), false),
        (Textual::I64, Value::I64(value)) => (value.to_string(), false),
        (Textual::U32, Value::U32(value)) => (value.to_string(), false),
        (Textual::U64, Value::U64(value)) => (value.to_string(), false),
        (Textual::F32, Value::F32(value)) => (
            float_text(
                value.is_sign_negative(),
                value.is_nan(),
                value.is_infinite(),
                *value == 0.0,
                || format!("{:e}", value.abs()),
            ),
            value.is_nan() && value.to_bits() != f32::NAN.to_bits(),
        ),
        (Textual::F64, Value::F64(value)) => (
            float_text(
                value.is_sign_negative(),
                value.is_nan(),
                value.is_infinite(),
                *value == 0.0,
                || format!("{:e}", value.abs()),
            ),
            value.is_nan() && value.to_bits() != f64::NAN.to_bits(),
        ),
        _ => return Err(ConversionError::InputContract),
    })
}

/// Shortest round-trip digits in the ECMAScript Number-to-String layout, except that
/// negative zero keeps its sign: plain decimal for decimal exponents -7 < n <= 21,
/// otherwise `d.ddde±x`. NaN is `NaN` and infinities are `Infinity`/`-Infinity`.
fn float_text(
    negative: bool,
    nan: bool,
    infinite: bool,
    zero: bool,
    scientific: impl Fn() -> String,
) -> String {
    if nan {
        return "NaN".into();
    }
    let sign = if negative { "-" } else { "" };
    if infinite {
        return format!("{sign}Infinity");
    }
    if zero {
        return format!("{sign}0");
    }
    // `{:e}` prints the shortest digits that round-trip: `d[.ddd]e<exp>`.
    let text = scientific();
    let (mantissa, exponent) = text.split_once('e').unwrap_or((&text, "0"));
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let k = digits.len() as i64;
    let n = exponent.parse::<i64>().unwrap_or(0) + 1;
    let body = if k <= n && n <= 21 {
        format!("{digits}{}", "0".repeat((n - k) as usize))
    } else if 0 < n && n <= 21 {
        format!("{}.{}", &digits[..n as usize], &digits[n as usize..])
    } else if -6 < n && n <= 0 {
        format!("0.{}{digits}", "0".repeat((-n) as usize))
    } else {
        let fraction = if k > 1 {
            format!(".{}", &digits[1..])
        } else {
            String::new()
        };
        let e = n - 1;
        format!(
            "{}{fraction}e{}{}",
            &digits[..1],
            if e < 0 { '-' } else { '+' },
            e.abs()
        )
    };
    format!("{sign}{body}")
}

fn parse(target: Textual, text: &str) -> Result<Value, ConversionError> {
    match target {
        Textual::Bool => match text {
            "true" => Ok(Value::Bool(true)),
            "false" => Ok(Value::Bool(false)),
            _ => Err(ConversionError::Parse),
        },
        Textual::F32 | Textual::F64 => {
            let value = parse_float(text)?;
            if target == Textual::F32 {
                // Parse binary32 directly: a binary64 intermediate could double-round.
                let narrow: f32 = match value {
                    FloatText::Finite => text.parse().map_err(|_| ConversionError::Parse)?,
                    FloatText::Special(value) => value as f32,
                };
                if narrow.is_infinite() && matches!(value, FloatText::Finite) {
                    return Err(ConversionError::OutOfRange);
                }
                Ok(Value::F32(narrow))
            } else {
                let wide: f64 = match value {
                    FloatText::Finite => text.parse().map_err(|_| ConversionError::Parse)?,
                    FloatText::Special(value) => value,
                };
                if wide.is_infinite() && matches!(value, FloatText::Finite) {
                    return Err(ConversionError::OutOfRange);
                }
                Ok(Value::F64(wide))
            }
        }
        _ => {
            let bytes = text.as_bytes();
            let (negative, digits) = match bytes.first() {
                Some(b'-') => (true, &bytes[1..]),
                Some(b'+') => (false, &bytes[1..]),
                _ => (false, bytes),
            };
            if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
                return Err(ConversionError::Parse);
            }
            // Syntax is checked first; the value saturates past every target range.
            let mut magnitude: i128 = 0;
            for digit in digits {
                magnitude = (magnitude * 10 + i128::from(digit - b'0')).min(1 << 65);
            }
            let value = if negative { -magnitude } else { magnitude };
            let out = |_| ConversionError::OutOfRange;
            Ok(match target {
                Textual::I32 => Value::I32(i32::try_from(value).map_err(out)?),
                Textual::I64 => Value::I64(i64::try_from(value).map_err(out)?),
                Textual::U32 => Value::U32(u32::try_from(value).map_err(out)?),
                _ => Value::U64(u64::try_from(value).map_err(out)?),
            })
        }
    }
}

enum FloatText {
    Finite,
    Special(f64),
}

/// Invariant float grammar: `[+-]?(D+(.D*)?|.D+)([eE][+-]?D+)?`, `[+-]?Infinity` or
/// `NaN`, ASCII only, no surrounding whitespace. Rust's parser accepts a superset
/// (`inf`, `nan` in any case), so the syntax is checked here first.
fn parse_float(text: &str) -> Result<FloatText, ConversionError> {
    let bytes = text.as_bytes();
    let unsigned = match bytes.first() {
        Some(b'+' | b'-') => &bytes[1..],
        _ => bytes,
    };
    if unsigned == b"Infinity" {
        let negative = bytes[0] == b'-';
        return Ok(FloatText::Special(if negative {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        }));
    }
    if bytes == b"NaN" {
        return Ok(FloatText::Special(f64::NAN));
    }
    let digits = |s: &[u8]| s.iter().take_while(|b| b.is_ascii_digit()).count();
    let integer = digits(unsigned);
    let mut rest = &unsigned[integer..];
    let mut fraction = 0;
    if let Some(b'.') = rest.first() {
        fraction = digits(&rest[1..]);
        rest = &rest[1 + fraction..];
    }
    if integer + fraction == 0 {
        return Err(ConversionError::Parse);
    }
    if let Some(b'e' | b'E') = rest.first() {
        let exponent = match rest.get(1) {
            Some(b'+' | b'-') => &rest[2..],
            _ => &rest[1..],
        };
        let count = digits(exponent);
        if count == 0 {
            return Err(ConversionError::Parse);
        }
        rest = &exponent[count..];
    }
    if rest.is_empty() {
        Ok(FloatText::Finite)
    } else {
        Err(ConversionError::Parse)
    }
}
