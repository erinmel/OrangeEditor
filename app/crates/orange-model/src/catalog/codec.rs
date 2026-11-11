//! JSON decoding of package node definition files (12 §4).
//!
//! A package ships one definition per file in the 12 §4 wire shape. Decoding owns shape
//! only: the importer's repeated-key, JSON-budget and nesting checks run first, then the
//! implemented field subset is read with unknown fields rejected, so a declaration this
//! core does not model yet is refused instead of silently dropped. Contract rules (versions,
//! type parameters, ports, kinds, suspension, defaults) are checked when the definition is
//! published through [`RegistryBuilder`](super::RegistryBuilder).

use serde::Deserialize;
use thiserror::Error;

use super::{Determinism, NodeDefinition, Ports, Volatility};
use crate::document::codec::{ImportBudget, ScanError, scan_json};
use crate::document::nesting::MAX_NESTING;
use crate::document::{DefinitionRef, NodeKind, PortDescriptor, Suspends, TypeParam};
use crate::error::{ImportResource, JsonError, Rejected};
use crate::id::DefinitionId;

/// Safety limits for decoding one node definition file.
///
/// The defaults are 1 MiB of JSON text and 1,024 declared ports. `max_bytes` is checked
/// before parsing; a streaming pass then applies [`ImportBudget::default`] and the shared
/// nesting budget before any value is built. The port count is checked after decoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DefinitionLimits {
    pub max_bytes: usize,
    pub max_ports: usize,
}

impl DefinitionLimits {
    pub const fn new(max_bytes: usize, max_ports: usize) -> Self {
        Self {
            max_bytes,
            max_ports,
        }
    }
}

impl Default for DefinitionLimits {
    fn default() -> Self {
        Self::new(1024 * 1024, 1024)
    }
}

/// Why a node definition file is rejected. Messages name the failed rule, never file text.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DefinitionError {
    /// Malformed JSON, a missing or invalid field, an unknown field, or an unknown token.
    #[error("invalid node definition: {0}")]
    Json(JsonError),
    /// A byte or decoded-JSON budget is exceeded.
    #[error("node definition exceeds {resource} limit: {actual} > {max}")]
    LimitExceeded {
        resource: ImportResource,
        max: usize,
        actual: usize,
    },
    /// A JSON object repeats a key, at any depth. `key` is the first repeat, cut to 64
    /// characters.
    #[error("duplicate JSON object key")]
    DuplicateKey { key: Rejected },
    /// Objects and arrays nest deeper than the shared document budget.
    #[error("node definition nests JSON deeper than {max} levels")]
    NestingLimit { max: usize },
    /// More ports than [`DefinitionLimits::max_ports`].
    #[error("node definition declares {actual} ports; the limit is {max}")]
    TooManyPorts { max: usize, actual: usize },
}

impl DefinitionError {
    /// Stable message key for host presentation (13 §2).
    pub const fn message_key(&self) -> &'static str {
        match self {
            Self::Json(_) => "core/definition-json",
            Self::LimitExceeded { .. } => "core/definition-limit-exceeded",
            Self::DuplicateKey { .. } => "core/definition-duplicate-key",
            Self::NestingLimit { .. } => "core/definition-nesting-limit",
            Self::TooManyPorts { .. } => "core/definition-too-many-ports",
        }
    }
}

impl From<serde_json::Error> for DefinitionError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error.into())
    }
}

impl NodeDefinition {
    /// Decodes one package node definition file with [`DefinitionLimits::default`].
    pub fn from_json(text: &str) -> Result<Self, DefinitionError> {
        Self::from_json_with_limits(text, DefinitionLimits::default())
    }

