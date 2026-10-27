//! Public product definitions and exact native provider resolution.

use std::collections::BTreeMap;
use std::sync::Arc;

use orange_model::catalog::{
    NodeDefinition, Ports, ProviderDescriptor, ProviderKey, RegistryBuilder, RegistrySnapshot,
    TypeArgs,
};
use orange_model::document::{
    ConfigValue, DefinitionRef, NodeKind, OutputSource, PortDescriptor, TypeParam,
};
use serde_json::json;

use crate::kernel::{FloatPolicy, Overflow, Scalar};
use crate::{BindingError, CatalogError, Operation, ResolvedKernel};

const NUMERIC_TYPES: [&str; 6] = [
    "core/i32@1",
    "core/i64@1",
    "core/u32@1",
    "core/u64@1",
    "core/f32@1",
    "core/f64@1",
];

/// Definitions are ordinary replaceable package contracts. The native provider supports
/// six scalar numeric specializations; a package `num` witness cannot select native code.
pub fn math_definitions() -> Result<Vec<NodeDefinition>, CatalogError> {
    let mut definitions = Vec::with_capacity(Operation::ALL.len());
    for operation in Operation::ALL {
        let boolean = !operation.is_numeric();
        let ty: orange_model::id::TypeExpr = if boolean { "core/bool@1" } else { "$T" }.parse()?;
        let mut ports: Vec<_> = operation
            .input_ports()
            .iter()
            .map(|&name| PortDescriptor::data_in(name, ty.clone()))
            .collect();
        ports.push(PortDescriptor::data_out(
            operation.output_port(),
            ty,
            OutputSource::Computed,
        ));
        let type_params = if boolean {
            vec![]
        } else {
            vec![TypeParam {
                name: "T".into(),
                constraints: vec!["num".into()],
                extensions: Default::default(),
            }]
        };
        let config_schema = if boolean {
            json!({
                "$schema":"orange.config-schema/v1", "type":"object",
                "properties":{}, "additionalProperties":false
            })
        } else {
            json!({
                "$schema":"orange.config-schema/v1", "type":"object",
                "properties":{
                    "int_overflow":{"enum":["checked","wrapping","saturating"],"default":"checked"},
                    "float_policy":{"enum":["finite","ieee"],"default":"finite"}
                }, "additionalProperties":false
            })
        };
        definitions.push(NodeDefinition {
            definition: DefinitionRef {
                id: operation.definition_id().parse()?,
                version: 1,
            },
            config_version: 1,
            kind: NodeKind::Pure,
            volatility: None,
            determinism: orange_model::catalog::Determinism::Deterministic,
            suspends: None,
            type_params,
            ports: Ports::Declared(ports),
            references: vec![],
            specializations: None,
            config_schema: Some(config_schema),
        });
    }
    Ok(definitions)
}

/// A bundled build identity authenticated by the host. This API validates digest syntax;
/// it does not hash executable bytes or authenticate the caller's build assertion.
#[derive(Debug, Clone)]
pub struct MathPackage {
    key: ProviderKey,
}

impl MathPackage {
    pub fn new(content_digest: impl Into<String>) -> Result<Self, CatalogError> {
        let content_digest = content_digest.into();
        let valid = content_digest.strip_prefix("sha256:").is_some_and(|hex| {
            hex.len() == 64
                && hex
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        });
        if !valid {
            return Err(CatalogError::InvalidBuildDigest);
        }
        Ok(Self {
            key: ProviderKey {
                package: "org.orange.math".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                content_digest,
                id: "native-v1".into(),
            },
        })
    }

    pub fn provider_key(&self) -> &ProviderKey {
        &self.key
    }

