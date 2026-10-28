//! Trusted host executable resolution; package loading is outside this interface.

use std::fmt::Debug;
use std::sync::Arc;

use orange_model::catalog::{ProviderKey, RegistrySnapshot};
use orange_model::document::{ConfigValue, DefinitionRef, NodeInstance, SignatureSnapshot};
use orange_model::id::{NodeId, PortKey};
use orange_model::{GraphDocument, Value};

/// Safe static error envelope. No input data belongs in these fields. The compiler
/// checks bounded identifiers and retains the original producer's node as provenance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KernelFault {
    pub code: &'static str,
    pub message_key: &'static str,
    pub message: &'static str,
}
impl KernelFault {
    /// Shared by Count loops and Repeat zones.
    pub const NEGATIVE_COUNT: Self = Self {
        code: "core/OutOfRange",
        message_key: "core/count-out-of-range",
        message: "Loop count must be nonnegative.",
    };
}

/// Implementors are trusted host code: bounded, deterministic and free of effects.
/// `fold` must fill every output with its declared type. Cancellation cannot interrupt
/// a callback; untrusted/heavy implementations require a separately isolated backend.
pub trait PureKernel: Debug + Send + Sync {
    fn fold(&self, inputs: &[Value], outputs: &mut [Value]) -> Result<(), KernelFault>;
}

/// Ordinary executable port order must match the full fresh signature. Compiler-owned
/// constant lazy steps retain only the demanded port projection, specialized to their
/// immutable selector. Such a binding is not a general runtime branch instruction.
/// Debug reports port counts and never invokes the downstream kernel formatter.
#[derive(Clone)]
pub struct KernelBinding {
    pub inputs: Box<[PortKey]>,
    pub outputs: Box<[PortKey]>,
    pub kernel: Arc<dyn PureKernel>,
}
impl Debug for KernelBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KernelBinding")
            .field("inputs", &self.inputs.len())
            .field("outputs", &self.outputs.len())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveError {
    Unavailable,
    Contract,
}

/// The exact facts one executable node binds from: its ID, definition, provider,
/// config and fresh signature, with the registry snapshot to retain. The compiler
/// builds it only for nodes admitted from fresh validation, whose provider bindings
/// passed exact package-lock checks. It exposes no other document or node data.
pub struct BindingRequest<'a> {
    document: &'a GraphDocument,
    node: &'a NodeInstance,
    provider: Option<ProviderKey>,
    signature: &'a SignatureSnapshot,
    snapshot: Arc<RegistrySnapshot>,
}
impl<'a> BindingRequest<'a> {
    /// Extracts the node's exact provider from the document lock. A bound provider
    /// without exactly one matching lock entry is [`ResolveError::Unavailable`].
    pub fn new(
        document: &'a GraphDocument,
        node: &'a NodeInstance,
        signature: &'a SignatureSnapshot,
        snapshot: Arc<RegistrySnapshot>,
    ) -> Result<Self, ResolveError> {
        let provider = match &node.provider {
            None => None,
            Some(binding) => {
                Some(ProviderKey::of(document, binding).ok_or(ResolveError::Unavailable)?)
            }
        };
        Ok(Self {
            document,
            node,
            provider,
            signature,
            snapshot,
        })
    }
    pub fn node(&self) -> NodeId {
        self.node.id
    }
    pub fn definition(&self) -> &'a DefinitionRef {
        &self.node.definition
    }
    /// The exact locked provider, or `None` for a node without a provider binding,
    /// such as a Core intrinsic.
    pub fn provider(&self) -> Option<&ProviderKey> {
        self.provider.as_ref()
    }
    pub fn config_version(&self) -> u32 {
        self.node.config_version
    }
    pub fn config(&self) -> &'a ConfigValue {
        &self.node.config
    }
    pub fn signature(&self) -> &'a SignatureSnapshot {
        self.signature
    }
    pub fn snapshot(&self) -> &Arc<RegistrySnapshot> {
        &self.snapshot
    }
}
/// Debug reports identities only, never config values or the document.
impl Debug for BindingRequest<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BindingRequest")
            .field("node", &self.node.id)
            .field("definition", &self.node.definition)
            .field("provider", &self.provider)
            .field("config_version", &self.node.config_version)
            .finish_non_exhaustive()
    }
}

