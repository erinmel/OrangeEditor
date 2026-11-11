//! Trusted effect bindings. Resolution is inert; only a control token invokes an action.
use crate::kernel::{BindingRequest, KernelFault, ResolveError};
use orange_model::Value;
use orange_model::id::PortKey;
use std::sync::Arc;

/// Trusted, bounded, nonblocking synchronous action. Never used by constant folding.
/// Heavy or potentially blocking work must use an asynchronous command instead.
pub trait ActionKernel: std::fmt::Debug + Send + Sync {
    fn execute(&self, inputs: &[Value], outputs: &mut [Value]) -> Result<(), KernelFault>;
}

/// Exact fresh data-port order and execution mode. `None` submits an asynchronous
/// command to the embedding host; `Some` is legal only for an Action definition.
#[derive(Clone)]
pub struct EffectBinding {
    /// Host-private routing key chosen during exact-provider binding.
    pub route: u64,
    /// Immutable host-owned executable generation/config/capability context.
    /// Retain the exact bound implementation here; never retarget it on updates.
    /// The numeric route is only a dispatch hint, not a replaceable global lookup.
    pub context: Arc<dyn std::any::Any + Send + Sync>,
    pub inputs: Box<[PortKey]>,
    pub outputs: Box<[PortKey]>,
    pub action: Option<Arc<dyn ActionKernel>>,
}
impl std::fmt::Debug for EffectBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EffectBinding")
            .field("inputs", &self.inputs.len())
            .field("outputs", &self.outputs.len())
            .field("synchronous", &self.action.is_some())
            .finish()
    }
}
/// Resolve only this node's exact locked provider/config/signature. No effects,
/// acquisition, executor work or guest calls are allowed during resolution.
pub trait EffectResolver {
    fn resolve_effect(&self, request: &BindingRequest<'_>) -> Result<EffectBinding, ResolveError>;
}
