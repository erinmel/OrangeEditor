//! Owned, bounded zone accumulators and the Frame steps that drive them. The
//! generic work loop, memoization and slot publication stay in `data`.
//!
//! A Fold/Map/Filter source list stays retained by its memoized outer slot for the
//! machine's whole life, so the machine charges only its shared handle (part of
//! its own size), never the list again.

use super::{DataError, Frame, Work};
use orange_compiler::{
    ConstantFault, DataNode, DataOperation, DataPlan, FoldedValue, KernelFault, NodeIx, ZoneIx,
};
use orange_model::Value;
use orange_model::id::NodeId;
use orange_model::validation::ZoneKind;
use orange_model::value::shape::{VALUE_CHARGE, ValueCheckError, ValueShape};
use orange_model::value::{ListBuilder, ListPushError, ListRepr};
use std::sync::Arc;

/// Begin inputs collected before the first iteration: Repeat count/initial,
/// Fold items/initial, Map/Filter items.
const MAX_PREFIX: usize = 2;

/// Per-zone state in a Frame. The generation is never reset, so a body memo
/// stamp from an earlier iteration or epoch can never match a later one.
#[derive(Default)]
pub(super) struct ZoneCell {
    generation: u64,
    live: bool,
    machine: Option<Box<ZoneMachine>>,
}

impl ZoneCell {
    /// Memo stamp of the live iteration; None between iterations.
    pub(super) fn stamp(&self) -> Option<u64> {
        self.live.then_some(self.generation)
    }
}

enum MachineError {
    Contract,
    Limit,
    Fault(KernelFault),
}

impl From<ValueCheckError> for MachineError {
    fn from(error: ValueCheckError) -> Self {
        match error {
            ValueCheckError::ResourceLimit => Self::Limit,
            _ => Self::Contract,
        }
    }
}

impl From<ListPushError> for MachineError {
    fn from(error: ListPushError) -> Self {
        match error {
            ListPushError::Capacity => Self::Limit,
            _ => Self::Contract,
        }
    }
}

