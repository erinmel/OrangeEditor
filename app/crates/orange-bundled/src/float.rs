//! Binary32/binary64 vocabulary. Bind once; evaluate without allocation or lookup.

use std::sync::Arc;

use orange_model::Value;
use orange_model::catalog::{NodeDefinition, Ports, ProviderKey, RegistrySnapshot, TypeArgs};
use orange_model::document::{
    ConfigValue, DefinitionRef, NodeKind, OutputSource, PortDescriptor, TypeParam,
};
use orange_model::id::TypeExpr;
use orange_model::scalar::{FloatPolicy, Scalar};
use serde_json::json;

use crate::{BindingError, CatalogError, KernelError, MathPackage};

/// Float-only pure operations. Every input and the output `result` have type `$T`.
///
/// Operations whose names L02a already gives `T: int` integer contracts use a
/// `float-` prefix: their NaN, signed-zero and domain rules differ, so they are
/// separate definitions rather than a silent widening of the integer contracts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloatOperation {
    SquareRoot,
    Exponential,
    NaturalLog,
    Log2,
    Log10,
    Power,
    Floor,
    Ceiling,
    Truncate,
    RoundTiesEven,
    Negate,
    Absolute,
    Minimum,
    Maximum,
    Clamp,
    Remainder,
}

impl FloatOperation {
    pub const ALL: [Self; 16] = [
        Self::SquareRoot,
        Self::Exponential,
        Self::NaturalLog,
        Self::Log2,
        Self::Log10,
        Self::Power,
        Self::Floor,
        Self::Ceiling,
        Self::Truncate,
        Self::RoundTiesEven,
        Self::Negate,
        Self::Absolute,
        Self::Minimum,
        Self::Maximum,
        Self::Clamp,
        Self::Remainder,
    ];

    pub const fn definition_id(self) -> &'static str {
        match self {
            Self::SquareRoot => "org.orange.math/square-root",
            Self::Exponential => "org.orange.math/exponential",
            Self::NaturalLog => "org.orange.math/natural-log",
            Self::Log2 => "org.orange.math/log2",
            Self::Log10 => "org.orange.math/log10",
            Self::Power => "org.orange.math/power",
            Self::Floor => "org.orange.math/floor",
            Self::Ceiling => "org.orange.math/ceiling",
            Self::Truncate => "org.orange.math/truncate",
            Self::RoundTiesEven => "org.orange.math/round-ties-even",
            Self::Negate => "org.orange.math/float-negate",
            Self::Absolute => "org.orange.math/float-absolute",
            Self::Minimum => "org.orange.math/float-minimum",
            Self::Maximum => "org.orange.math/float-maximum",
            Self::Clamp => "org.orange.math/float-clamp",
            Self::Remainder => "org.orange.math/float-remainder",
        }
    }

    pub fn from_definition_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|op| op.definition_id() == id)
    }

    /// Strict executable input order; every input has type `$T`.
    pub const fn input_ports(self) -> &'static [&'static str] {
        match self {
            Self::Power => &["base", "exponent"],
            Self::Minimum | Self::Maximum | Self::Remainder => &["a", "b"],
            Self::Clamp => &["value", "min", "max"],
            _ => &["value"],
        }
    }

    pub const fn output_port(self) -> &'static str {
        "result"
    }
}

/// Ordinary package definitions with `T: float` and exact f32/f64 specializations.
pub fn float_definitions() -> Result<Vec<NodeDefinition>, CatalogError> {
    let generic: TypeExpr = "$T".parse()?;
    FloatOperation::ALL
        .into_iter()
        .map(|operation| {
            let mut ports: Vec<_> = operation
                .input_ports()
                .iter()
                .map(|&name| PortDescriptor::data_in(name, generic.clone()))
                .collect();
            ports.push(PortDescriptor::data_out(
                operation.output_port(),
                generic.clone(),
                OutputSource::Computed,
            ));
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
                    constraints: vec!["float".into()],
                    extensions: Default::default(),
                }],
                ports: Ports::Declared(ports),
                references: vec![],
                specializations: None,
                config_schema: Some(json!({"$schema":"orange.config-schema/v1","type":"object",
                "properties":{"float_policy":{"enum":["finite","ieee"],"default":"finite"}},
                "additionalProperties":false})),
            })
        })
        .collect()
}

/// A pinned exact provider/type/config binding; no graph-validity claim is implied.
#[derive(Debug, Clone)]
pub struct ResolvedFloat {
    snapshot: Arc<RegistrySnapshot>,
    provider: ProviderKey,
    operation: FloatOperation,
    scalar: Scalar,
    policy: FloatPolicy,
}

impl ResolvedFloat {
    pub fn snapshot(&self) -> &Arc<RegistrySnapshot> {
        &self.snapshot
    }
    pub fn provider(&self) -> &ProviderKey {
        &self.provider
    }
    pub fn operation(&self) -> FloatOperation {
        self.operation
    }

