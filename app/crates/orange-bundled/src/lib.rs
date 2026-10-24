//! Shipped native kernels under the bundled-code exception (ADR-03/15).
//!
//! The host registers [`MathPackage`] metadata, validates graph inputs, then resolves
//! exact provider/type/config bindings once. [`ResolvedKernel::evaluate`] performs no
//! registry lookup, type-name parsing or I/O. This is a scalar kernel surface, not a
//! graph compiler, run actor, package installer or sandbox.

mod catalog;
mod conversion;
mod error;
mod kernel;

pub use catalog::{MathPackage, math_definitions};
pub use conversion::{ConversionOutput, ResolvedConversion, resolve_core_conversion};
pub use error::{BindingError, CatalogError, ConversionError, KernelError};
pub use kernel::{Operation, ResolvedKernel};
