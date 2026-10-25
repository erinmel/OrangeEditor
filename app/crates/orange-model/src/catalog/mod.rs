//! Host metadata for node definitions: contracts, provider identity and the immutable
//! registry snapshot that owns them (12 §4, §8).

mod availability;
mod call;
pub mod definition;
pub(crate) mod intrinsics;
pub(crate) mod language;
pub mod provider;
pub mod snapshot;

pub use call::{CallInterface, CallInterfaceError, call_interface};
pub(crate) use call::{body_rigid_params, is_function_marker, project_interface, rigid_params};
pub use definition::{
    ContractError, Determinism, EventAccess, NodeContract, NodeDefinition, Ports, ReferenceError,
    ReferenceField, ReferenceKind, TypeArgs, VariableAccess, Volatility,
};
pub(crate) use language::{
    implicit_port, implicit_ports, intrinsic, is_converter_special_form, is_intrinsic,
};
pub use provider::{ProviderDescriptor, ProviderKey};
pub use snapshot::{RegistryBuilder, RegistryError, RegistrySnapshot};
