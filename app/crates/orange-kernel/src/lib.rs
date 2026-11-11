//! The interface between a compiled graph and the native code that implements its nodes.
//!
//! A provider implements these traits: a pure kernel folds values, an entropy kernel
//! draws from a host stream, an action kernel runs inline and an effect binding hands
//! asynchronous work to the host. Resolvers bind one node's exact locked provider.
//!
//! This crate depends only on the graph model, so a provider or a package loader
//! needs neither the compiler nor the runtime.

mod effects;
mod kernel;

pub use effects::{ActionKernel, EffectBinding, EffectResolver};
pub use kernel::{
    BindingRequest, EntropyBinding, EntropyKernel, EntropyKernelResolver, EntropySource, FaultText,
    KernelBinding, KernelFault, PureKernel, PureKernelResolver, ResolveEntropyInDocument,
    ResolveError, ResolveInDocument,
};