    /// Decodes one package node definition file subject to `limits`.
    ///
    /// Reads the implemented 12 §4 subset: `definition_id`, `definition_version`,
    /// `config_version`, `kind`, `determinism`, and optionally `suspends`, `volatility`,
    /// `type_params`, `ports` and `config_schema`. Port descriptors keep their presentation
    /// fields (`label`, `description`, ...) as extensions, as saved signatures do. Every other
    /// top-level field, including the 12 §4 fields not modeled yet (`label`, `errors`,
    /// `capabilities`, `editor`, ...), is rejected. The result declares its ports; it has no
    /// reference fields and no core specialization list, since a package lists supported
    /// argument sets on its provider (11 §5.5). Bound host reads before building `text`.
    ///
    /// Errors come in this order: byte limit, decoded-JSON budget or malformed JSON, repeated
    /// key, nesting, shape, port count.
    pub fn from_json_with_limits(
        text: &str,
        limits: DefinitionLimits,
    ) -> Result<Self, DefinitionError> {
        if text.len() > limits.max_bytes {
            return Err(DefinitionError::LimitExceeded {
                resource: ImportResource::Bytes,
                max: limits.max_bytes,
                actual: text.len(),
            });
        }
        let scan = scan_json(text, ImportBudget::default()).map_err(|error| match error {
            ScanError::Limit {
                resource,
                max,
                actual,
            } => DefinitionError::LimitExceeded {
                resource,
                max,
                actual,
            },
            ScanError::Json(error) => DefinitionError::Json(error),
        })?;
        if let Some(key) = scan.duplicate {
            return Err(DefinitionError::DuplicateKey { key: key.into() });
        }
        if scan.nesting > MAX_NESTING {
            return Err(DefinitionError::NestingLimit { max: MAX_NESTING });
        }
        let wire: DefinitionWire = serde_json::from_str(text)?;
        if wire.ports.len() > limits.max_ports {
            return Err(DefinitionError::TooManyPorts {
                max: limits.max_ports,
                actual: wire.ports.len(),
            });
        }
        Ok(wire.into())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DefinitionWire {
    definition_id: DefinitionId,
    definition_version: u32,
    config_version: u32,
    kind: NodeKind,
    #[serde(default)]
    suspends: Option<Suspends>,
    #[serde(default)]
    volatility: Option<VolatilityWire>,
    determinism: DeterminismWire,
    #[serde(default)]
    type_params: Vec<TypeParam>,
    #[serde(default)]
    ports: Vec<PortDescriptor>,
    #[serde(default)]
    config_schema: Option<serde_json::Value>,
    /// The node uses a declared connection, named by its `connection` config field.
    #[serde(default)]
    connection: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum VolatilityWire {
    StateRead,
    Clock,
    Entropy,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum DeterminismWire {
    Deterministic,
    Seeded,
    External,
    Nondeterministic,
}

impl From<DefinitionWire> for NodeDefinition {
    fn from(wire: DefinitionWire) -> Self {
        Self {
            definition: DefinitionRef {
                id: wire.definition_id,
                version: wire.definition_version,
            },
            config_version: wire.config_version,
            kind: wire.kind,
            volatility: wire.volatility.map(|volatility| match volatility {
                VolatilityWire::StateRead => Volatility::StateRead,
                VolatilityWire::Clock => Volatility::Clock,
                VolatilityWire::Entropy => Volatility::Entropy,
            }),
            determinism: match wire.determinism {
                DeterminismWire::Deterministic => Determinism::Deterministic,
                DeterminismWire::Seeded => Determinism::Seeded,
                DeterminismWire::External => Determinism::External,
                DeterminismWire::Nondeterministic => Determinism::Nondeterministic,
            },
            suspends: wire.suspends,
            type_params: wire.type_params,
            ports: Ports::Declared(wire.ports),
            references: if wire.connection {
                vec![super::language::reference_field(super::ReferenceKind::Connection).clone()]
            } else {
                Vec::new()
            },
            specializations: None,
            config_schema: wire.config_schema,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::document::{OutputSource, PortDirection};

    fn triple() -> serde_json::Value {
        json!({
            "definition_id": "acme.math/triple", "definition_version": 1,
            "config_version": 1, "kind": "pure", "determinism": "deterministic",
            "type_params": [{"name": "T", "constraints": ["num"]}],
            "ports": [
                {"port_id": "value", "direction": "input", "kind": "data", "type": "$T",
                 "label": {"key": "acme.math/triple-port-value", "fallback": "Value"}},
                {"port_id": "result", "direction": "output", "kind": "data", "type": "$T",
                 "source": "computed"}
            ],
            "config_schema": {"type": "object", "additionalProperties": false}
        })
    }

    fn decode(value: &serde_json::Value) -> Result<NodeDefinition, DefinitionError> {
        NodeDefinition::from_json(&value.to_string())
    }

    #[test]
    fn decodes_the_implemented_subset() {
        let definition = decode(&triple()).unwrap();
        assert_eq!(definition.definition.id.as_str(), "acme.math/triple");
        assert_eq!(definition.definition.version, 1);
        assert_eq!(definition.kind, NodeKind::Pure);
        assert_eq!(definition.determinism, Determinism::Deterministic);
        assert_eq!((definition.volatility, definition.suspends), (None, None));
        assert_eq!(definition.type_params[0].constraints, ["num"]);
        let Ports::Declared(ports) = &definition.ports else {
            panic!("decoded ports are declared")
        };
        assert_eq!(ports[0].direction, PortDirection::Input);
        assert!(ports[0].extensions.contains_key("label"));
        assert_eq!(ports[1].source, Some(OutputSource::Computed));
        assert!(definition.references.is_empty());
        assert_eq!(definition.specializations, None);
        assert!(definition.config_schema.is_some());

        let mut entropy = triple();
        entropy["kind"] = json!("volatile");
        entropy["volatility"] = json!("entropy");
        entropy["determinism"] = json!("seeded");
        let entropy = decode(&entropy).unwrap();
        assert_eq!(entropy.volatility, Some(Volatility::Entropy));
        assert_eq!(entropy.determinism, Determinism::Seeded);
    }

    #[test]
    fn rejects_bad_shapes_unknown_fields_and_repeated_keys() {
        let json_error = |value: serde_json::Value| {
            assert!(
                matches!(decode(&value), Err(DefinitionError::Json(_))),
                "{value}"
            );
        };
        let mut missing = triple();
        missing.as_object_mut().unwrap().remove("determinism");
        json_error(missing);
        let mut unknown = triple();
        unknown["errors"] = json!(["core/NumericOverflow"]);
        json_error(unknown);
        let mut token = triple();
        token["kind"] = json!("macro");
        json_error(token);
        let mut id = triple();
        id["definition_id"] = json!("no namespace");
        json_error(id);
        let mut port = triple();
        port["ports"][0]["port_id"] = json!("Bad Port");
        json_error(port);
        assert!(matches!(
            NodeDefinition::from_json("{\"definition_id\": "),
            Err(DefinitionError::Json(_))
        ));

        let text = triple().to_string().replacen(
            "\"config_version\":1",
            "\"config_version\":1,\"config_version\":2",
            1,
        );
        let error = NodeDefinition::from_json(&text).unwrap_err();
        assert_eq!(error.message_key(), "core/definition-duplicate-key");
        let DefinitionError::DuplicateKey { key } = error else {
            unreachable!()
        };
        assert_eq!(key.reveal(), "config_version");
        // The rejected key is input text and never appears in messages.
        assert!(!format!("{key} {key:?}").contains("config_version"));
    }

    #[test]
    fn enforces_byte_port_and_nesting_limits() {
        let text = triple().to_string();
        assert_eq!(
            NodeDefinition::from_json_with_limits(&text, DefinitionLimits::new(10, 1024)),
            Err(DefinitionError::LimitExceeded {
                resource: ImportResource::Bytes,
                max: 10,
                actual: text.len(),
            })
        );
        assert_eq!(
            NodeDefinition::from_json_with_limits(&text, DefinitionLimits::new(text.len(), 1)),
            Err(DefinitionError::TooManyPorts { max: 1, actual: 2 })
        );
        let mut deep = json!(0);
        for _ in 0..MAX_NESTING {
            deep = json!([deep]);
        }
        let mut nested = triple();
        nested["config_schema"] = json!({ "default": deep });
        assert_eq!(
            decode(&nested),
            Err(DefinitionError::NestingLimit { max: MAX_NESTING })
        );
    }
}
