//! Node contracts and resolved type arguments.

use std::collections::BTreeMap;

use thiserror::Error;

use crate::config_pointer::pointer_get;
use crate::document::{
    ConfigValue, DefinitionRef, NodeKind, PortDescriptor, Suspends, TypeParam, TypeParamName,
};
use crate::edit::DeclId;
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
    Update,
    WriteShared,
    UpdateShared,
}

/// Where a definition's ports come from.
#[derive(Debug, Clone, PartialEq)]
pub enum Ports {
    /// Fixed ports; data port types may mention `$name` parameters.
    Declared(Vec<PortDescriptor>),
    /// Variable-derived ports: Read has a `value` output, Write has a `value`
    /// input, and Update has a `value` input plus `old`/`new` outputs. Shared writes
    /// use the async shared-variable contracts.
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

/// Why a reference field does not yield a declaration ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ReferenceError {
    #[error("reference field is absent")]
    Missing,
    #[error("reference field is not a valid declaration ID")]
    Malformed,
}

impl ReferenceField {
    /// Reads the field from `config`; the only place a config string becomes a declaration ID.
    pub(crate) fn parse(&self, config: &ConfigValue) -> Result<DeclId, ReferenceError> {
        let value = pointer_get(config, &self.pointer).ok_or(ReferenceError::Missing)?;
        let text = value.as_str().ok_or(ReferenceError::Malformed)?;
        let parsed = match self.kind {
            ReferenceKind::Variable => text.parse().map(DeclId::Variable).ok(),
            ReferenceKind::Event => text.parse().map(DeclId::Event).ok(),
            ReferenceKind::Function => text.parse().map(DeclId::Function).ok(),
        };
        parsed.ok_or(ReferenceError::Malformed)
    }
}

/// Why a contract cannot be computed from a definition and a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ContractError {
    #[error("the referenced variable is missing or not declared")]
    VariableUnknown,
    #[error("synchronous variable writes cannot target workspace scope")]
    VariableScope,
    #[error("variable update operation is missing, malformed or unknown")]
    UpdateOperation,
    #[error("variable update operation is recognized but not supported by this contract")]
    UpdateUnsupported,
    #[error("shared variable writes require a workspace variable")]
    SharedVariableScope,
    #[error("variable type has no exact proof for the requested update operation")]
    UpdateConstraint,
    #[error("variable type cannot be resolved")]
    TypeMismatch,
    #[error("variable type resolution exceeded its resource limit")]
    TypeComplexity,
    #[error("converter config is malformed or uses an unsupported version")]
    ConverterConfig,
    #[error("converter entry is unavailable")]
    ConverterEntry,
    #[error("converter options are invalid or incomplete")]
    ConverterOptions,
    #[error("converter provider is unavailable")]
    ConverterProvider,
    #[error("converter package provider has no unique package lock entry")]
    ConverterPackageLockMissing,
}