/// Resolve exact providers/types/config once. Implementations must not invoke guest
/// code or effects here. This trait is open to downstream trusted host implementations.
///
/// The compiler binds through [`resolve_binding`](Self::resolve_binding). Its default
/// forwards to the original whole-document [`resolve`](Self::resolve), so existing
/// implementors keep working. New implementors override `resolve_binding` and may
/// implement `resolve` as `self.resolve_binding(&BindingRequest::new(..)?)`.
pub trait PureKernelResolver {
    fn resolve(
        &self,
        document: &GraphDocument,
        node: &NodeInstance,
        signature: &SignatureSnapshot,
        snapshot: Arc<RegistrySnapshot>,
    ) -> Result<KernelBinding, ResolveError>;

    fn resolve_binding(&self, request: &BindingRequest<'_>) -> Result<KernelBinding, ResolveError> {
        self.resolve(
            request.document,
            request.node,
            request.signature,
            request.snapshot.clone(),
        )
    }
}

/// Trusted host-owned deterministic stream for one resolved node invocation. This
/// interface exposes no document, scheduler, OS source or other node stream.
pub trait EntropySource {
    fn next_u64(&mut self) -> Result<u64, KernelFault>;
}

/// Trusted effect-free entropy operation. Unlike PureKernel it is never constant-folded;
/// the caller supplies the correct seed/node/activation/invocation scope.
pub trait EntropyKernel: Debug + Send + Sync {
    fn evaluate(
        &self,
        inputs: &[Value],
        outputs: &mut [Value],
        entropy: &mut dyn EntropySource,
    ) -> Result<(), KernelFault>;
}

/// Checked by the compiler before publication. Debug reports port counts and does
/// not invoke the downstream kernel formatter.
#[derive(Clone)]
pub struct EntropyBinding {
    pub inputs: Box<[PortKey]>,
    pub outputs: Box<[PortKey]>,
    pub kernel: Arc<dyn EntropyKernel>,
}
impl Debug for EntropyBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EntropyBinding")
            .field("inputs", &self.inputs.len())
            .field("outputs", &self.outputs.len())
            .finish_non_exhaustive()
    }
}

/// Binding only, not execution approval. Lowering must check fresh semantic evidence
/// and seeded-entropy metadata before accepting this separate executable ABI.
/// The compiler binds through `resolve_entropy_binding`, whose default forwards to
/// `resolve_entropy` exactly as [`PureKernelResolver::resolve_binding`] does.
pub trait EntropyKernelResolver: PureKernelResolver {
    fn resolve_entropy(
        &self,
        document: &GraphDocument,
        node: &NodeInstance,
        signature: &SignatureSnapshot,
        snapshot: Arc<RegistrySnapshot>,
    ) -> Result<EntropyBinding, ResolveError>;

    fn resolve_entropy_binding(
        &self,
        request: &BindingRequest<'_>,
    ) -> Result<EntropyBinding, ResolveError> {
        self.resolve_entropy(
            request.document,
            request.node,
            request.signature,
            request.snapshot.clone(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DebugTrap;
    impl Debug for DebugTrap {
        fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            panic!("binding Debug must not invoke downstream formatting")
        }
    }
    impl PureKernel for DebugTrap {
        fn fold(&self, _: &[Value], _: &mut [Value]) -> Result<(), KernelFault> {
            panic!("formatting must not execute a callback")
        }
    }
    impl EntropyKernel for DebugTrap {
        fn evaluate(
            &self,
            _: &[Value],
            _: &mut [Value],
            _: &mut dyn EntropySource,
        ) -> Result<(), KernelFault> {
            panic!("formatting must not execute a callback")
        }
    }
    #[test]
    fn binding_debug_does_not_call_downstream_formatters() {
        let pure = KernelBinding {
            inputs: Box::new([]),
            outputs: Box::new([]),
            kernel: Arc::new(DebugTrap),
        };
        let entropy = EntropyBinding {
            inputs: Box::new([]),
            outputs: Box::new([]),
            kernel: Arc::new(DebugTrap),
        };
        assert!(format!("{pure:#?}").contains("KernelBinding"));
        assert!(format!("{entropy:#?}").contains("EntropyBinding"));
    }
}
