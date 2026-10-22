//! Generic type inference: structural unification, read-only component analysis and the
//! commands that persist its results.

pub mod component;
pub mod persist;
pub mod unify;

pub use component::{
    ComponentInference, GenericDefinition, InferenceIssue, ProviderKey, StaleNodeTemplate,
    TemplateCatalog, TypeArgs, infer_components,
};
pub use persist::{InferencePlan, InferredCommit, apply_inferred, plan_edit};
pub use unify::{
    ConcreteTypeError, EquationContext, EquationEvidence, MAX_EXPANDED_TYPE_NODES, ScopedTypeTerm,
    TypeEquation, TypeScopeError, TypeVariable, Unification, UnificationError, unify,
};
