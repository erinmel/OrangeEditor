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

mod catalog;
mod comparison;
mod compiler;
mod conversion;
mod entropy;
mod error;
mod float;
mod integer;
mod kernel;

pub use catalog::{MathPackage, math_definitions};
pub use comparison::{Comparison, ResolvedComparison, comparison_definitions};
pub use conversion::{ConversionOutput, ResolvedConversion, resolve_core_conversion};
pub use entropy::{RANDOM_U64_DEFINITION, ResolvedEntropyKernel, random_u64_definition};
pub use error::{BindingError, CatalogError, ConversionError, KernelError};
pub use float::{FloatOperation, ResolvedFloat, float_definitions};
pub use integer::{IntegerOperation, ResolvedInteger, integer_definitions};
pub use kernel::{Operation, ResolvedKernel};
