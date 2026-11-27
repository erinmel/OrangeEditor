//! Shipped native kernels under the bundled-code exception (ADR-03/15).
//!
//! The host registers [`MathPackage`] metadata, validates graph inputs, then resolves
//! exact provider/type/config bindings once. [`ResolvedKernel::evaluate`] performs no
//! registry lookup, type-name parsing or I/O; [`ResolvedComparison::evaluate`] likewise
//! walks only its bound shape. This is a kernel surface, not a graph compiler, run
//! actor, package installer or sandbox.
// Product code reports failures as typed errors; each remaining panic is a documented
// internal invariant (10, "Panic policy").
#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]

// Keep each operation and its stable definition ID in a single table. Its order
// is also the public ALL order used when registering the catalogue.
macro_rules! operation_catalog {
    ($(#[$attr:meta])* pub enum $name:ident { $($variant:ident => $id:literal),+ $(,)? }) => {
        $(#[$attr])*
        pub enum $name {
            $($variant),+
        }

        impl $name {
            pub const ALL: [Self; [$($id),+].len()] = [$(Self::$variant),+];

            pub const fn definition_id(self) -> &'static str {
                match self {
                    $(Self::$variant => $id),+
                }
            }

            pub fn from_definition_id(id: &str) -> Option<Self> {
                Self::ALL.into_iter().find(|op| op.definition_id() == id)
            }
        }
    };
}

mod catalog;
mod collections;
mod comparison;
mod compiler;
mod conversion;
mod entropy;
mod error;
mod float;
mod integer;
mod json;
mod kernel;
mod nominal;
mod presentation_values;
mod text;
mod text_conversion;

pub use catalog::{MathPackage, math_definitions};
pub use collections::{
    CollectionOperation, MAX_COLLECTION_BYTES, ResolvedCollection, collection_definitions,
};
pub use comparison::{Comparison, ResolvedComparison, comparison_definitions};
pub use conversion::{ConversionOutput, ResolvedConversion, resolve_core_conversion};
pub use entropy::{RANDOM_U64_DEFINITION, ResolvedEntropyKernel, random_u64_definition};
pub use error::{BindingError, CatalogError, ConversionError, KernelError};
pub use float::{FloatOperation, ResolvedFloat, float_definitions};
pub use integer::{IntegerOperation, ResolvedInteger, integer_definitions};
pub use json::{JsonOperation, ResolvedJson, json_definitions};
pub use kernel::{Operation, ResolvedKernel};
pub use nominal::{NominalOperation, ResolvedNominal, nominal_definitions};
pub use text::{MAX_TEXT_BYTES, ResolvedText, TextOperation, TextUnit, text_definitions};