    /// Adds definitions and this exact provider to an unpublished builder. Duplicate
    /// definitions/providers are rejected by the ordinary registry publication rules.
    pub fn register(&self, mut builder: RegistryBuilder) -> Result<RegistryBuilder, CatalogError> {
        let mut definitions = math_definitions()?;
        definitions.push(crate::random_u64_definition()?);
        let numeric = NUMERIC_TYPES
            .into_iter()
            .map(|name| Ok(BTreeMap::from([("T".into(), name.parse()?)])))
            .collect::<Result<Vec<TypeArgs>, CatalogError>>()?;
        let mut provider = ProviderDescriptor {
            key: self.key.clone(),
            implements: Default::default(),
            specializations: Default::default(),
        };
        for definition in definitions {
            let supported = if definition.type_params.is_empty() {
                vec![TypeArgs::new()]
            } else {
                numeric.clone()
            };
            provider
                .implements
                .insert((definition.definition.clone(), definition.config_version));
            provider
                .specializations
                .insert(definition.definition.clone(), supported);
            builder = builder.definition(definition);
        }
        Ok(builder.provider(provider))
    }

    /// Resolves an exact native binding once, before execution. The caller supplies fresh
    /// type arguments and an exact ProviderKey derived from the graph's package lock.
    /// This checks kernel support, not graph validity or analysis freshness. Another
    /// selected provider never falls back to these bundled kernels.
    pub fn resolve(
        &self,
        snapshot: Arc<RegistrySnapshot>,
        provider: &ProviderKey,
        definition: &DefinitionRef,
        config_version: u32,
        type_args: &TypeArgs,
        config: &ConfigValue,
    ) -> Result<ResolvedKernel, BindingError> {
        if provider != &self.key
            || definition.version != 1
            || config_version != 1
            || !snapshot.provides(definition, config_version, provider)
        {
            return Err(BindingError::Unavailable);
        }
        if snapshot
            .provider(provider)
            .and_then(|descriptor| descriptor.specializations.get(definition))
            .is_none_or(|supported| !supported.contains(type_args))
        {
            return Err(BindingError::UnsupportedType);
        }
        let operation = Operation::from_definition_id(definition.id.as_str())
            .ok_or(BindingError::Unavailable)?;
        let scalar = if !operation.is_numeric() {
            if !type_args.is_empty() {
                return Err(BindingError::UnsupportedType);
            }
            Scalar::Bool
        } else {
            if type_args.len() != 1 {
                return Err(BindingError::UnsupportedType);
            }
            match type_args.get("T").map(|ty| ty.as_str()) {
                Some("core/i32@1") => Scalar::I32,
                Some("core/i64@1") => Scalar::I64,
                Some("core/u32@1") => Scalar::U32,
                Some("core/u64@1") => Scalar::U64,
                Some("core/f32@1") => Scalar::F32,
                Some("core/f64@1") => Scalar::F64,
                _ => return Err(BindingError::UnsupportedType),
            }
        };
        let (overflow, float_policy) = policies(operation, config)?;
        Ok(ResolvedKernel {
            snapshot,
            provider: provider.clone(),
            operation,
            scalar,
            overflow,
            float_policy,
        })
    }
}

fn policies(
    operation: Operation,
    config: &ConfigValue,
) -> Result<(Overflow, FloatPolicy), BindingError> {
    if !operation.is_numeric() && !config.is_empty()
        || config
            .keys()
            .any(|key| key != "int_overflow" && key != "float_policy")
    {
        return Err(BindingError::InvalidConfig);
    }
    let overflow = match config.get("int_overflow") {
        None => Overflow::Checked,
        Some(value) => match value.as_str() {
            Some("checked") => Overflow::Checked,
            Some("wrapping") => Overflow::Wrapping,
            Some("saturating") => Overflow::Saturating,
            _ => return Err(BindingError::InvalidConfig),
        },
    };
    let float_policy = match config.get("float_policy") {
        None => FloatPolicy::Finite,
        Some(value) => match value.as_str() {
            Some("finite") => FloatPolicy::Finite,
            Some("ieee") => FloatPolicy::Ieee,
            _ => return Err(BindingError::InvalidConfig),
        },
    };
    Ok((overflow, float_policy))
}
