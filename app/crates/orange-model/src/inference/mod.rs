//! Generic type inference: structural unification and read-only component analysis.
//! Persisting inferred bindings into a document is owned by a later WU-00c slice.

pub mod component;
pub mod unify;

pub use component::{
    ComponentInference, GenericDefinition, InferenceIssue, TemplateCatalog, infer_components,
};
pub use unify::{
    ConcreteTypeError, EquationContext, EquationEvidence, MAX_EXPANDED_TYPE_NODES, ScopedTypeTerm,
    TypeEquation, TypeScopeError, TypeVariable, Unification, UnificationError, unify,
};
