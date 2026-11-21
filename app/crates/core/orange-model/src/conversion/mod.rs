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

/// Unit choices used by the Core time converters, in nanoseconds per unit.
pub const TIME_UNITS: [(&str, i64); 6] = [
    ("seconds", 1_000_000_000),
    ("milliseconds", 1_000_000),
    ("microseconds", 1_000),
    ("nanoseconds", 1),
    ("minutes", 60_000_000_000),
    ("hours", 3_600_000_000_000),
];

/// Stable policy values. The first four are also valid Unix timestamp units.
pub const TIME_UNIT_NAMES: [&str; TIME_UNITS.len()] = {
    let mut names = [""; TIME_UNITS.len()];
    let mut index = 0;
    while index < TIME_UNITS.len() {
        names[index] = TIME_UNITS[index].0;
        index += 1;
    }
    names
};

#[cfg(test)]
mod tests;
