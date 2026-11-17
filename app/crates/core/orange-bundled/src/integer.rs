//! Fixed-width integer vocabulary. Bind once; evaluate without allocation or lookup.

use std::sync::Arc;

use orange_model::Value;
use orange_model::catalog::{NodeDefinition, Ports, ProviderKey, RegistrySnapshot, TypeArgs};
use orange_model::document::{
    ConfigValue, DefinitionRef, NodeKind, OutputSource, PortDescriptor, TypeParam,
};
use orange_model::id::TypeExpr;
use orange_model::scalar::{Overflow, Scalar};
use serde_json::json;

use crate::{BindingError, CatalogError, KernelError, MathPackage};

/// Integer-only pure operations. Every output is `result: $T`; shift counts are u32.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegerOperation {
    Remainder,
    Modulo,
    Negate,
    Absolute,
    Minimum,
    Maximum,
    Clamp,
    BitAnd,
    BitOr,
    BitXor,
    BitNot,
    ShiftLeft,
    ShiftRight,
}

impl IntegerOperation {
    pub const ALL: [Self; 13] = [
        Self::Remainder,
        Self::Modulo,
        Self::Negate,
        Self::Absolute,
        Self::Minimum,
        Self::Maximum,
        Self::Clamp,
        Self::BitAnd,
        Self::BitOr,
        Self::BitXor,
        Self::BitNot,
        Self::ShiftLeft,
        Self::ShiftRight,
    ];

    pub const fn definition_id(self) -> &'static str {
        match self {
            Self::Remainder => "org.orange.math/remainder",
            Self::Modulo => "org.orange.math/modulo",
            Self::Negate => "org.orange.math/negate",
            Self::Absolute => "org.orange.math/absolute",
            Self::Minimum => "org.orange.math/minimum",
            Self::Maximum => "org.orange.math/maximum",
            Self::Clamp => "org.orange.math/clamp",
            Self::BitAnd => "org.orange.math/bit-and",
            Self::BitOr => "org.orange.math/bit-or",
            Self::BitXor => "org.orange.math/bit-xor",
            Self::BitNot => "org.orange.math/bit-not",
            Self::ShiftLeft => "org.orange.math/shift-left",
            Self::ShiftRight => "org.orange.math/shift-right",
        }
    }

    pub fn from_definition_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|op| op.definition_id() == id)
    }

    /// Strict executable input order. Every input except `count` has type `$T`.
    pub const fn input_ports(self) -> &'static [&'static str] {
        match self {
            Self::Negate | Self::Absolute | Self::BitNot => &["value"],
            Self::Clamp => &["value", "min", "max"],
            Self::ShiftLeft | Self::ShiftRight => &["value", "count"],
            _ => &["a", "b"],
        }
    }

    pub const fn output_port(self) -> &'static str {
        "result"
    }

    fn has_overflow(self) -> bool {
        matches!(
            self,
            Self::Negate | Self::Absolute | Self::ShiftLeft | Self::ShiftRight
        )
    }

    fn is_shift(self) -> bool {
        matches!(self, Self::ShiftLeft | Self::ShiftRight)
    }
}

/// Ordinary package definitions with `T: int` and four exact native specializations.
pub fn integer_definitions() -> Result<Vec<NodeDefinition>, CatalogError> {
    let generic: TypeExpr = "$T".parse()?;
    let count: TypeExpr = "core/u32@1".parse()?;
    IntegerOperation::ALL
        .into_iter()
        .map(|operation| {
            let mut ports: Vec<_> = operation
                .input_ports()
                .iter()
                .map(|&name| {
                    PortDescriptor::data_in(
                        name,
                        if name == "count" {
                            count.clone()
                        } else {
                            generic.clone()
                        },
                    )
                })
                .collect();
            ports.push(PortDescriptor::data_out(
                operation.output_port(),
                generic.clone(),
                OutputSource::Computed,
            ));
            let properties = if operation.has_overflow() {
                let choices = if operation.is_shift() {
                    json!(["checked", "wrapping"])
                } else {
                    json!(["checked", "wrapping", "saturating"])
                };
                json!({"int_overflow":{"enum":choices,"default":"checked"}})
            } else {
                json!({})
            };
            Ok(NodeDefinition {
                definition: DefinitionRef {
                    id: operation.definition_id().parse()?,
                    version: 1,
                },
                config_version: 1,
                kind: NodeKind::Pure,
                volatility: None,
                determinism: orange_model::catalog::Determinism::Deterministic,
                suspends: None,
                type_params: vec![TypeParam {
                    name: "T".into(),
                    constraints: vec!["int".into()],
                    extensions: Default::default(),
                }],
                ports: Ports::Declared(ports),
                references: vec![],
                specializations: None,
                config_schema: Some(json!({"$schema":"orange.config-schema/v1","type":"object",
                "properties":properties,"additionalProperties":false})),
            })
        })
        .collect()
}

/// A pinned exact provider/type/config binding; no graph-validity claim is implied.
#[derive(Debug, Clone)]
pub struct ResolvedInteger {
    snapshot: Arc<RegistrySnapshot>,
    provider: ProviderKey,
    operation: IntegerOperation,
    scalar: Scalar,
    overflow: Overflow,
}

impl ResolvedInteger {
    pub fn snapshot(&self) -> &Arc<RegistrySnapshot> {
        &self.snapshot
    }
    pub fn provider(&self) -> &ProviderKey {
        &self.provider
    }
    pub fn operation(&self) -> IntegerOperation {
        self.operation
    }

