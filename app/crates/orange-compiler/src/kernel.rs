//! Trusted host executable resolution; package loading is outside this interface.

use std::fmt::Debug;
use std::sync::Arc;

use orange_model::catalog::RegistrySnapshot;
use orange_model::document::{NodeInstance, SignatureSnapshot};
use orange_model::id::PortKey;
use orange_model::{GraphDocument, Value};

/// Safe static error envelope. No input data belongs in these fields. The compiler
/// checks bounded identifiers and retains the original producer's node as provenance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KernelFault {
    pub code: &'static str,
    pub message_key: &'static str,
    pub message: &'static str,
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
#[derive(Debug, Clone)]
pub struct KernelBinding {
    pub inputs: Box<[PortKey]>,
    pub outputs: Box<[PortKey]>,
    pub kernel: Arc<dyn PureKernel>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveError {
    Unavailable,
    Contract,
}

/// Resolve exact providers/types/config once. Implementations must not invoke guest
/// code or effects here. This trait is open to downstream trusted host implementations.
pub trait PureKernelResolver {
    fn resolve(
        &self,
        document: &GraphDocument,
        node: &NodeInstance,
        signature: &SignatureSnapshot,
        snapshot: Arc<RegistrySnapshot>,
    ) -> Result<KernelBinding, ResolveError>;
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

#[derive(Debug, Clone)]
pub struct EntropyBinding {
    pub inputs: Box<[PortKey]>,
    pub outputs: Box<[PortKey]>,
    pub kernel: Arc<dyn EntropyKernel>,
}

/// Binding only, not execution approval. Lowering must check fresh semantic evidence
/// and seeded-entropy metadata before accepting this separate executable ABI.
pub trait EntropyKernelResolver: PureKernelResolver {
    fn resolve_entropy(
        &self,
        document: &GraphDocument,
        node: &NodeInstance,
        signature: &SignatureSnapshot,
        snapshot: Arc<RegistrySnapshot>,
    ) -> Result<EntropyBinding, ResolveError>;
}
