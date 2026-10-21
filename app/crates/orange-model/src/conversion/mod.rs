//! Validated converter contracts and provider alternatives (11 §6, 12 §5/§8).
//!
//! A converter entry is semantic identity: its endpoint types, class, declared policy
//! choices, errors, and contract version. Providers are implementations of one entry and
//! may compete in its pair override slot. This module validates metadata only; it does not
//! select a provider for a graph edit or execute converter kernels. `plan` turns a selection
//! into document commands.

mod definitions;
mod errors;
mod plan;
mod provider;
mod registry;
mod selection;

pub use definitions::*;
pub use errors::*;
pub use plan::*;
pub use provider::*;
pub use registry::*;
pub use selection::*;

#[cfg(test)]
mod tests;
