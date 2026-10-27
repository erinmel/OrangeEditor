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
pub trait EntropyKernelResolver: PureKernelResolver {
    fn resolve_entropy(
        &self,
        document: &GraphDocument,
        node: &NodeInstance,
        signature: &SignatureSnapshot,
        snapshot: Arc<RegistrySnapshot>,
    ) -> Result<EntropyBinding, ResolveError>;
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
