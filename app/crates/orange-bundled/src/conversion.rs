//! Executable implementations of the Core converter contracts: every ordered pair of
//! distinct i32/i64/u32/u64/f32/f64 types, plus the text entries in `text_conversion`.

use std::sync::Arc;

use orange_model::Value;
use orange_model::catalog::RegistrySnapshot;
use orange_model::conversion::{ConversionClass, ConverterConfig, ConverterKey, ConverterProvider};
use orange_model::value::JsonText;

use crate::text_conversion::TextConversion;
use crate::{BindingError, ConversionError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Numeric {
    I32,
    I64,
    U32,
    U64,
    F32,
    F64,
}

impl Numeric {
    fn parse(ty: &str) -> Option<Self> {
        Some(match ty {
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
    fn integer(self, value: i128) -> Result<Value, ConversionError> {
        let out_of_range = |_| ConversionError::OutOfRange;
        Ok(match self {
            Self::I32 => Value::I32(i32::try_from(value).map_err(out_of_range)?),
            Self::I64 => Value::I64(i64::try_from(value).map_err(out_of_range)?),
            Self::U32 => Value::U32(u32::try_from(value).map_err(out_of_range)?),
            Self::U64 => Value::U64(u64::try_from(value).map_err(out_of_range)?),
            _ => return Err(ConversionError::InputContract),
        })
    }
}

/// Explicit float-to-integer rounding, applied before the range check.
#[derive(Debug, Clone, Copy)]
enum Rounding {
    Truncate,
    Floor,
    Ceiling,
    NearestEven,
}

#[derive(Debug, Clone, Copy)]
struct Cast {
    from: Numeric,
    to: Numeric,
    rounding: Rounding,
}

#[derive(Debug, Clone, Copy)]
enum Conversion {
    Cast(Cast),
    Text(TextConversion),
}

/// Result ports of one explicit conversion. Only a lossy entry supplies `loss`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversionOutput {
    pub result: Value,
    pub loss: Option<Value>,
}

/// Exact core entry bound to its immutable registry snapshot, with all options checked.
/// This is not evidence of graph validity, freshness or compiler constant evaluation.
#[derive(Debug, Clone)]
pub struct ResolvedConversion {
    snapshot: Arc<RegistrySnapshot>,
    entry: ConverterKey,
    operation: Conversion,
    loss_flags: Option<[Value; 2]>,
}

impl ResolvedConversion {
    pub fn snapshot(&self) -> &Arc<RegistrySnapshot> {
        &self.snapshot
    }
    pub fn entry(&self) -> &ConverterKey {
        &self.entry
    }
    pub fn provider(&self) -> ConverterProvider {
        ConverterProvider::Core
    }
    /// Whether this entry also produces the `loss` port.
    pub(crate) fn lossy(&self) -> bool {
        self.loss_flags.is_some()
    }

    /// Executes one value conversion. Input variants must match the bound source type.
    /// Loss metadata is prebuilt at binding; evaluation performs no registry lookup,
    /// policy parsing or buffer construction, including on the lossy success path.
    pub fn evaluate(&self, input: &Value) -> Result<ConversionOutput, ConversionError> {
        let Cast { from, to, rounding } = match self.operation {
            Conversion::Cast(cast) => cast,
            Conversion::Text(text) => {
                let (result, lost) = text.evaluate(input)?;
                return Ok(self.output(result, lost));
            }
        };
        let (result, lost) = match (input, from) {
            (Value::F32(value), Numeric::F32) if to == Numeric::F64 => {
                (Value::F64(widen_float(*value)), false)
            }
            (Value::F64(value), Numeric::F64) if to == Numeric::F32 => {
                let result = narrow_float(*value)?;
                let lost = widen_float(result).to_bits() != value.to_bits();
                (Value::F32(result), lost)
            }
            (Value::F32(value), Numeric::F32) => {
                (to.integer(round(f64::from(*value), rounding)?)?, false)
            }
            (Value::F64(value), Numeric::F64) => (to.integer(round(*value, rounding)?)?, false),
            (Value::I32(value), Numeric::I32) => integer_to(to, i128::from(*value))?,
            (Value::I64(value), Numeric::I64) => integer_to(to, i128::from(*value))?,
            (Value::U32(value), Numeric::U32) => integer_to(to, i128::from(*value))?,
            (Value::U64(value), Numeric::U64) => integer_to(to, i128::from(*value))?,
            _ => return Err(ConversionError::InputContract),
        };
        Ok(self.output(result, lost))
    }

    fn output(&self, result: Value, lost: bool) -> ConversionOutput {
        ConversionOutput {
            result,
            loss: self
                .loss_flags
                .as_ref()
                .map(|flags| flags[usize::from(lost)].clone()),
        }
    }
}

/// Binds an exact existing core entry. Package alternatives never fall back to Core;
/// defaults for new bindings never fill missing persisted options here. Graph/node
/// version checks, exact lock resolution and fresh semantic evidence belong to the caller.
pub fn resolve_core_conversion(
    snapshot: Arc<RegistrySnapshot>,
    config: &ConverterConfig,
    provider: &ConverterProvider,
) -> Result<ResolvedConversion, BindingError> {
    if provider != &ConverterProvider::Core || config.entry.contract_version != 1 {
        return Err(BindingError::Unavailable);
    }
    let registry = snapshot.converters();
    let definition = registry
        .entry(&config.entry)
        .ok_or(BindingError::Unavailable)?;
    if !registry
        .providers(&config.entry)
        .is_some_and(|mut registered| registered.any(|candidate| candidate == provider))
    {
        return Err(BindingError::Unavailable);
    }
    if config.options.len() != definition.policies.len()
        || definition.policies.iter().any(|policy| {
            config
                .options
                .get(&policy.name)
                .is_none_or(|value| !policy.choices.contains(value))
        })
    {
        return Err(BindingError::InvalidConfig);
    }
    let loss = |lossy: bool| -> Result<_, BindingError> {
        Ok(if lossy {
            Some([loss_flag(false)?, loss_flag(true)?])
        } else {
            None
        })
    };
    if let Some((text, lossy)) =
        crate::text_conversion::bind(config.entry.id.as_str(), definition, |name| {
            config.options.get(name).cloned()
        })?
    {
        return Ok(ResolvedConversion {
            snapshot,
            entry: config.entry.clone(),
            operation: Conversion::Text(text),
            loss_flags: loss(lossy)?,
        });
    }
    let (Some(from), Some(to)) = (
        Numeric::parse(definition.from.as_str()),
        Numeric::parse(definition.to.as_str()),
    ) else {
        return Err(BindingError::Unavailable);
    };
    if from == to
        || config.entry.id.as_str() != format!("core/{}-to-{}", from.name(), to.name())
        || definition.class != expected_class(from, to)
    {
        return Err(BindingError::Unavailable);
    }
    // The executable contract is fixed per pair; registry metadata cannot widen it.
    let option = |name: &str| config.options.get(name).map(String::as_str);
    let mut rounding = Rounding::Truncate;
    let expected = match (from.is_float(), to.is_float()) {
        (true, true) if from == Numeric::F64 => {
            option("rounding") == Some("nearest-even") && option("overflow") == Some("reject")
        }
        (true, false) => {
            rounding = match option("rounding") {
                Some("truncate") => Rounding::Truncate,
                Some("floor") => Rounding::Floor,
                Some("ceiling") => Rounding::Ceiling,
                Some("nearest-even") => Rounding::NearestEven,
                _ => return Err(BindingError::InvalidConfig),
            };
            config.options.len() == 1
        }
        _ if definition.class == ConversionClass::Lossy => {
            config.options.len() == 1 && option("rounding") == Some("nearest-even")
        }
        _ => config.options.is_empty(),
    };
    if !expected {
        return Err(BindingError::InvalidConfig);
    }
    let loss_flags = loss(definition.class == ConversionClass::Lossy)?;
    Ok(ResolvedConversion {
        snapshot,
        entry: config.entry.clone(),
        operation: Conversion::Cast(Cast { from, to, rounding }),
        loss_flags,
    })
}

/// The Core class of a numeric pair; it must match the registered entry exactly.
fn expected_class(from: Numeric, to: Numeric) -> ConversionClass {
    use Numeric::*;
    match (from, to) {
        (I32, I64) | (U32, I64) | (U32, U64) | (I32 | U32, F64) | (F32, F64) => {
            ConversionClass::LosslessTotal
        }
        (F64, F32) => ConversionClass::Lossy,
        (_, F32 | F64) if !from.is_float() => ConversionClass::Lossy,
        _ => ConversionClass::CheckedPartial,
    }
}

/// Integer source to any other numeric type. Integer targets are checked; float targets
/// round to nearest, ties to even (Rust `as` from integers), and report whether the
/// result differs from the source. Every result fits i128 exactly (|x| <= 2^64).
fn integer_to(to: Numeric, value: i128) -> Result<(Value, bool), ConversionError> {
    Ok(match to {
        Numeric::F32 => {
            let result = value as f32;
            (Value::F32(result), result as i128 != value)
        }
        Numeric::F64 => {
            let result = value as f64;
            (Value::F64(result), result as i128 != value)
        }
        _ => (to.integer(value)?, false),
    })
}

/// Rounds a finite float to an integral value and returns it exactly as i128 when it
/// lies in the widest target range; the caller narrows to the exact target.
fn round(value: f64, rounding: Rounding) -> Result<i128, ConversionError> {
    if !value.is_finite() {
        return Err(ConversionError::NonFinite);
    }
    let rounded = match rounding {
        Rounding::Truncate => value.trunc(),
        Rounding::Floor => value.floor(),
        Rounding::Ceiling => value.ceil(),
        Rounding::NearestEven => value.round_ties_even(),
    };
    // -2^63 and 2^64 are exact binary64 values; outside them no target can hold it.
    if !(-9_223_372_036_854_775_808.0..18_446_744_073_709_551_616.0).contains(&rounded) {
        return Err(ConversionError::OutOfRange);
    }
    Ok(rounded as i128)
}

fn loss_flag(lost: bool) -> Result<Value, BindingError> {
    let text = if lost {
        r#"{"lost":true}"#
    } else {
        r#"{"lost":false}"#
    };
    let json = JsonText::try_from(text).map_err(|_| BindingError::Unavailable)?;
    Ok(Value::Json(Arc::new(json)))
}

fn widen_float(value: f32) -> f64 {
    let bits = value.to_bits();
    let fraction = bits & 0x007f_ffff;
    if bits & 0x7f80_0000 == 0x7f80_0000 && fraction != 0 {
        // Preserve NaN sign, signaling/quiet bit and payload; arithmetic casts may
        // normalize NaNs. The 29 extra fraction bits remain zero.
        let sign = u64::from(bits & 0x8000_0000) << 32;
        f64::from_bits(sign | 0x7ff0_0000_0000_0000 | (u64::from(fraction) << 29))
    } else {
        f64::from(value)
    }
}

fn narrow_float(value: f64) -> Result<f32, ConversionError> {
    let bits = value.to_bits();
    let fraction = bits & 0x000f_ffff_ffff_ffff;
    if bits & 0x7ff0_0000_0000_0000 == 0x7ff0_0000_0000_0000 && fraction != 0 {
        let sign = ((bits >> 32) as u32) & 0x8000_0000;
        let payload = ((fraction >> 29) as u32).max(1);
        return Ok(f32::from_bits(sign | 0x7f80_0000 | payload));
    }
    if value.is_finite() && value.abs() > f64::from(f32::MAX) {
        return Err(ConversionError::OutOfRange);
    }
    Ok(value as f32)
}
