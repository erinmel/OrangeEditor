//! Activation-owned registers for dynamic data slots.
//!
//! A slot is a dense `Value` register, a one-byte state and its retained charge.
//! Faults are rare, so their full `ConstantFault` provenance lives in a side store
//! keyed by register: an entry exists exactly while its register holds a fault.
//! Overwriting a register removes its entry, so the store never holds more entries
//! than there are dynamic slots and needs no per-epoch sweep (older epochs' slots
//! stay charged and readable until overwritten, faults included).

use orange_compiler::{ConstantFault, FoldedValue};
use orange_model::Value;
use std::collections::BTreeMap;

/// A borrowed slot outcome, shaped like `EvaluatedExport::outcome`.
pub(super) type Outcome<'a> = Result<&'a Value, &'a ConstantFault>;

pub(super) fn borrow(value: &FoldedValue) -> Outcome<'_> {
    match value {
        FoldedValue::Value(value) => Ok(value),
        FoldedValue::Fault(fault) => Err(fault),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Empty,
    Value,
    Fault,
}

/// Bytes one dynamic slot occupies across the parallel register arrays.
pub(super) const SLOT_BYTES: usize =
    std::mem::size_of::<Value>() + std::mem::size_of::<State>() + std::mem::size_of::<usize>();

pub(super) struct Registers {
    values: Vec<Value>,
    states: Vec<State>,
    charges: Vec<usize>,
    faults: BTreeMap<usize, ConstantFault>,
}

impl Registers {
    pub(super) fn new(count: usize) -> Self {
        Self {
            values: vec![Value::Unit; count],
            states: vec![State::Empty; count],
            charges: vec![0; count],
            faults: BTreeMap::new(),
        }
    }
    /// None until the first publication, or if a fault entry is missing.
    pub(super) fn get(&self, index: usize) -> Option<Outcome<'_>> {
        match self.states[index] {
            State::Empty => None,
            State::Value => Some(Ok(&self.values[index])),
            State::Fault => self.faults.get(&index).map(Err),
        }
    }
    pub(super) fn charge(&self, index: usize) -> usize {
        self.charges[index]
    }
    /// Retain a value; a stale fault entry of this register is released.
    pub(super) fn set_value(&mut self, index: usize, value: Value, charge: usize) {
        if self.states[index] == State::Fault {
            self.faults.remove(&index);
        }
        self.values[index] = value;
        self.states[index] = State::Value;
        self.charges[index] = charge;
    }
    /// Retain a fault with its provenance; the previous value payload is released.
    pub(super) fn set_fault(&mut self, index: usize, fault: ConstantFault, charge: usize) {
        self.values[index] = Value::Unit;
        self.faults.insert(index, fault);
        self.states[index] = State::Fault;
        self.charges[index] = charge;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orange_compiler::KernelFault;
    use orange_model::value::JsonText;
    use std::sync::Arc;

    fn fault(source: u32, fault: KernelFault) -> ConstantFault {
        ConstantFault {
            source: format!("10000000-0000-4000-8000-{source:012x}")
                .parse()
                .unwrap(),
            fault,
        }
    }

    #[test]
    fn republish_keeps_fault_provenance_and_releases_the_replaced_outcome() {
        let mut registers = Registers::new(2);
        assert_eq!(registers.get(0), None);
        let payload = Arc::new(JsonText::new("1".into()).unwrap());
        let observer = Arc::downgrade(&payload);
        registers.set_value(0, Value::Json(payload), 16);
        let first = fault(7, KernelFault::NEGATIVE_COUNT);
        registers.set_fault(0, first.clone(), 32);
        // The fault releases the value payload and keeps source and kernel fault.
        assert!(observer.upgrade().is_none());
        assert_eq!(registers.get(0), Some(Err(&first)));
        assert_eq!(registers.charge(0), 32);
        registers.set_value(0, Value::U64(3), 16);
        assert_eq!(registers.get(0), Some(Ok(&Value::U64(3))));
        assert!(registers.faults.is_empty());
        // Overwriting a fault with a fault keeps one entry per register.
        let second = fault(9, KernelFault::NEGATIVE_COUNT);
        registers.set_fault(0, first, 32);
        registers.set_fault(0, second.clone(), 32);
        registers.set_fault(1, second.clone(), 32);
        assert_eq!(registers.faults.len(), 2);
        assert_eq!(registers.get(0), Some(Err(&second)));
        assert_eq!(registers.get(1), Some(Err(&second)));
    }
}
