//! Structural generic inference primitives. Graph traversal, registry validation and
//! document mutation are owned by later WU-00c slices.

pub mod unify;

pub use unify::{
    ConcreteTypeError, EquationContext, EquationEvidence, MAX_EXPANDED_TYPE_NODES, ScopedTypeTerm,
    TypeEquation, TypeScopeError, TypeVariable, Unification, UnificationError, unify,
};
