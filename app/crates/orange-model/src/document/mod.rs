//! Graph document structures (12 §3.1), serialized exactly as format v1 (12 §6).
//!
//! The codec checks structure only; whether nodes, ports and types make sense is the
//! validator's job (00d), so unknown definitions and orphan edges load fine (12 I-4).
//! Every extensible object keeps unknown keys in `extensions` so that a newer writer's
//! fields survive load→save (FR-05).
//!
//! State owned by transactions (revision, bodies, declarations, editor layout) is private
//! and changes only through [`GraphDocument::apply`]. Locks and exports have no document
//! command yet (12 §3.3) and stay public until their flows exist.
//!
//! `graph` owns storage and lookup, `node` owns instances and activation policies,
//! `port` owns connections/signatures, and `declaration` owns variables/events/functions.
//! Their types are re-exported here so callers keep one stable document API.

pub mod codec;
pub mod keyed;
pub mod literal;

mod declaration;
mod graph;
mod intrinsics;
mod node;
mod port;

pub use declaration::{
    ConflictPolicy, EventDecl, EventScope, FunctionDecl, TypeParam, VarScope, VariableDecl,
};
pub use graph::{BodyRef, GraphBody, GraphDocument, GraphExport, PackageLockEntry, TypeLockEntry};
pub use node::{
    ActivationPolicy, DefinitionRef, FaultPolicy, NodeInstance, OverflowPolicy, ProviderBinding,
    ReentrancyMode, VariadicState,
};
pub use port::{
    Edge, NodeKind, OutputSource, PortDescriptor, PortDirection, PortKind, PortRef, Sensitivity,
    SignatureSnapshot, Suspends,
};

pub(crate) use graph::fresh_state_id;
pub(crate) use intrinsics::is_intrinsic_definition;

use serde_json::{Map, Value as Json};

/// Unknown keys of an object, preserved in their original order.
pub type Extensions = Map<String, Json>;
/// Node configuration, validated later against the definition's `config_schema`.
pub type ConfigValue = Map<String, Json>;
/// Name of a definition's type parameter, such as `T`.
pub type TypeParamName = String;
