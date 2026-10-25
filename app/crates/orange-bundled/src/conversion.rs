//! Executable implementations of the four foundational core converter contracts.

use std::sync::Arc;

use orange_model::Value;
use orange_model::catalog::RegistrySnapshot;
use orange_model::conversion::{ConversionClass, ConverterConfig, ConverterKey, ConverterProvider};
use orange_model::value::JsonText;

use crate::{BindingError, ConversionError};

#[derive(Debug, Clone, Copy)]
enum Conversion {
    I32ToI64,
    I64ToI32,
    F32ToF64,
    F64ToF32,
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

    /// Executes one value conversion. Input variants must match the bound source type.
    /// Loss metadata is prebuilt at binding; evaluation performs no registry lookup,
    /// policy parsing or buffer construction, including on the lossy success path.
    pub fn evaluate(&self, input: &Value) -> Result<ConversionOutput, ConversionError> {
        let (result, lost) = match (self.operation, input) {
            (Conversion::I32ToI64, Value::I32(value)) => (Value::I64(i64::from(*value)), false),
            (Conversion::I64ToI32, Value::I64(value)) => {
                let value = i32::try_from(*value).map_err(|_| ConversionError::OutOfRange)?;
                (Value::I32(value), false)
            }
            (Conversion::F32ToF64, Value::F32(value)) => (Value::F64(widen_float(*value)), false),
            (Conversion::F64ToF32, Value::F64(value)) => {
                let result = narrow_float(*value)?;
                let lost = widen_float(result).to_bits() != value.to_bits();
                (Value::F32(result), lost)
            }
            _ => return Err(ConversionError::InputContract),
        };
        Ok(ConversionOutput {
            result,
            loss: self
                .loss_flags
                .as_ref()
                .map(|flags| flags[usize::from(lost)].clone()),
        })
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
    let operation = match (
        config.entry.id.as_str(),
        definition.from.as_str(),
        definition.to.as_str(),
        definition.class,
    ) {
        ("core/i32-to-i64", "core/i32@1", "core/i64@1", ConversionClass::LosslessTotal) => {
            Conversion::I32ToI64
        }
        ("core/i64-to-i32", "core/i64@1", "core/i32@1", ConversionClass::CheckedPartial) => {
            Conversion::I64ToI32
        }
        ("core/f32-to-f64", "core/f32@1", "core/f64@1", ConversionClass::LosslessTotal) => {
            Conversion::F32ToF64
        }
        ("core/f64-to-f32", "core/f64@1", "core/f32@1", ConversionClass::Lossy) => {
            Conversion::F64ToF32
        }
        _ => return Err(BindingError::Unavailable),
    };
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
    let loss_flags = if matches!(operation, Conversion::F64ToF32) {
        if config.options.len() != 2
            || config.options.get("rounding").map(String::as_str) != Some("nearest-even")
            || config.options.get("overflow").map(String::as_str) != Some("reject")
        {
            return Err(BindingError::InvalidConfig);
        }
        Some([loss_flag(false)?, loss_flag(true)?])
    } else {
        if !config.options.is_empty() {
            return Err(BindingError::InvalidConfig);
        }
        None
    };
    Ok(ResolvedConversion {
        snapshot,
        entry: config.entry.clone(),
        operation,
        loss_flags,
    })
}

fn loss_flag(lost: bool) -> Result<Value, BindingError> {
    let text = if lost {
        r#"{"lost":true}"#
    } else {
        r#"{"lost":false}"#
    };
    let json = JsonText::new(text.to_owned()).map_err(|_| BindingError::Unavailable)?;
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
