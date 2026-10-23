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

/// Identity namespace of a config field that holds a graph declaration ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReferenceKind {
    Variable,
    Event,
    Function,
}

/// A config field whose string value is a typed graph identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceField {
    /// RFC 6901 JSON pointer within the node config, such as `/variable`.
    pub pointer: String,
    pub kind: ReferenceKind,
}

/// Whether a variable node reads or writes its declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VariableAccess {
    Read,
    Write,
}

/// Where a definition's ports come from.
#[derive(Debug, Clone, PartialEq)]
pub enum Ports {
    /// Fixed ports; data port types may mention `$name` parameters.
    Declared(Vec<PortDescriptor>),
    /// One `value` port typed by the referenced variable declaration.
    Variable(VariableAccess),
}

/// One registered node definition version.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeDefinition {
    pub definition: DefinitionRef,
    pub config_version: u32,
    /// Kind of the projected signature snapshots of this definition's instances.
    pub kind: NodeKind,
    /// Declared suspension behavior. `None` is unresolved for async/iterator definitions.
    pub suspends: Option<Suspends>,
    pub type_params: Vec<TypeParam>,
    pub ports: Ports,
    /// Config fields holding declaration IDs; the only fields treated as references.
    pub references: Vec<ReferenceField>,
    /// Core-implemented argument sets. `None`: constraints decide.
    pub specializations: Option<Vec<TypeArgs>>,
    /// Stored for hosts; not enforced by the model.
    pub config_schema: Option<serde_json::Value>,
}

impl NodeDefinition {
    /// The contract of a definition with fixed ports.
    pub(crate) fn declared_contract(&self) -> Option<NodeContract> {
        match &self.ports {
            Ports::Declared(ports) => Some(NodeContract {
                kind: self.kind,
                suspends: self.suspends,
                type_params: self.type_params.clone(),
                ports: ports.clone(),
            }),
            Ports::Variable(_) => None,
        }
    }
}
