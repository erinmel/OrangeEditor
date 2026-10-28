//! Generic type inference: structural unification, read-only component analysis and the
//! commands that persist its results.

pub mod component;
pub mod persist;
pub mod unify;

pub use component::{ComponentInference, InferenceIssue, infer_components};
pub use persist::{
    InferencePlan, InferredCommit, apply_inferred, apply_inferred_indexed, apply_plan, plan_edit,
    plan_edit_indexed,
};
pub use unify::{
    ConcreteTypeError, EquationContext, EquationEvidence, MAX_EXPANDED_TYPE_NODES, ScopedTypeTerm,
    TypeEquation, TypeScopeError, TypeVariable, Unification, UnificationError, unify,
};
