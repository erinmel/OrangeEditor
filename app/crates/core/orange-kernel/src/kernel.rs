//! Trusted host executable resolution; package loading is outside this interface.

use std::fmt::Debug;
use std::sync::Arc;

use orange_model::catalog::{ProviderKey, RegistrySnapshot};
use orange_model::document::{ConfigValue, DefinitionRef, NodeInstance, SignatureSnapshot};
use orange_model::id::{NodeId, PortKey};
use orange_model::{GraphDocument, Value};

/// Fault identifiers borrow built-in text or share package-owned text. Clones retain
/// the allocation only while an error, plan or result still needs it.
#[derive(Debug, Clone, Eq)]
pub enum FaultText {
    Static(&'static str),
    Shared(Arc<str>),
}
impl PartialEq for FaultText {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}
impl FaultText {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Static(text) => text,
            Self::Shared(text) => text,
        }
    }
    /// Conservative retained charge, including the shared allocation header.
    pub fn retained_bytes(&self) -> usize {
        match self {
            Self::Static(_) => 0,
            Self::Shared(text) => text.len().saturating_add(2 * size_of::<usize>()),
        }
    }
}
impl std::ops::Deref for FaultText {
    type Target = str;
    fn deref(&self) -> &str {
        self.as_str()
    }
}
impl From<&'static str> for FaultText {
    fn from(text: &'static str) -> Self {
        Self::Static(text)
    }
}
impl From<String> for FaultText {
    fn from(text: String) -> Self {
        Self::Shared(text.into())
    }
}
impl PartialEq<&str> for FaultText {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}
impl PartialEq<String> for FaultText {
    fn eq(&self, other: &String) -> bool {
        self.as_str() == other
    }
}
impl std::fmt::Display for FaultText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Safe redacted error envelope. No input data belongs in these fields. The compiler
/// checks bounded identifiers and retains the original producer's node as provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelFault {
    pub code: FaultText,
    pub message_key: FaultText,
    pub message: &'static str,
}
impl KernelFault {
    /// Whether this redacted envelope satisfies the shared bounded ABI.
    pub fn is_valid(&self) -> bool {
        use orange_model::value::shape::is_error_code;
        is_error_code(&self.code)
            && is_error_code(&self.message_key)
            && !self.message.is_empty()
            && self.message.len() <= 512
            && !self.message.chars().any(char::is_control)
    }
    /// Conservative per-fault charge; shared identifiers are charged per retention,
    /// so clones cannot bypass a plan or activation's independent admission budget.
    pub fn retained_bytes(&self) -> usize {
        size_of::<Self>()
            .saturating_add(self.code.retained_bytes())
            .saturating_add(self.message_key.retained_bytes())
    }
    /// Shared by Count loops and Repeat zones.
    pub const NEGATIVE_COUNT: Self = Self {
        code: FaultText::Static("core/OutOfRange"),
        message_key: FaultText::Static("core/count-out-of-range"),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ResolveError {
    #[error("no kernel is available for the node")]
    Unavailable,
    #[error("the node violates the kernel's contract")]
    Contract,
    /// The provider is available but has no executable kernel for the node's resolved
    /// types, for example a constraint proven only by a package implementation.
    #[error("the kernel does not support the node's resolved types")]
    UnsupportedType,
}

/// The exact facts one executable node binds from: its ID, definition, provider,
/// config and fresh signature, with the registry snapshot to retain. The compiler
/// builds it only for nodes admitted from fresh validation, whose provider bindings
/// passed exact package-lock checks. It exposes no other document or node data.
pub struct BindingRequest<'a> {
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
/// A resolver sees only the [`BindingRequest`]: the facts of the one node it binds, never
/// the rest of the document. Callers holding a whole document use [`ResolveInDocument`].
pub trait PureKernelResolver {
    fn resolve_binding(&self, request: &BindingRequest<'_>) -> Result<KernelBinding, ResolveError>;
}

/// Resolves one node of a whole document by building its [`BindingRequest`] first. It is
/// implemented for every resolver and cannot be overridden, so the document never reaches
/// a resolver.
pub trait ResolveInDocument {
    fn resolve(
        &self,
        document: &GraphDocument,
        node: &NodeInstance,
        signature: &SignatureSnapshot,
        snapshot: Arc<RegistrySnapshot>,
    ) -> Result<KernelBinding, ResolveError>;
}

impl<T: PureKernelResolver + ?Sized> ResolveInDocument for T {
    fn resolve(
        &self,
        document: &GraphDocument,
        node: &NodeInstance,
        signature: &SignatureSnapshot,
        snapshot: Arc<RegistrySnapshot>,
    ) -> Result<KernelBinding, ResolveError> {
        self.resolve_binding(&BindingRequest::new(document, node, signature, snapshot)?)
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
/// and seeded-entropy metadata before accepting this separate executable ABI. Like
/// [`PureKernelResolver`], it sees only the [`BindingRequest`].
pub trait EntropyKernelResolver: PureKernelResolver {
    fn resolve_entropy_binding(
        &self,
        request: &BindingRequest<'_>,
    ) -> Result<EntropyBinding, ResolveError>;
}

/// [`ResolveInDocument`] for entropy bindings.
pub trait ResolveEntropyInDocument {
    fn resolve_entropy(
        &self,
        document: &GraphDocument,
        node: &NodeInstance,
        signature: &SignatureSnapshot,
        snapshot: Arc<RegistrySnapshot>,
    ) -> Result<EntropyBinding, ResolveError>;
}

impl<T: EntropyKernelResolver + ?Sized> ResolveEntropyInDocument for T {
    fn resolve_entropy(
        &self,
        document: &GraphDocument,
        node: &NodeInstance,
        signature: &SignatureSnapshot,
        snapshot: Arc<RegistrySnapshot>,
    ) -> Result<EntropyBinding, ResolveError> {
        self.resolve_entropy_binding(&BindingRequest::new(document, node, signature, snapshot)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fault_text_equality_validation_and_lifetime_ignore_storage_kind() {
        let shared: Arc<str> = Arc::from("acme/Overflow");
        let weak = Arc::downgrade(&shared);
        let fault = KernelFault {
            code: FaultText::Shared(shared),
            message_key: String::from("acme/overflow").into(),
            message: "The operation failed.",
        };
        assert!(fault.is_valid());
        assert_eq!(fault.code, FaultText::Static("acme/Overflow"));
        assert!(fault.retained_bytes() > size_of::<KernelFault>());
        let retained = fault.clone();
        drop(fault);
        assert!(weak.upgrade().is_some());
        drop(retained);
        assert!(weak.upgrade().is_none());
    }

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