    /// Exact values in declared order; wrong type/arity is a contract error. Integer
    /// arithmetic is fixed-width, and widened remainder never divides a native MIN/-1.
    pub fn evaluate(&self, inputs: &[Value]) -> Result<Value, KernelError> {
        use IntegerOperation::*;
        macro_rules! integer {
            ($variant:ident, $primitive:ty) => {{
                let value = match self.operation {
                    Negate | Absolute | BitNot => {
                        let [Value::$variant(a)] = inputs else {
                            return Err(KernelError::InputContract);
                        };
                        match self.operation {
                            BitNot => !a,
                            Negate => match self.overflow {
                                Overflow::Checked => (0 as $primitive)
                                    .checked_sub(*a)
                                    .ok_or(KernelError::NumericOverflow)?,
                                Overflow::Wrapping => (0 as $primitive).wrapping_sub(*a),
                                Overflow::Saturating => (0 as $primitive).saturating_sub(*a),
                            },
                            _ => match <$primitive>::try_from(i128::from(*a).abs()) {
                                Ok(value) => value,
                                Err(_) => match self.overflow {
                                    Overflow::Checked => return Err(KernelError::NumericOverflow),
                                    Overflow::Wrapping => *a,
                                    Overflow::Saturating => <$primitive>::MAX,
                                },
                            },
                        }
                    }
                    Clamp => {
                        let [
                            Value::$variant(value),
                            Value::$variant(min),
                            Value::$variant(max),
                        ] = inputs
                        else {
                            return Err(KernelError::InputContract);
                        };
                        if min > max {
                            return Err(KernelError::Domain);
                        }
                        (*value).clamp(*min, *max)
                    }
                    ShiftLeft | ShiftRight => {
                        let [Value::$variant(value), Value::U32(count)] = inputs else {
                            return Err(KernelError::InputContract);
                        };
                        // Checked shifts validate the count, not discarded value bits.
                        match (self.operation, self.overflow) {
                            (ShiftLeft, Overflow::Checked) => {
                                value.checked_shl(*count).ok_or(KernelError::OutOfRange)?
                            }
                            (ShiftRight, Overflow::Checked) => {
                                value.checked_shr(*count).ok_or(KernelError::OutOfRange)?
                            }
                            (ShiftLeft, Overflow::Wrapping) => value.wrapping_shl(*count),
                            (ShiftRight, Overflow::Wrapping) => value.wrapping_shr(*count),
                            _ => return Err(KernelError::InputContract),
                        }
                    }
                    _ => {
                        let [Value::$variant(a), Value::$variant(b)] = inputs else {
                            return Err(KernelError::InputContract);
                        };
                        match self.operation {
                            Remainder | Modulo => {
                                if *b == 0 {
                                    return Err(KernelError::DivideByZero);
                                }
                                let (a, b) = (i128::from(*a), i128::from(*b));
                                let result = if self.operation == Remainder {
                                    a % b
                                } else {
                                    a.rem_euclid(b)
                                };
                                <$primitive>::try_from(result)
                                    .map_err(|_| KernelError::NumericOverflow)?
                            }
                            Minimum => (*a).min(*b),
                            Maximum => (*a).max(*b),
                            BitAnd => a & b,
                            BitOr => a | b,
                            BitXor => a ^ b,
                            _ => return Err(KernelError::InputContract),
                        }
                    }
                };
                Ok(Value::$variant(value))
            }};
        }
        match self.scalar {
            Scalar::I32 => integer!(I32, i32),
            Scalar::I64 => integer!(I64, i64),
            Scalar::U32 => integer!(U32, u32),
            Scalar::U64 => integer!(U64, u64),
            _ => Err(KernelError::InputContract),
        }
    }
}

impl MathPackage {
    /// Resolves only an exact registered four-integer specialization. Package-defined
    /// `int` evidence never selects native code for a different type or provider.
    pub fn resolve_integer(
        &self,
        snapshot: Arc<RegistrySnapshot>,
        provider: &ProviderKey,
        definition: &DefinitionRef,
        config_version: u32,
        ty: &TypeExpr,
        config: &ConfigValue,
    ) -> Result<ResolvedInteger, BindingError> {
        let operation = IntegerOperation::from_definition_id(definition.id.as_str())
            .ok_or(BindingError::Unavailable)?;
        if provider != self.provider_key()
            || definition.version != 1
            || config_version != 1
            || !snapshot.provides(definition, config_version, provider)
        {
            return Err(BindingError::Unavailable);
        }
        let args = TypeArgs::from([("T".into(), ty.clone())]);
        if snapshot
            .provider(provider)
            .and_then(|p| p.specializations.get(definition))
            .is_none_or(|list| !list.contains(&args))
        {
            return Err(BindingError::UnsupportedType);
        }
        let scalar = match ty.as_str() {
            "core/i32@1" => Scalar::I32,
            "core/i64@1" => Scalar::I64,
            "core/u32@1" => Scalar::U32,
            "core/u64@1" => Scalar::U64,
            _ => return Err(BindingError::UnsupportedType),
        };
        if config.keys().any(|key| key != "int_overflow")
            || !operation.has_overflow() && !config.is_empty()
        {
            return Err(BindingError::InvalidConfig);
        }
        let overflow = match config.get("int_overflow").map(|v| v.as_str()) {
            None | Some(Some("checked")) => Overflow::Checked,
            Some(Some("wrapping")) => Overflow::Wrapping,
            Some(Some("saturating")) if !operation.is_shift() => Overflow::Saturating,
            _ => return Err(BindingError::InvalidConfig),
        };
        Ok(ResolvedInteger {
            snapshot,
            provider: provider.clone(),
            operation,
            scalar,
            overflow,
        })
    }
}
