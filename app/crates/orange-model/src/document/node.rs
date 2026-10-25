//! Node instances, provider bindings and entry activation policies.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::keyed::Keyed;
use super::literal::Literal;
use super::{ConfigValue, Extensions, TypeParamName};
use crate::id::{DefinitionId, NodeId, PortId, PortKey, SignatureKey, TypeExpr};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeInstance {
    pub id: NodeId,
    #[serde(flatten)]
    pub definition: DefinitionRef,
    pub config_version: u32,
    #[serde(default)]
    pub config: ConfigValue,
    /// Persisted inference result.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub type_args: BTreeMap<TypeParamName, TypeExpr>,
    /// Explicit user type choices. Missing on older documents means no pins; `type_args`
    /// remains an inferred cache and is never migrated into this map.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub type_pins: BTreeMap<TypeParamName, TypeExpr>,
    /// `None` only for `core/` intrinsics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderBinding>,
    /// Required for every non-intrinsic node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<SignatureKey>,
    /// Includes literals shadowed by a connected edge.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub inputs: BTreeMap<PortKey, Literal>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub variadic: BTreeMap<PortId, VariadicState>,
    /// Entries only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activation: Option<ActivationPolicy>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

impl Keyed for NodeInstance {
    type Key = NodeId;

    fn key(&self) -> NodeId {
        self.id
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DefinitionRef {
    #[serde(rename = "definition_id")]
    pub id: DefinitionId,
    #[serde(rename = "definition_version")]
    pub version: u32,
}

/// Exact provider selected for a node; changed only by an explicit rebind (ADR-07).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderBinding {
    pub package: String,
    pub version: String,
    pub id: String,
    #[serde(flatten)]
    pub extensions: Extensions,
}

/// Variadic instance order plus the next never-reused counter (11 §4.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VariadicState {
    pub order: Vec<u16>,
    pub next: u16,
}

impl VariadicState {
    /// Active counters must be unique and strictly below the never-reused allocator.
    /// Malformed imports remain editable, but cannot be executed or allocated from.
    pub fn is_valid(&self) -> bool {
        let mut seen = std::collections::BTreeSet::new();
        self.order
            .iter()
            .all(|&key| key < self.next && seen.insert(key))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActivationPolicy {
    pub mode: ReentrancyMode,
    /// Required by `parallel`; checked by the validator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_parallel: Option<u32>,
    pub capacity: u32,
    pub overflow: OverflowPolicy,
    pub on_fault: FaultPolicy,
    pub order: u32,
    #[serde(flatten)]
    pub extensions: Extensions,
}

/// 11 §10.1. Packages cannot add modes (11 §16), so the set is closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReentrancyMode {
    Queue,
    Drop,
    Replace,
    Parallel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OverflowPolicy {
    Backpressure,
    DropOldest,
    DropNewest,
    CoalesceLatest,
    Fault,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FaultPolicy {
    Continue,
    StopRun,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::NodeInstance;

    #[test]
    fn legacy_node_without_type_pins_loads_with_no_pins() {
        let mut value = json!({
            "id": "10000000-0000-4000-8000-000000000001",
            "definition_id": "org.example/math",
            "definition_version": 1,
            "config_version": 1,
            "config": {},
            "type_args": {"T": "core/i64@1"}
        });
        let node: NodeInstance = serde_json::from_value(value.clone()).unwrap();
        assert!(node.type_pins.is_empty());
        assert_eq!(node.type_args["T"].as_str(), "core/i64@1");

        value["type_pins"] = json!({"T": "core/f64@1"});
        let pinned: NodeInstance = serde_json::from_value(value).unwrap();
        assert_eq!(pinned.type_pins["T"].as_str(), "core/f64@1");
        let encoded = serde_json::to_value(pinned).unwrap();
        assert_eq!(encoded["type_pins"]["T"], "core/f64@1");
    }
}
