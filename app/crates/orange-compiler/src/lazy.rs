//! Language-owned constant branch specialization, never package-controlled laziness.
use crate::{CompileError, KernelBinding, KernelFault, PureKernel};
use orange_model::Value;
use orange_model::catalog::intrinsic_id as ids;
use orange_model::document::{NodeInstance, PortDirection, SignatureSnapshot};
use std::sync::Arc;

#[derive(Debug, Clone, Copy)]
pub(crate) enum Lazy {
    Select,
    And,
    Or,
    Coalesce,
    Try,
}
impl Lazy {
    pub(crate) fn resolve(
        node: &NodeInstance,
        sig: &SignatureSnapshot,
    ) -> Result<Option<Self>, CompileError> {
        let kind = match node.definition.id.as_str() {
            ids::SELECT => Self::Select,
            ids::AND => Self::And,
            ids::OR => Self::Or,
            ids::COALESCE => Self::Coalesce,
            ids::TRY => Self::Try,
            _ => return Ok(None),
        };
        let bad = || CompileError::KernelContract { node: node.id };
        if node.definition.version != 1 || node.config_version != 1 || node.provider.is_some() {
            return Err(bad());
        }
        let expected: &[(&str, bool)] = match kind {
            Self::Select => &[
                ("condition", false),
                ("if_true", true),
                ("if_false", true),
                ("result", false),
            ],
            Self::And | Self::Or => &[("a", false), ("b", true), ("result", false)],
            Self::Coalesce => &[("value", false), ("fallback", true), ("result", false)],
            Self::Try => &[("value", true), ("result", false)],
        };
        if sig.ports.len() != expected.len() {
            return Err(bad());
        }
        for (index, (port, (name, lazy))) in sig.ports.iter().zip(expected).enumerate() {
            let direction = if index + 1 == expected.len() {
                PortDirection::Output
            } else {
                PortDirection::Input
            };
            if port.port_id.as_str() != *name
                || port.is_lazy() != *lazy
                || port.direction != direction
            {
                return Err(bad());
            }
        }
        if !matches!(kind, Self::Coalesce | Self::Try)
            && sig.ports[0].ty.as_ref().map(|ty| ty.as_str()) != Some("core/bool@1")
        {
            return Err(bad());
        }
        match kind {
            Self::Select if sig.ports[1..].iter().any(|p| p.ty != sig.ports[1].ty) => {
                return Err(bad());
            }
            Self::And | Self::Or if sig.ports.iter().any(|p| p.ty != sig.ports[0].ty) => {
                return Err(bad());
            }
            Self::Coalesce => {
                let inner = sig.ports[0].ty.as_ref().and_then(|ty| {
                    ty.as_str()
                        .strip_prefix("core/option@1<")
                        .and_then(|s| s.strip_suffix('>'))
                });
                if inner.is_none()
                    || inner != sig.ports[1].ty.as_ref().map(|ty| ty.as_str())
                    || sig.ports[1].ty != sig.ports[2].ty
                {
                    return Err(bad());
                }
            }
            Self::Try => {
                let inner = sig.ports[1].ty.as_ref().and_then(|ty| {
                    ty.as_str()
                        .strip_prefix("core/result@1<")
                        .and_then(|s| s.strip_suffix('>'))
                });
                if inner.is_none() || inner != sig.ports[0].ty.as_ref().map(|ty| ty.as_str()) {
                    return Err(bad());
                }
            }
            _ => {}
        }
        Ok(Some(kind))
    }
    pub(crate) fn selected(self, value: &Value) -> Option<Option<&'static str>> {
        Some(match (self, value) {
            (Self::Select, Value::Bool(true)) => Some("if_true"),
            (Self::Select, Value::Bool(false)) => Some("if_false"),
            (Self::And, Value::Bool(true)) | (Self::Or, Value::Bool(false)) => Some("b"),
            (Self::And | Self::Or, Value::Bool(_)) => None,
            (Self::Coalesce, Value::None) => Some("fallback"),
            (Self::Coalesce, _) => None,
            (Self::Try, _) => None,
            _ => return None,
        })
    }
}

/// Specialized to one immutable constant decision. This is not a general branch ABI.
#[derive(Debug)]
pub(crate) enum ProjectionMode {
    Value,
    UnwrapOption,
    Result { boxed: bool },
}
pub(crate) fn binding(
    inputs: Box<[orange_model::id::PortKey]>,
    outputs: Box<[orange_model::id::PortKey]>,
    faulted: bool,
    mode: ProjectionMode,
) -> KernelBinding {
    let index = if faulted {
        None
    } else {
        inputs.len().checked_sub(1)
    };
    let arity = inputs.len();
    KernelBinding {
        inputs,
        outputs,
        kernel: Arc::new(Projection { index, arity, mode }),
    }
}
#[derive(Debug)]
struct Projection {
    index: Option<usize>,
    arity: usize,
    mode: ProjectionMode,
}
impl PureKernel for Projection {
    fn fold(&self, inputs: &[Value], outputs: &mut [Value]) -> Result<(), KernelFault> {
        const BAD: KernelFault = KernelFault {
            code: "core/InternalFault",
            message_key: "core/compile-kernel-contract",
            message: "The constant selection inputs violate the checked contract.",
        };
        if inputs.len() != self.arity {
            return Err(BAD);
        }
        let [output] = outputs else {
            return Err(BAD);
        };
        let value = self.index.and_then(|index| inputs.get(index)).ok_or(BAD)?;
        *output = match (&self.mode, value) {
            (ProjectionMode::UnwrapOption, Value::Some(inner)) => inner.as_ref().clone(),
            (ProjectionMode::Result { boxed: true }, _) => Value::Ok(Arc::new(value.clone())),
            _ => value.clone(),
        };
        Ok(())
    }
}
