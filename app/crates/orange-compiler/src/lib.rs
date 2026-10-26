//! Checked immutable data and bounded structured-control plans.
//! Pure/seeded data demand supports Select/And/Or/Coalesce/Try. Control plans
//! additionally admit the documented entries, Get/Set, branches, Sequence,
//! Count/While, Fork and virtual Wait/Yield. PurePlan also folds bounded graph-local
//! pure calls. Dynamic/volatile/action/async calls and services remain unsupported.

mod calls;
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
    ConstantFault, Export, FoldedValue, InputSource, NodeIx, PlanType, PureCallSite, PurePlan,
    PureStep, SlotIx, SlotSource,
};
