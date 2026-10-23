//! Node contracts and resolved type arguments.

use std::collections::BTreeMap;

use crate::document::{
    DefinitionRef, NodeKind, PortDescriptor, Suspends, TypeParam, TypeParamName,
};
use crate::id::TypeExpr;

/// Host-supplied contract of one definition version: its declared ports before type
/// substitution. Persisted signature snapshots are resolved results, never contracts.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeContract {
    /// Kind of the projected signature snapshots of this definition's instances.
    pub kind: NodeKind,
    /// Declared suspension behavior. `None` is unresolved for async/iterator definitions.
    pub suspends: Option<Suspends>,
    pub type_params: Vec<TypeParam>,
    /// Declared ports; data port types may mention `$name` parameters.
    pub ports: Vec<PortDescriptor>,
}

/// Resolved type arguments of one generic node, by parameter name.
pub type TypeArgs = BTreeMap<TypeParamName, TypeExpr>;

/// One registered node definition version.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeDefinition {
    pub definition: DefinitionRef,
    pub config_version: u32,
    /// Declared ports only; config-derived ports stay contextual.
    pub contract: NodeContract,
    /// Core-implemented argument sets. `None`: constraints decide.
    pub specializations: Option<Vec<TypeArgs>>,
    /// Stored for hosts; not enforced by the model.
    pub config_schema: Option<serde_json::Value>,
}
