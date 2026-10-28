//! Checked immutable data and bounded structured-control plans.
//!
//! Current profile: pure/seeded data demand supports Select/And/Or/Coalesce/Try, the
//! Repeat, Fold, Map and Filter iteration zones, and bounded graph-local Pure calls; data
//! plans also support seeded volatile calls. Control plans additionally admit the
//! documented entries, Get/Set, branches, Sequence, Count/While/ForEach, Fork, scalar
//! Switch, Match-result, Gate/DoOnce/FlipFlop/Register, Core variable updates, virtual
//! Wait/Yield and graph-local Action/Async calls. Events, package-provided function bodies,
//! clock readers and host services remain unsupported. The README tracks this profile.

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
mod zones;

pub use compile::{CompileLimits, CompileOptions, compile_pure};
pub use control::{
    ControlCell, ControlIx, ControlNode, ControlOperation, ControlPlan, ControlState,
    ControlVariable, ExecInput, ExecTarget, ForkJoin, FunctionLayout, HostSource,
    IterationProjection, SwitchCase, SwitchKey, UpdateDemand, UpdateOperation, UpdateProjection,
    compile_control,
};
pub use data::{DataInput, DataNode, DataOperation, DataPlan, DataResources, DataStability};
pub use data_compile::compile_data;
pub use error::CompileError;
pub use evaluate::{EvaluatedExport, EvaluationError, EvaluationOptions, PureEvaluation};
pub use kernel::{
    BindingRequest, EntropyBinding, EntropyKernel, EntropyKernelResolver, EntropySource,
    KernelBinding, KernelFault, PureKernel, PureKernelResolver, ResolveEntropyInDocument,
    ResolveError, ResolveInDocument,
};
pub use plan::{
    ConstantFault, Export, FoldedValue, InputSource, NodeIx, PureCallSite, PurePlan, PureStep,
    SlotIx, SlotSource,
};
pub use zones::{DataZone, ZoneIx};

pub(crate) use orange_model::value::shape::{ValueCheckError, ValueShape};