struct ZoneMachine {
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
    /// `result` is the end's type: the carry type, or the list type a builder fills.
    fn new(
        kind: ZoneKind,
        inputs: &[Value],
        result: &ValueShape,
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
        let free = limit
            .checked_sub(machine.base_charge)
            .ok_or(MachineError::Limit)?;
        match (kind, inputs) {
            (ZoneKind::Repeat, [Value::I64(count), initial]) => {
                if *count < 0 {
                    return Err(MachineError::Fault(KernelFault::NEGATIVE_COUNT));
                }
                machine.count = usize::try_from(*count).map_err(|_| MachineError::Limit)?;
                machine.carry_charge = result.checked_charge(initial, free)?;
                machine.carry = Some(initial.clone());
            }
            (ZoneKind::Fold, [Value::List(items), initial]) => {
                machine.carry_charge = result.checked_charge(initial, free)?;
                machine.source = Some(items.clone());
                machine.count = items.len();
                machine.carry = Some(initial.clone());
            }
            (ZoneKind::Map | ZoneKind::Filter, [Value::List(items)]) => {
                let ValueShape::List(element) = result else {
                    return Err(MachineError::Contract);
                };
                machine.builder = Some(ListBuilder::new(Some(element)));
                // An empty list charges one value; each push adds its checked element charge.
                machine.builder_charge = VALUE_CHARGE;
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

    fn bytes(&self) -> Result<usize, MachineError> {
        self.base_charge
            .checked_add(self.carry_charge)
            .and_then(|sum| sum.checked_add(self.builder_charge))
            .ok_or(MachineError::Limit)
    }

    fn finished(&self) -> bool {
        self.index == self.count
    }

    fn parameters(&self, outputs: &mut Vec<Value>) -> Result<(), MachineError> {
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
    fn advance(
        &mut self,
        inputs: &[Value],
        result: &ValueShape,
        limit: usize,
    ) -> Result<(), MachineError> {
        let element = match (self.kind, result) {
            (ZoneKind::Map | ZoneKind::Filter, ValueShape::List(element)) => element.as_ref(),
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
            // Filter's item is strict even when omitted: it was already demanded,
            // fault-checked and type-checked when its producer published it.
            (ZoneKind::Filter, [Value::Bool(false), _]) => {}
            _ => return Err(MachineError::Contract),
        }
        self.index = self.index.checked_add(1).ok_or(MachineError::Limit)?;
        Ok(())
    }

    fn finish(self) -> Result<(Value, usize), MachineError> {
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

fn zone_operation(node: &DataNode) -> Result<(ZoneIx, usize), DataError> {
    match node.operation() {
        DataOperation::Zone { zone, prefix } => Ok((*zone, *prefix)),
        _ => Err(DataError::KernelContract { node: node.node() }),
    }
}
fn machine_error(node: NodeId, error: MachineError) -> DataError {
    match error {
        MachineError::Limit => DataError::ResourceLimit,
        MachineError::Contract | MachineError::Fault(_) => DataError::KernelContract { node },
    }
}

impl Frame {
    pub(super) fn clear_zones(&mut self) {
        for zone in &mut self.zones {
            zone.machine = None;
            zone.live = false;
        }
        self.zone_bytes = 0;
    }
    /// Collect the strict begin prefix (count/items/initial), then start the zone.
    pub(super) fn zone_prefix(
        &mut self,
        plan: &DataPlan,
        index: NodeIx,
        next: usize,
    ) -> Result<(), DataError> {
        let node = &plan.nodes()[index.index()];
        let (zone, prefix) = zone_operation(node)?;
        if let Some(fault) = self.pulled_fault(node, next, 0)? {
            return self.publish_fault(plan, node, fault);
        }
        if next == prefix {
            return self.start_zone(plan, node, zone, prefix);
        }
        self.push(Work::ZonePrefix {
            node: index,
            next: next + 1,
        })?;
        self.pull_input(node, next)
    }
    /// Finish the zone, or publish the next iteration's parameters under a fresh
    /// generation and demand its body.
    pub(super) fn zone_next(&mut self, plan: &DataPlan, index: NodeIx) -> Result<(), DataError> {
        self.operation()?;
        let node = &plan.nodes()[index.index()];
        let (zone, prefix) = zone_operation(node)?;
        let machine = self.zones[zone.index()]
            .machine
            .as_ref()
            .ok_or(DataError::KernelContract { node: node.node() })?;
        if machine.finished() {
            return self.finish_zone(node, zone);
        }
        self.outputs.clear();
        machine
            .parameters(&mut self.outputs)
            .map_err(|e| machine_error(node.node(), e))?;
        let cell = &mut self.zones[zone.index()];
        cell.generation = cell
            .generation
            .checked_add(1)
            .ok_or(DataError::ResourceLimit)?;
        cell.live = true;
        let parameters = plan.zones()[zone.index()]
            .parameters()
            .ok_or(DataError::KernelContract { node: node.node() })?;
        self.publish_values(plan, &plan.nodes()[parameters.index()])?;
        self.push(Work::ZoneBody {
            node: index,
            next: prefix,
        })
    }
    /// Demand the end's body inputs for this iteration, then advance the machine.
    pub(super) fn zone_body(
        &mut self,
        plan: &DataPlan,
        index: NodeIx,
        next: usize,
    ) -> Result<(), DataError> {
        let node = &plan.nodes()[index.index()];
        let (zone, prefix) = zone_operation(node)?;
        if let Some(fault) = self.pulled_fault(node, next, prefix)? {
            self.retire_zone(zone, node.node())?;
            return self.publish_fault(plan, node, fault);
        }
        if next == node.inputs().len() {
            self.advance_zone(plan, node, zone, prefix)?;
            return self.push(Work::ZoneNext(index));
        }
        self.push(Work::ZoneBody {
            node: index,
            next: next + 1,
        })?;
        self.pull_input(node, next)
    }
    fn zone_capacity(&self, credit: usize) -> Result<usize, DataError> {
        self.limits
            .max_retained_bytes
            .checked_sub(self.retained)
            .and_then(|free| free.checked_sub(self.zone_bytes.checked_sub(credit)?))
            .ok_or(DataError::ResourceLimit)
    }
    /// Copy already demanded, fault-free operand values into the scratch inputs.
    fn collect_inputs(
        &mut self,
        node: &DataNode,
        inputs: std::ops::Range<usize>,
    ) -> Result<(), DataError> {
        self.inputs.clear();
        for i in inputs {
            let FoldedValue::Value(value) = self.outcome(node, i)? else {
                return Err(DataError::KernelContract { node: node.node() });
            };
            self.inputs.push(value.clone());
        }
        Ok(())
    }
    fn start_zone(
        &mut self,
        plan: &DataPlan,
        node: &DataNode,
        zone: ZoneIx,
        prefix: usize,
    ) -> Result<(), DataError> {
        if prefix > MAX_PREFIX || self.zones[zone.index()].machine.is_some() {
            return Err(DataError::KernelContract { node: node.node() });
        }
        self.collect_inputs(node, 0..prefix)?;
        let result = node
            .outputs()
            .first()
            .ok_or(DataError::KernelContract { node: node.node() })?;
        let machine = ZoneMachine::new(
            plan.zones()[zone.index()].kind(),
            &self.inputs,
            &plan.slot_types()[result.index()],
            self.zone_capacity(0)?,
        );
        self.inputs.clear();
        match machine {
            Ok(machine) => {
                self.zone_bytes = self
                    .zone_bytes
                    .checked_add(machine.bytes().map_err(|e| machine_error(node.node(), e))?)
                    .ok_or(DataError::ResourceLimit)?;
                self.zones[zone.index()].machine = Some(Box::new(machine));
                self.push(Work::ZoneNext(node.index()))
            }
            Err(MachineError::Fault(fault)) => self.publish_fault(
                plan,
                node,
                ConstantFault {
                    source: plan.zones()[zone.index()].begin(),
                    fault,
                },
            ),
            Err(error) => Err(machine_error(node.node(), error)),
        }
    }
    fn advance_zone(
        &mut self,
        plan: &DataPlan,
        node: &DataNode,
        zone: ZoneIx,
        prefix: usize,
    ) -> Result<(), DataError> {
        self.collect_inputs(node, prefix..node.inputs().len())?;
        let missing = DataError::KernelContract { node: node.node() };
        let old = self.zones[zone.index()]
            .machine
            .as_ref()
            .ok_or(missing)?
            .bytes()
            .map_err(|e| machine_error(node.node(), e))?;
        let capacity = self.zone_capacity(old)?;
        // The machine stays in its cell, so zone_bytes remains exact on failure;
        // advance validates and reserves before it changes any state.
        let machine = self.zones[zone.index()].machine.as_mut().ok_or(missing)?;
        let new = machine
            .advance(
                &self.inputs,
                &plan.slot_types()[node.outputs()[0].index()],
                capacity,
            )
            .and_then(|()| machine.bytes())
            .map_err(|e| machine_error(node.node(), e))?;
        self.inputs.clear();
        self.zone_bytes = self
            .zone_bytes
            .checked_sub(old)
            .and_then(|n| n.checked_add(new))
            .ok_or(DataError::ResourceLimit)?;
        Ok(())
    }
    fn retire_zone(&mut self, zone: ZoneIx, node: NodeId) -> Result<Box<ZoneMachine>, DataError> {
        let cell = &mut self.zones[zone.index()];
        cell.live = false;
        let machine = cell
            .machine
            .take()
            .ok_or(DataError::KernelContract { node })?;
        self.zone_bytes = self
            .zone_bytes
            .checked_sub(machine.bytes().map_err(|e| machine_error(node, e))?)
            .ok_or(DataError::ResourceLimit)?;
        Ok(machine)
    }
    fn finish_zone(&mut self, node: &DataNode, zone: ZoneIx) -> Result<(), DataError> {
        let machine = self.retire_zone(zone, node.node())?;
        let (value, charge) = machine
            .finish()
            .map_err(|e| machine_error(node.node(), e))?;
        // Only the private machine can produce this verified value/charge pair.
        // Each append/carry was checked before retention; provider outputs still
        // use commit_outputs' full type/byte validation.
        let [slot] = node.outputs() else {
            return Err(DataError::KernelContract { node: node.node() });
        };
        let target = self.prepared.dynamic_slots[slot.index()]
            .ok_or(DataError::KernelContract { node: node.node() })?;
        let stamp = self.stamp(node)?;
        self.replace_charge(self.old_charge(node)?, charge)?;
        self.slots[target] = Some(FoldedValue::Value(value));
        self.charges[target] = charge;
        self.memo[node.index().index()] = Some(stamp);
        Ok(())
    }
}
