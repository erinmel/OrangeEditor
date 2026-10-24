//! Checked immutable plans for root-body pure scalar/JSON/option/result exports.
//! Constant Select/And/Or/Coalesce/Try demands are supported. Control, volatile,
//! function and scheduled execution remain separate slices.

mod compile;
mod error;
mod evaluate;
mod kernel;
mod lazy;
mod plan;

pub use compile::{CompileLimits, CompileOptions, compile_pure};
pub use error::CompileError;
pub use evaluate::{EvaluatedExport, EvaluationError, EvaluationOptions, PureEvaluation};
pub use kernel::{
    EntropyBinding, EntropyKernel, EntropyKernelResolver, EntropySource, KernelBinding,
    KernelFault, PureKernel, PureKernelResolver, ResolveError,
};
pub use plan::{
    ConstantFault, Export, FoldedValue, InputSource, NodeIx, PlanType, PurePlan, PureStep, SlotIx,
    SlotSource,
};
