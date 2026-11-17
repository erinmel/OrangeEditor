//! JSON decoding of package node definition files (12 §4).
//!
//! A package ships one definition per file in the 12 §4 wire shape. Decoding owns shape
//! only: the importer's repeated-key, JSON-budget and nesting checks run first, then the
//! implemented field subset is read with unknown fields rejected, so a declaration this
//! core does not model yet is refused instead of silently dropped. Contract rules (versions,
//! type parameters, ports, kinds, suspension, defaults) are checked when the definition is
//! published through [`RegistryBuilder`](super::RegistryBuilder).

use std::collections::BTreeMap;

use serde::Deserialize;
use thiserror::Error;

use super::{Determinism, NodeDefinition, Ports, Volatility};
use crate::document::codec::{ImportBudget, ScanError, scan_json};
use crate::document::nesting::MAX_NESTING;
use crate::document::{DefinitionRef, NodeKind, PortDescriptor, Suspends, TypeParam};
use crate::error::{ImportResource, JsonError, Rejected};
use crate::id::{DefinitionId, PortId};
use crate::replace::Replacement;

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

    /// Like [`NodeDefinition::from_json`], and also returns the file's presentation:
    /// `label`, `description` and the category of its `editor` object.
    pub fn described(text: &str) -> Result<(Self, Presentation), DefinitionError> {
        Self::read_file(text).map(|file| (file.definition, file.presentation))
    }

    /// Everything a definition file says: the contract, its presentation and the older
    /// definitions it replaces (`replaces`, 12 §6.4).
    pub fn read_file(text: &str) -> Result<DefinitionFile, DefinitionError> {
        Self::decode(text, DefinitionLimits::default())
    }

    /// Decodes one package node definition file subject to `limits`.
    ///
    /// Reads the implemented 12 §4 subset: `definition_id`, `definition_version`,
    /// `config_version`, `kind`, `determinism`, and optionally `suspends`, `volatility`,
    /// `type_params`, `ports`, `config_schema`, `connection`, `label`, `description` and the
    /// category of `editor`. Port descriptors keep their presentation
    /// fields (`label`, `description`, ...) as extensions, as saved signatures do. Every other
    /// top-level field, including the 12 §4 fields not modeled yet (`errors`,
    /// `capabilities`, ...), is rejected. The result declares its ports; it has no
    /// reference fields and no core specialization list, since a package lists supported
    /// argument sets on its provider (11 §5.5). Bound host reads before building `text`.
    ///
    /// Errors come in this order: byte limit, decoded-JSON budget or malformed JSON, repeated
    /// key, nesting, shape, port count.
    pub fn from_json_with_limits(
        text: &str,
        limits: DefinitionLimits,
    ) -> Result<Self, DefinitionError> {
        Self::decode(text, limits).map(|file| file.definition)
    }

    fn decode(text: &str, limits: DefinitionLimits) -> Result<DefinitionFile, DefinitionError> {
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
        let mut wire: DefinitionWire = serde_json::from_str(text)?;
        if wire.ports.len() > limits.max_ports {
            return Err(DefinitionError::TooManyPorts {
                max: limits.max_ports,
                actual: wire.ports.len(),
            });
        }
        let editor = wire.editor.take().unwrap_or_default();
        let presentation = Presentation {
            label: wire.label.take(),
            description: wire.description.take(),
            category_id: editor.category_id,
            category: editor.category,
            connection_adapter: editor.connection_adapter,
            then_label: editor.then_label,
            paths: editor.paths,
            keywords: editor.keywords,
        };
        let replaces = std::mem::take(&mut wire.replaces);
        let definition: NodeDefinition = wire.into();
        let replaces = replaces
            .into_iter()
            .map(|old| Replacement {
                from: DefinitionRef {
                    id: old.definition_id,
                    version: old.definition_version,
                },
                to: definition.definition.clone(),
                ports: old.ports,
                config: old.config,
            })
            .collect();
        Ok(DefinitionFile {
            definition,
            presentation,
            replaces,
        })
    }
}

/// One decoded definition file ([`NodeDefinition::read_file`]).
#[derive(Debug, Clone, PartialEq)]
pub struct DefinitionFile {
    pub definition: NodeDefinition,
    pub presentation: Presentation,
    /// Older definitions this one takes the place of, each with its renames. Adding them
    /// to a [`Replacements`](crate::replace::Replacements) table checks them.
    pub replaces: Vec<Replacement>,
}

/// One `replaces` entry: an older definition version and what was renamed since.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplacesWire {
    definition_id: DefinitionId,
    definition_version: u32,
    #[serde(default)]
    ports: BTreeMap<PortId, PortId>,
    #[serde(default)]
    config: BTreeMap<String, String>,
}

/// Translatable text of a definition file (13 §2): a message key in the package's
/// namespace and the text to show when no catalogue has the key.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextRef {
    pub key: String,
    pub fallback: String,
}

