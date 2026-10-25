//! Checked immutable data and bounded structured-control plans.
//! Pure/seeded data demand supports Select/And/Or/Coalesce/Try. Control plans
//! additionally admit the documented entries, Get/Set, branches, Sequence,
//! Count/While and virtual Wait/Yield. Functions/Fork/services remain unsupported.

mod compile;
mod control;
mod data;
mod data_compile;
mod error;
mod evaluate;
mod kernel;
mod lazy;
mod plan;

pub use compile::{CompileLimits, CompileOptions, compile_pure};
pub use control::{
    ControlIx, ControlNode, ControlOperation, ControlPlan, ControlVariable, ForkJoin, HostSource,
    compile_control,
};
pub use data::{DataInput, DataNode, DataOperation, DataPlan, DataResources, DataStability};
pub use data_compile::compile_data;
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
