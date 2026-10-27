//! Owned, bounded zone accumulators. Scheduling and memoization stay in data::Frame.
//!
//! A Fold/Map/Filter source list stays retained by its memoized outer slot for the
//! machine's whole life, so the machine charges only its shared handle (part of
//! its own size), never the list again.

use orange_compiler::{KernelFault, PlanType, ValueCheckError};
use orange_model::Value;
use orange_model::validation::ZoneKind;
use orange_model::value::ListRepr;
use std::sync::Arc;

/// Begin inputs collected before the first iteration: Repeat count/initial,
/// Fold items/initial, Map/Filter items.
pub(crate) const MAX_PREFIX: usize = 2;
/// Fixed charge of a list builder before its first element.
const BUILDER_CHARGE: usize = 16;

pub(crate) enum MachineError {
    Contract,
    Limit,
    Fault(KernelFault),
}

impl From<ValueCheckError> for MachineError {
    fn from(error: ValueCheckError) -> Self {
        match error {
            ValueCheckError::InvalidType => Self::Contract,
            ValueCheckError::ResourceLimit => Self::Limit,
        }
    }
}

enum ListBuilder {
    Bool(Vec<bool>),
    I32(Vec<i32>),
    I64(Vec<i64>),
    U32(Vec<u32>),
    U64(Vec<u64>),
    F32(Vec<f32>),
    F64(Vec<f64>),
    Str(Vec<Arc<String>>),
    Values(Vec<Value>),
}

impl ListBuilder {
    fn new(ty: &PlanType) -> Self {
        match ty {
            PlanType::Bool => Self::Bool(Vec::new()),
            PlanType::I32 => Self::I32(Vec::new()),
            PlanType::I64 => Self::I64(Vec::new()),
            PlanType::U32 => Self::U32(Vec::new()),
            PlanType::U64 => Self::U64(Vec::new()),
            PlanType::F32 => Self::F32(Vec::new()),
            PlanType::F64 => Self::F64(Vec::new()),
            PlanType::String => Self::Str(Vec::new()),
            _ => Self::Values(Vec::new()),
        }
    }
    fn push(&mut self, value: Value) -> Result<(), MachineError> {
        macro_rules! push {
            ($($variant:ident),*) => {
                match (self, value) {
                    $((Self::$variant(values), Value::$variant(value)) => {
                        values.try_reserve(1).map_err(|_| MachineError::Limit)?;
                        values.push(value);
                    })*
                    (Self::Values(values), value) => {
                        values.try_reserve(1).map_err(|_| MachineError::Limit)?;
                        values.push(value);
                    }
                    _ => return Err(MachineError::Contract),
                }
            }
        }
        push!(Bool, I32, I64, U32, U64, F32, F64, Str);
        Ok(())
    }
    fn finish(self) -> ListRepr {
        macro_rules! finish {
            ($($variant:ident),*) => {
                match self {
                    $(Self::$variant(values) => ListRepr::$variant(values.into_boxed_slice()),)*
                }
            }
        }
        finish!(Bool, I32, I64, U32, U64, F32, F64, Str, Values)
    }
}

pub(crate) struct ZoneMachine {
    kind: ZoneKind,
    index: usize,
    count: usize,
    source: Option<Arc<ListRepr>>,
    carry: Option<Value>,
    builder: Option<ListBuilder>,
    base_charge: usize,
    carry_charge: usize,
    builder_charge: usize,
}

