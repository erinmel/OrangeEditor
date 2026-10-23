//! Node contracts and resolved type arguments.

use std::collections::BTreeMap;

use crate::document::{NodeKind, PortDescriptor, Suspends, TypeParam, TypeParamName};
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