/// What a definition file says about showing its node to people. It never affects
/// validation, compilation or execution, and it is not part of a node's identity.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Presentation {
    pub label: Option<TextRef>,
    pub description: Option<TextRef>,
    /// Stable identity of the palette category; nodes with the same one group together.
    pub category_id: Option<String>,
    pub category: Option<TextRef>,
    /// The adapter kind of the connection the node uses, such as
    /// `org.orange.audio/output`, so an editor can offer or declare a connection of that
    /// kind. The contract only says that the node names a connection; whether the
    /// session is of the right kind is still checked when the node runs.
    pub connection_adapter: Option<String>,
    /// What the node's `then` output is called when "Exit" would mislead, such as
    /// "Queued" for a send that only puts a message in a queue.
    pub then_label: Option<TextRef>,
    /// Text inputs that hold a path on this machine, so an editor can offer the
    /// system's file or folder chooser beside them.
    pub paths: Vec<PathInput>,
    /// Other words a person may search for the node by, comma separated, such as
    /// "sound, music" for Play sound; a catalogue gives them in other languages.
    pub keywords: Option<TextRef>,
}

/// A text input that holds a path, from a definition file's `editor.paths`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathInput {
    /// The input's port ID.
    pub port: String,
    /// The path names a folder instead of a file.
    #[serde(default)]
    pub folder: bool,
    /// The node writes the file, so the chooser asks where to save it.
    #[serde(default)]
    pub save: bool,
    /// File name extensions offered, without the dot; empty offers every file.
    #[serde(default)]
    pub extensions: Vec<String>,
}

/// The implemented part of a definition file's `editor` object. Other editor hints
/// are accepted and not read.
#[derive(Default, Deserialize)]
struct EditorWire {
    #[serde(default)]
    category_id: Option<String>,
    #[serde(default)]
    category: Option<TextRef>,
    #[serde(default)]
    connection_adapter: Option<String>,
    #[serde(default)]
    then_label: Option<TextRef>,
    #[serde(default)]
    paths: Vec<PathInput>,
    #[serde(default)]
    keywords: Option<TextRef>,
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
    #[serde(default)]
    label: Option<TextRef>,
    #[serde(default)]
    description: Option<TextRef>,
    #[serde(default)]
    editor: Option<EditorWire>,
    #[serde(default)]
    replaces: Vec<ReplacesWire>,
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

    #[test]
    fn a_definition_file_can_say_how_its_node_is_shown() {
        let mut file = triple();
        file["label"] = json!({"key": "acme.math/triple-label", "fallback": "Triple"});
        file["editor"] = json!({"category_id": "acme.math/category",
            "category": {"key": "acme.math/category", "fallback": "Math"}, "control": "standard",
            "connection_adapter": "acme.math/abacus",
            "then_label": {"key": "acme.math/triple-then", "fallback": "Tripled"},
            "paths": [{"port": "value", "extensions": ["txt"]}]});
        let text = file.to_string();
        let (definition, shown) = NodeDefinition::described(&text).unwrap();
        // The text is beside the contract, not part of it.
        assert_eq!(definition, NodeDefinition::from_json(&text).unwrap());
        assert_eq!(
            definition,
            NodeDefinition::from_json(&triple().to_string()).unwrap()
        );
        assert_eq!(shown.label.unwrap().fallback, "Triple");
        assert_eq!(shown.description, None);
        assert_eq!(shown.category_id.as_deref(), Some("acme.math/category"));
        assert_eq!(shown.category.unwrap().fallback, "Math");
        assert_eq!(
            shown.connection_adapter.as_deref(),
            Some("acme.math/abacus")
        );
        assert_eq!(shown.then_label.unwrap().fallback, "Tripled");
        assert_eq!(
            shown.paths,
            [PathInput {
                port: "value".to_owned(),
                folder: false,
                save: false,
                extensions: vec!["txt".to_owned()]
            }]
        );

        file["label"] = json!("Triple");
        assert!(NodeDefinition::described(&file.to_string()).is_err());
    }

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
    fn a_definition_file_names_the_older_definitions_it_replaces() {
        let mut file = triple();
        file["definition_version"] = json!(2);
        file["replaces"] = json!([
            {"definition_id": "acme.math/triple", "definition_version": 1,
             "ports": {"input": "value"}, "config": {"mode": "rounding"}},
            {"definition_id": "acme.math/thrice", "definition_version": 3}
        ]);
        let read = NodeDefinition::read_file(&file.to_string()).unwrap();
        let to = read.definition.definition.clone();
        assert_eq!(read.replaces.len(), 2);
        assert_eq!(read.replaces[0].from.id.as_str(), "acme.math/triple");
        assert_eq!(read.replaces[0].from.version, 1);
        assert_eq!(read.replaces[0].to, to);
        assert_eq!(
            read.replaces[0].ports[&"input".parse().unwrap()].as_str(),
            "value"
        );
        assert_eq!(read.replaces[0].config["mode"], "rounding");
        assert!(read.replaces[1].ports.is_empty() && read.replaces[1].config.is_empty());
        // Replacements are not part of the contract.
        assert_eq!(
            read.definition,
            NodeDefinition::from_json(&{
                let mut plain = triple();
                plain["definition_version"] = json!(2);
                plain.to_string()
            })
            .unwrap()
        );

        file["replaces"][0]["renamed"] = json!({});
        assert!(NodeDefinition::read_file(&file.to_string()).is_err());
        file["replaces"] = json!([{"definition_id": "acme.math/triple",
            "definition_version": 1, "ports": {"Input": "value"}}]);
        assert!(
            NodeDefinition::read_file(&file.to_string()).is_err(),
            "port IDs are checked"
        );
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
