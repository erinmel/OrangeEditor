use crate::{ExecTarget, ValueShape};
use orange_model::Value;
use std::sync::Arc;

/// Language-owned exact scalar keys; nominal Eq needs separate lowering.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum SwitchKey {
    Bool(bool),
    Signed(i64),
    Unsigned(u64),
    String(Arc<String>),
}
impl std::fmt::Debug for SwitchKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Bool(_) => "Bool(..)",
            Self::Signed(_) => "Signed(..)",
            Self::Unsigned(_) => "Unsigned(..)",
            Self::String(_) => "String(..)",
        })
    }
}
impl SwitchKey {
    pub fn from_value(value: &Value) -> Option<Self> {
        Some(match value {
            Value::Bool(v) => Self::Bool(*v),
            Value::I32(v) => Self::Signed(i64::from(*v)),
            Value::I64(v) => Self::Signed(*v),
            Value::U32(v) => Self::Unsigned(u64::from(*v)),
            Value::U64(v) => Self::Unsigned(*v),
            Value::Str(v) => Self::String(v.clone()),
            _ => return None,
        })
    }
}
#[derive(Debug, Clone)]
pub struct SwitchCase {
    pub(super) key: SwitchKey,
    pub(super) instance: u16,
    pub(super) target: Option<ExecTarget>,
}
impl SwitchCase {
    pub fn key(&self) -> &SwitchKey {
        &self.key
    }
    pub fn instance(&self) -> u16 {
        self.instance
    }
    pub fn target(&self) -> Option<ExecTarget> {
        self.target
    }
}
pub(super) fn case_key(ty: &ValueShape, value: &serde_json::Value) -> Option<SwitchKey> {
    Some(match ty {
        ValueShape::Bool => SwitchKey::Bool(value.as_bool()?),
        ValueShape::I32 => SwitchKey::Signed(i64::from(i32::try_from(value.as_i64()?).ok()?)),
        ValueShape::I64 => SwitchKey::Signed(value.as_i64()?),
        ValueShape::U32 => SwitchKey::Unsigned(u64::from(u32::try_from(value.as_u64()?).ok()?)),
        ValueShape::U64 => SwitchKey::Unsigned(value.as_u64()?),
        ValueShape::String => SwitchKey::String(Arc::new(value.as_str()?.to_owned())),
        _ => return None,
    })
}
