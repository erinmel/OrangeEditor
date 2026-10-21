//! Validated converter contracts and provider alternatives (11 §6, 12 §5/§8).
//!
//! A converter entry is semantic identity: its endpoint types, class, declared policy
//! choices, errors, and contract version. Providers are implementations of one entry and
//! may compete in its pair override slot. This module validates metadata only; it does not
//! select a provider for a graph edit or execute converter kernels.

mod definitions;
mod errors;
mod provider;
mod registry;

pub use definitions::*;
pub use errors::*;
pub use provider::*;
pub use registry::*;

#[cfg(test)]
mod tests;
