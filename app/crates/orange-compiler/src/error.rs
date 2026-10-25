use orange_model::id::{NodeId, PortKey};
use thiserror::Error;

/// Compile failures carry only typed source IDs and safe fallbacks. No partial plan
/// is returned. A folded arithmetic error is instead retained as a constant fault.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CompileError {
    #[error("Compiler analysis belongs to another document state.")]
    StaleDocument,
    #[error("Compiler analysis belongs to another registry snapshot.")]
    StaleRegistry,
    #[error("Compiler input requires successful semantic validation.")]
    ValidationIncomplete,
    #[error("This node is unsupported by the pure export compiler.")]
    UnsupportedNode { node: NodeId },
    #[error("This port is unsupported by the pure export compiler.")]
    UnsupportedPort { node: NodeId, port: PortKey },
    #[error("The exact executable provider is unavailable.")]
    KernelUnavailable { node: NodeId },
    #[error("Resolved executable metadata violates the checked contract.")]
    KernelContract { node: NodeId },
    #[error("The input literal cannot supply its declared slot type.")]
    InvalidLiteral { node: NodeId, port: PortKey },
    #[error("Compilation exceeds the host's resource limits.")]
    ResourceLimit,
    #[error("Compilation was cancelled.")]
    Cancelled,
}

impl CompileError {
    pub const fn message_key(&self) -> &'static str {
        match self {
            Self::StaleDocument => "core/compile-stale-document",
            Self::StaleRegistry => "core/compile-stale-registry",
            Self::ValidationIncomplete => "core/compile-validation-incomplete",
            Self::UnsupportedNode { .. } => "core/compile-unsupported-node",
            Self::UnsupportedPort { .. } => "core/compile-unsupported-port",
            Self::KernelUnavailable { .. } => "core/compile-kernel-unavailable",
            Self::KernelContract { .. } => "core/compile-kernel-contract",
            Self::InvalidLiteral { .. } => "core/compile-invalid-literal",
            Self::ResourceLimit => "core/compile-resource-limit",
            Self::Cancelled => "core/compile-cancelled",
        }
    }
}
