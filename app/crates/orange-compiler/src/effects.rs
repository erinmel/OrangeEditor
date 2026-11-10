//! Plan metadata for trusted effect bindings. The binding interface itself is in
//! `orange-kernel`.
use orange_kernel::EffectBinding;
use orange_model::value::shape::ValueShape;

/// Immutable, typed executable metadata retained with the plan's registry snapshot.
#[derive(Debug, Clone)]
pub struct ControlEffect {
    pub(crate) binding: EffectBinding,
    pub(crate) outputs: Box<[ValueShape]>,
    pub(crate) shared: Option<SharedWrite>,
}
/// Closed built-in workspace command; never routed to a package executor.
#[derive(Debug, Clone, Copy)]
pub struct SharedWrite {
    pub variable: orange_model::id::VariableId,
    /// None is Set; Some is one of the checked native Update operations.
    pub operation: Option<crate::UpdateOperation>,
}
impl ControlEffect {
    pub fn shared(&self) -> Option<&SharedWrite> {
        self.shared.as_ref()
    }
    pub fn binding(&self) -> &EffectBinding {
        &self.binding
    }
    pub fn outputs(&self) -> &[ValueShape] {
        &self.outputs
    }
}