    /// Exact values in declared order; wrong type/arity is a contract error. Finite
    /// policy checks non-finite inputs, then poles (`DivideByZero`), then the domain,
    /// then a non-finite result. IEEE policy returns the native IEEE-754 result.
    pub fn evaluate(&self, inputs: &[Value]) -> Result<Value, KernelError> {
        use FloatOperation::*;
        // IEEE 754-2019 minimum/maximum: NaN propagates (first NaN operand) and
        // -0 orders below +0. Native `min`/`max` ignore NaN and leave zeros unordered.
        macro_rules! minimum {
            ($a:expr, $b:expr) => {{
                let (a, b) = ($a, $b);
                if a.is_nan() || a < b || a == b && a.is_sign_negative() {
                    a
                } else if b.is_nan() || b < a || a == b {
                    b
                } else {
                    a
                }
            }};
        }
        macro_rules! maximum {
            ($a:expr, $b:expr) => {{
                let (a, b) = ($a, $b);
                if a.is_nan() || a > b || a == b && a.is_sign_positive() {
                    a
                } else if b.is_nan() || b > a || a == b {
                    b
                } else {
                    a
                }
            }};
        }
        macro_rules! float {
            ($variant:ident, $p:ty) => {{
                let arity = self.operation.input_ports().len();
                if inputs.len() != arity {
                    return Err(KernelError::InputContract);
                }
                let mut v: [$p; 3] = [0.0; 3];
                for (slot, input) in v.iter_mut().zip(inputs) {
                    let Value::$variant(x) = input else {
                        return Err(KernelError::InputContract);
                    };
                    *slot = *x;
                }
                let finite = self.policy == FloatPolicy::Finite;
                if finite && v[..arity].iter().any(|x| !x.is_finite()) {
                    return Err(KernelError::NonFinite);
                }
                let [a, b, c] = v;
                let result = match self.operation {
                    SquareRoot => {
                        if finite && a < 0.0 {
                            return Err(KernelError::Domain);
                        }
                        a.sqrt()
                    }
                    Exponential => a.exp(),
                    NaturalLog | Log2 | Log10 => {
                        if finite && a == 0.0 {
                            return Err(KernelError::DivideByZero);
                        }
                        if finite && a < 0.0 {
                            return Err(KernelError::Domain);
                        }
                        match self.operation {
                            NaturalLog => a.ln(),
                            Log2 => a.log2(),
                            _ => a.log10(),
                        }
                    }
                    Power => {
                        if finite && a == 0.0 && b < 0.0 {
                            return Err(KernelError::DivideByZero);
                        }
                        if finite && a < 0.0 && b.fract() != 0.0 {
                            return Err(KernelError::Domain);
                        }
                        a.powf(b)
                    }
                    Floor => a.floor(),
                    Ceiling => a.ceil(),
                    Truncate => a.trunc(),
                    RoundTiesEven => a.round_ties_even(),
                    Negate => -a,
                    Absolute => a.abs(),
                    Minimum => minimum!(a, b),
                    Maximum => maximum!(a, b),
                    Clamp => {
                        // Bounds must be ordered numbers in both policies; native
                        // clamp would panic instead.
                        if b.is_nan() || c.is_nan() || b > c {
                            return Err(KernelError::Domain);
                        }
                        let upper = minimum!(a, c);
                        maximum!(upper, b)
                    }
                    Remainder => {
                        if finite && b == 0.0 {
                            return Err(KernelError::DivideByZero);
                        }
                        a % b
                    }
                };
                if finite && !result.is_finite() {
                    return Err(KernelError::NonFinite);
                }
                Ok(Value::$variant(result))
            }};
        }
        match self.scalar {
            Scalar::F32 => float!(F32, f32),
            Scalar::F64 => float!(F64, f64),
            _ => Err(KernelError::InputContract),
        }
    }
}

impl MathPackage {
    /// Resolves only an exact registered f32/f64 specialization. Package-defined
    /// `float` evidence never selects native code for a different type or provider.
    pub fn resolve_float(
        &self,
        snapshot: Arc<RegistrySnapshot>,
        provider: &ProviderKey,
        definition: &DefinitionRef,
        config_version: u32,
        ty: &TypeExpr,
        config: &ConfigValue,
    ) -> Result<ResolvedFloat, BindingError> {
        let operation = FloatOperation::from_definition_id(definition.id.as_str())
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
            "core/f32@1" => Scalar::F32,
            "core/f64@1" => Scalar::F64,
            _ => return Err(BindingError::UnsupportedType),
        };
        if config.keys().any(|key| key != "float_policy") {
            return Err(BindingError::InvalidConfig);
        }
        let policy = match config.get("float_policy").map(|v| v.as_str()) {
            None | Some(Some("finite")) => FloatPolicy::Finite,
            Some(Some("ieee")) => FloatPolicy::Ieee,
            _ => return Err(BindingError::InvalidConfig),
        };
        Ok(ResolvedFloat {
            snapshot,
            provider: provider.clone(),
            operation,
            scalar,
            policy,
        })
    }
}