impl ZoneMachine {
    pub(crate) fn new(
        kind: ZoneKind,
        inputs: &[Value],
        charges: &[usize],
        result: &PlanType,
        limit: usize,
    ) -> Result<Self, MachineError> {
        let mut machine = Self {
            kind,
            index: 0,
            count: 0,
            source: None,
            carry: None,
            builder: None,
            base_charge: std::mem::size_of::<Self>(),
            carry_charge: 0,
            builder_charge: 0,
        };
        match (kind, inputs, charges) {
            (ZoneKind::Repeat, [Value::I64(count), initial], [_, charge]) => {
                if *count < 0 {
                    return Err(MachineError::Fault(KernelFault::NEGATIVE_COUNT));
                }
                machine.count = usize::try_from(*count).map_err(|_| MachineError::Limit)?;
                machine.carry = Some(initial.clone());
                machine.carry_charge = *charge;
            }
            (ZoneKind::Fold, [Value::List(items), initial], [_, charge]) => {
                machine.source = Some(items.clone());
                machine.count = items.len();
                machine.carry = Some(initial.clone());
                machine.carry_charge = *charge;
            }
            (ZoneKind::Map | ZoneKind::Filter, [Value::List(items)], [_]) => {
                let PlanType::List(element) = result else {
                    return Err(MachineError::Contract);
                };
                machine.builder = Some(ListBuilder::new(element));
                machine.builder_charge = BUILDER_CHARGE;
                machine.source = Some(items.clone());
                machine.count = items.len();
            }
            _ => return Err(MachineError::Contract),
        }
        if i64::try_from(machine.count).is_err() || machine.bytes()? > limit {
            return Err(MachineError::Limit);
        }
        Ok(machine)
    }

    pub(crate) fn bytes(&self) -> Result<usize, MachineError> {
        self.base_charge
            .checked_add(self.carry_charge)
            .and_then(|sum| sum.checked_add(self.builder_charge))
            .ok_or(MachineError::Limit)
    }

    pub(crate) fn finished(&self) -> bool {
        self.index == self.count
    }

    pub(crate) fn parameters(&self, outputs: &mut Vec<Value>) -> Result<(), MachineError> {
        if self.finished() {
            return Err(MachineError::Contract);
        }
        let index = Value::I64(i64::try_from(self.index).map_err(|_| MachineError::Limit)?);
        if self.kind == ZoneKind::Repeat {
            outputs.push(index);
        } else {
            outputs.push(
                self.source
                    .as_ref()
                    .and_then(|s| s.get(self.index))
                    .ok_or(MachineError::Contract)?,
            );
            outputs.push(index);
        }
        if let Some(carry) = &self.carry {
            outputs.push(carry.clone());
        }
        Ok(())
    }

    /// Validate and reserve before retaining a new carry or appending an item.
    pub(crate) fn advance(
        &mut self,
        inputs: &[Value],
        result: &PlanType,
        limit: usize,
    ) -> Result<(), MachineError> {
        let element = match (self.kind, result) {
            (ZoneKind::Map | ZoneKind::Filter, PlanType::List(element)) => element.as_ref(),
            (ZoneKind::Repeat | ZoneKind::Fold, ty) => ty,
            _ => return Err(MachineError::Contract),
        };
        match (self.kind, inputs) {
            (ZoneKind::Repeat | ZoneKind::Fold, [next]) => {
                let fixed = self
                    .base_charge
                    .checked_add(self.builder_charge)
                    .ok_or(MachineError::Limit)?;
                let charge = element
                    .checked_charge(next, limit.checked_sub(fixed).ok_or(MachineError::Limit)?)?;
                self.carry = Some(next.clone());
                self.carry_charge = charge;
            }
            (ZoneKind::Map, [item]) | (ZoneKind::Filter, [Value::Bool(true), item]) => {
                let free = limit
                    .checked_sub(self.bytes()?)
                    .ok_or(MachineError::Limit)?;
                let charge = element.checked_charge(item, free)?;
                self.builder
                    .as_mut()
                    .ok_or(MachineError::Contract)?
                    .push(item.clone())?;
                self.builder_charge = self
                    .builder_charge
                    .checked_add(charge)
                    .ok_or(MachineError::Limit)?;
            }
            (ZoneKind::Filter, [Value::Bool(false), item]) => {
                // Filter's item is strict even when omitted from the result.
                element.checked_charge(item, usize::MAX)?;
            }
            _ => return Err(MachineError::Contract),
        }
        self.index = self.index.checked_add(1).ok_or(MachineError::Limit)?;
        Ok(())
    }

    pub(crate) fn finish(self) -> Result<(Value, usize), MachineError> {
        if !self.finished() {
            return Err(MachineError::Contract);
        }
        match (self.carry, self.builder) {
            (Some(carry), None) => Ok((carry, self.carry_charge)),
            (None, Some(builder)) => {
                Ok((Value::List(Arc::new(builder.finish())), self.builder_charge))
            }
            _ => Err(MachineError::Contract),
        }
    }
}
