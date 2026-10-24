//! Checked immutable plans for root-body pure scalar/JSON exports (E1a).
//! Control, volatile, lazy, function and scheduled execution remain separate slices.

mod compile;
mod error;
mod kernel;
mod plan;

pub use compile::{CompileLimits, CompileOptions, compile_pure};
pub use error::CompileError;
pub use kernel::{KernelBinding, KernelFault, PureKernel, PureKernelResolver, ResolveError};
pub use plan::{
    ConstantFault, Export, FoldedValue, InputSource, NodeIx, PlanType, PurePlan, PureStep, SlotIx,
    SlotSource,
};
