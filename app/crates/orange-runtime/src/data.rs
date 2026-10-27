mod registers;
mod zone;
use registers::{Outcome, Registers};
use zone::ZoneCell;

use crate::{
    ActivationEntropy, CancelToken, EntropyError, EntropyLayout, EntropySlot, EntropyStream,
    FrameId,
};
use orange_compiler::{
    ConstantFault, DataNode, DataOperation, DataPlan, DataStability, EntropySource,
    EvaluatedExport, FoldedValue, KernelFault, NodeIx,
};
use orange_model::catalog::RegistrySnapshot;
use orange_model::id::NodeId;
use orange_model::value::shape::ValueCheckError;
use orange_model::{GraphDocument, Value};
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Clone, Copy)]
pub struct DataLimits {
    pub max_slots: usize,
    pub max_nodes: usize,
    pub max_exports: usize,
    pub max_work_items: usize,
    /// Evaluated nodes and zone iteration boundaries per consumption epoch.
    pub max_operations: usize,
    /// Fuel for the whole activation: each epoch start and each evaluated node
    /// and each zone iteration cost one unit. Never refilled, so repeated epochs
    /// and loops still end.
    pub max_activation_operations: usize,
    pub max_retained_bytes: usize,
    /// Per entropy callback invocation. A request beyond this budget faults the
    /// node even if trusted callback code ignores the source error.
    pub max_entropy_draws: usize,
}
impl Default for DataLimits {
    fn default() -> Self {
        Self {
            max_slots: 200_000,
            max_nodes: 20_000,
            max_exports: 100_000,
            max_work_items: 400_001,
            max_operations: 20_000,
            // About 500 epochs that each evaluate the default maximum node count.
            max_activation_operations: 10_000_000,
            max_retained_bytes: 16 * 1024 * 1024,
            max_entropy_draws: 1024,
        }
    }
}
/// The activation keeps its own token clone, so it borrows nothing from the host.
#[derive(Debug, Clone, Default)]
pub struct DataOptions {
    pub limits: DataLimits,
    pub cancel: Option<CancelToken>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum DataError {
    #[error("The data plan belongs to another document state.")]
    StaleDocument,
    #[error("The data plan belongs to another registry snapshot.")]
    StaleRegistry,
    #[error("Data evaluation exceeds a host resource limit.")]
    ResourceLimit,
    #[error("Data evaluation was cancelled.")]
    Cancelled,
    #[error("No data consumption epoch has begun.")]
    NoEpoch,
    #[error("The activation operation budget is exhausted.")]
    FuelExhausted,
    #[error("A demanded host input was not supplied in this epoch.")]
    MissingInput { node: NodeId },
    #[error("The host input does not exist, has the wrong type or was already supplied.")]
    InvalidInput { index: usize },
    #[error("Another data pull is suspended in this epoch.")]
    PullInProgress,
    #[error("A data operation violated its checked contract.")]
    KernelContract { node: NodeId },
    #[error("{0}")]
    Entropy(#[from] EntropyError),
}
impl DataError {
    pub const fn message_key(self) -> &'static str {
        match self {
            Self::StaleDocument => "core/data-stale-document",
            Self::StaleRegistry => "core/data-stale-registry",
            Self::ResourceLimit => "core/data-resource-limit",
            Self::Cancelled => "core/data-cancelled",
            Self::NoEpoch => "core/data-no-epoch",
            Self::FuelExhausted => "core/data-fuel-exhausted",
            Self::MissingInput { .. } => "core/data-missing-input",
            Self::InvalidInput { .. } => "core/data-invalid-input",
            Self::PullInProgress => "core/data-pull-in-progress",
            Self::KernelContract { .. } => "core/data-kernel-contract",
            Self::Entropy(error) => error.message_key(),
        }
    }
}
fn checkpoint(cancel: Option<&CancelToken>) -> Result<(), DataError> {
    if cancel.is_some_and(CancelToken::is_cancelled) {
        Err(DataError::Cancelled)
    } else {
        Ok(())
    }
}
/// `invalid` names who supplied the wrongly typed value: host input or kernel.
fn value_check(invalid: DataError) -> impl Fn(ValueCheckError) -> DataError {
    move |error| match error {
        ValueCheckError::ResourceLimit => DataError::ResourceLimit,
        _ => invalid,
    }
}

/// Immutable execution metadata shared by all frames of one retained plan.
struct PreparedEntropy {
    slot: EntropySlot,
    frame: FrameId,
    counter: usize,
}
pub(crate) struct PreparedData {
    plan: Arc<DataPlan>,
    retained: usize,
    layout: EntropyLayout,
    entropy_slots: Box<[Option<PreparedEntropy>]>,
    host_indices: Box<[Option<usize>]>,
    dynamic_slots: Box<[Option<usize>]>,
    dynamic_count: usize,
}
impl PreparedData {
    pub(crate) fn new(plan: Arc<DataPlan>, limits: DataLimits) -> Result<Arc<Self>, DataError> {
        let resources = plan.resources();
        Self::check_resources(&plan, limits)?;
        let retained = plan.initial_charge().ok_or(DataError::ResourceLimit)?;
        if retained > limits.max_retained_bytes {
            return Err(DataError::ResourceLimit);
        }
        let layout = EntropyLayout::new(
            plan.nodes()
                .iter()
                .filter(|n| matches!(n.operation(), DataOperation::Entropy(_)))
                .map(|n| n.node())
                .collect::<std::collections::BTreeSet<_>>(),
            limits.max_nodes,
        )?;
        // Hash only ancestry used by actual entropy instructions. Pure plans and
        // unused potential calls need no call-frame hashing or memo allocation.
        let mut frames = std::collections::BTreeMap::new();
        let mut path = Vec::new();
        let mut counter = 0;
        let entropy_slots = plan
            .nodes()
            .iter()
            .map(|node| {
                if !matches!(node.operation(), DataOperation::Entropy(_)) {
                    return Ok(None);
                }
                let slot = layout
                    .slot(node.node())
                    .ok_or(DataError::KernelContract { node: node.node() })?;
                let mut parent = node.call_instance();
                let mut frame = FrameId::ROOT;
                while let Some(call) = parent {
                    if let Some(known) = frames.get(&call) {
                        frame = *known;
                        break;
                    }
                    let site = plan
                        .call_sites()
                        .get(call)
                        .ok_or(DataError::KernelContract { node: node.node() })?;
                    if site.parent.is_some_and(|parent| parent >= call) {
                        return Err(DataError::KernelContract { node: node.node() });
                    }
                    path.push(call);
                    parent = site.parent;
                }
                while let Some(call) = path.pop() {
                    frame = frame.call(plan.call_sites()[call].call);
                    frames.insert(call, frame);
                }
                let prepared = PreparedEntropy {
                    slot,
                    frame,
                    counter,
                };
                counter += 1;
                Ok(Some(prepared))
            })
            .collect::<Result<Box<[_]>, DataError>>()?;

        let mut host_indices = vec![None; resources.nodes];
        for (i, node) in plan.host_inputs().iter().enumerate() {
            host_indices[node.index()] = Some(i);
        }
        let mut dynamic_count = 0;
        let dynamic_slots = plan
            .initial_values()
            .iter()
            .map(|value| {
                if value.is_some() {
                    None
                } else {
                    let index = dynamic_count;
                    dynamic_count += 1;
                    Some(index)
                }
            })
            .collect();
        Ok(Arc::new(Self {
            dynamic_slots,
            dynamic_count,
            plan,
            retained,
            layout,
            entropy_slots,
            host_indices: host_indices.into(),
        }))
    }
    fn check_resources(plan: &DataPlan, limits: DataLimits) -> Result<(), DataError> {
        let resources = plan.resources();
        if plan.zones().len() > limits.max_nodes
            || resources.slots > limits.max_slots
            || resources.nodes > limits.max_nodes
            || plan.exports().len() > limits.max_exports
            || resources.work_items > limits.max_work_items
        {
            return Err(DataError::ResourceLimit);
        }
        Ok(())
    }
    fn check_limits(&self, limits: DataLimits) -> Result<(), DataError> {
        Self::check_resources(&self.plan, limits)?;
        if self.retained > limits.max_retained_bytes {
            return Err(DataError::ResourceLimit);
        }
        Ok(())
    }
}

/// Frozen root-data activation with explicit consumption epochs. This is not a
/// run actor, event scheduler, control/function frame or host-service context.
///
/// ```compile_fail
/// let frame = orange_runtime::DataActivation::default();
/// ```
pub struct DataActivation {
    plan: Arc<DataPlan>,
    document_revision: u64,
    state: Frame,
}
pub(crate) struct CleanupScope {
    fuel_used: usize,
    fuel_limit: usize,
    cancel: Option<CancelToken>,
}
/// A suspended pull retains its work stack and current consumption epoch.
pub enum DataPoll<'a> {
    Pending,
    Ready(Option<EvaluatedExport<'a>>),
}
pub(crate) enum HostPoll<'a> {
    Pending,
    Input(usize),
    Ready(Option<EvaluatedExport<'a>>),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Drive {
    Pending,
    Input(usize),
    Ready,
}
impl std::fmt::Debug for DataActivation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DataActivation")
            .field("graph", &self.plan.graph_id())
            .field("epoch", &self.state.epoch)
            .field("operations", &self.state.operations)
            .field("failed", &self.state.failed)
            .finish_non_exhaustive()
    }
}
impl DataActivation {
    pub fn start(
        plan: Arc<DataPlan>,
        document: &GraphDocument,
        registry: &RegistrySnapshot,
        seed: u64,
        activation_sequence: u64,
        options: DataOptions,
    ) -> Result<Self, DataError> {
        checkpoint(options.cancel.as_ref())?;
        // Separate failures retain useful entry diagnostics; active ownership freezes
        // the plan after entry and does not consult later authoring publications.
        if !plan.is_current(document, plan.snapshot()) {
            return Err(DataError::StaleDocument);
        }
        if plan.snapshot().generation() != registry.generation() {
            return Err(DataError::StaleRegistry);
        }
        Self::from_frozen(
            plan,
            document.revision(),
            seed,
            activation_sequence,
            options,
        )
    }
    pub(crate) fn from_frozen(
        plan: Arc<DataPlan>,
        document_revision: u64,
        seed: u64,
        activation_sequence: u64,
        options: DataOptions,
    ) -> Result<Self, DataError> {
        checkpoint(options.cancel.as_ref())?;
        let prepared = PreparedData::new(plan, options.limits)?;
        Self::from_prepared(
            prepared,
            document_revision,
            seed,
            activation_sequence,
            options,
        )
    }
    pub(crate) fn from_prepared(
        prepared: Arc<PreparedData>,
        document_revision: u64,
        seed: u64,
        activation_sequence: u64,
        options: DataOptions,
    ) -> Result<Self, DataError> {
        checkpoint(options.cancel.as_ref())?;
        prepared.check_limits(options.limits)?;
        let plan = prepared.plan.clone();
        let resources = plan.resources();
        let limits = options.limits;
        let retained = prepared.retained;
        // One counter per expanded entropy instruction; call identity is bound
        // once in PreparedData, never hashed/looked up on each callback.
        let scopes = ActivationEntropy::new_dense(
            &prepared.layout,
            seed,
            activation_sequence,
            resources.entropy_nodes,
        );
        // Immutable constants remain in the shared plan. Only dynamic slots
        // need activation-owned registers, avoiding per-entry constant copies/drop.
        let state = Frame {
            zones: (0..plan.zones().len())
                .map(|_| ZoneCell::default())
                .collect(),
            zone_bytes: 0,
            scopes,
            registers: Registers::new(prepared.dynamic_count),
            prepared,
            retained,
            memo: vec![None; resources.nodes],
            epoch: None,
            operations: 0,
            fuel_used: 0,
            failed: None,
            work: Vec::with_capacity(resources.work_items),
            pending: None,
            work_limit: resources.work_items,
            inputs: Vec::with_capacity(resources.max_inputs),
            outputs: Vec::with_capacity(resources.max_outputs),
            output_charges: Vec::with_capacity(resources.max_outputs),
            limits,
            cancel: options.cancel,
        };
        checkpoint(state.cancel.as_ref())?;
        Ok(Self {
            plan,
            document_revision,
            state,
        })
    }
    pub fn plan(&self) -> &Arc<DataPlan> {
        &self.plan
    }
    pub fn source_revision(&self) -> u64 {
        self.plan.source_revision()
    }
    pub fn entropy_metadata(&self) -> crate::EntropyReplayMetadata {
        self.state.scopes.replay_metadata()
    }
    pub fn document_revision(&self) -> u64 {
        self.document_revision
    }
    /// Activation fuel left, in operation units. It never refills.
    pub fn fuel_remaining(&self) -> usize {
        self.state
            .limits
            .max_activation_operations
            .saturating_sub(self.state.fuel_used)
    }
    /// A run-owned cancellation transition reserves distinct bounded cleanup fuel.
    /// Preserve entropy identities; ordinary callers cannot revive failed epochs.
    pub(crate) fn enter_cleanup(&mut self, fuel: usize) -> Result<(), DataError> {
        self.state.limits.max_activation_operations = self
            .state
            .fuel_used
            .checked_add(fuel)
            .ok_or(DataError::FuelExhausted)?;
        self.state.cancel = None;
        self.state.close_epoch();
        Ok(())
    }
    pub(crate) fn scoped_cleanup(&mut self, fuel: usize) -> Result<CleanupScope, DataError> {
        let scope = CleanupScope {
            fuel_used: self.state.fuel_used,
            fuel_limit: self.state.limits.max_activation_operations,
            cancel: self.state.cancel.clone(),
        };
        self.enter_cleanup(fuel)?;
        Ok(scope)
    }
    pub(crate) fn finish_cleanup(&mut self, scope: CleanupScope) {
        self.state.fuel_used = scope.fuel_used;
        self.state.limits.max_activation_operations = scope.fuel_limit;
        self.state.cancel = scope.cancel;
        self.state.close_epoch();
    }
    fn checkpoint(&mut self) -> Result<(), DataError> {
        if let Err(error) = checkpoint(self.state.cancel.as_ref()) {
            if self.state.epoch.is_some() {
                self.state.failed = Some(error);
            }
            self.state.discard_pass();
            return Err(error);
        }
        Ok(())
    }
    /// Start one consumption scope. Values from older epochs stay charged until
    /// overwritten; generation stamps prevent reading them as current results.
    /// A rejected start invalidates any active scope. Clearing cancellation alone
    /// does not restore it; recovery requires a successful explicit start.
    /// Each start costs one fuel unit; exhausted fuel rejects every later start.
    pub fn begin_epoch(&mut self) -> Result<u64, DataError> {
        self.checkpoint()?;
        self.state.discard_pass();
        let epoch = self
            .state
            .burn()
            .and_then(|()| Ok(self.state.scopes.begin_epoch()?))
            .inspect_err(|error| self.state.failed = Some(*error))?;
        self.state.epoch = Some(epoch);
        self.state.operations = 0;
        self.state.failed = None;
        Ok(epoch)
    }
    /// Supply `plan.host_inputs()[index]` for the current epoch, at most once, before
    /// or between pulls. A rejected value changes nothing and does not abort the
    /// epoch. Values never carry over: each epoch needs its own.
    pub fn set_input(&mut self, index: usize, value: Value) -> Result<(), DataError> {
        self.set_input_outcome(index, FoldedValue::Value(value))
    }
    /// Trusted host/control input publication. A graph fault stays a fault rather
    /// than being mistaken for a missing infrastructure input or an ordinary Err.
    pub fn set_input_outcome(&mut self, index: usize, value: FoldedValue) -> Result<(), DataError> {
        self.checkpoint()?;
        if let Some(error) = self.state.failed {
            return Err(error);
        }
        if self.state.epoch.is_none() {
            return Err(DataError::NoEpoch);
        }
        let invalid = DataError::InvalidInput { index };
        let node = self.plan.host_inputs().get(index).ok_or(invalid)?;
        let node = &self.plan.nodes()[node.index()];
        let [slot] = node.outputs() else {
            return Err(DataError::KernelContract { node: node.node() });
        };
        if self.state.memo[node.index().index()] == self.state.epoch
            || match &value {
                FoldedValue::Value(value) => Some(
                    self.plan
                        .checked_value_charge(*slot, value, self.state.limits.max_retained_bytes)
                        .map_err(value_check(invalid))?,
                ),
                FoldedValue::Fault(fault) => self.plan.fault_charge(fault),
            }
            .is_none()
        {
            return Err(invalid);
        }
        match value {
            FoldedValue::Value(value) => self.state.publish_value(&self.plan, node, value),
            FoldedValue::Fault(fault) => self.state.publish_fault(&self.plan, node, fault),
        }
    }
    /// Pull one ordered export inside the current explicit epoch. Other pulls in
    /// the same epoch share producers. Borrowed results prevent concurrent mutation.
    pub fn export(&mut self, index: usize) -> Result<Option<EvaluatedExport<'_>>, DataError> {
        let mut budget = usize::MAX;
        match self.poll_export(index, &mut budget)? {
            DataPoll::Ready(value) => Ok(value),
            DataPoll::Pending => unreachable!("unbounded budget covers the finite admitted stack"),
        }
    }
    /// Consume at most the supplied number of iterative work items. Pending work
    /// can resume only at the same producer; no new epoch or entropy draw is implied.
    pub fn poll_export(
        &mut self,
        index: usize,
        budget: &mut usize,
    ) -> Result<DataPoll<'_>, DataError> {
        match self.poll_export_inner(index, budget, false)? {
            HostPoll::Pending => Ok(DataPoll::Pending),
            HostPoll::Ready(value) => Ok(DataPoll::Ready(value)),
            HostPoll::Input(_) => unreachable!("public pulls reject missing inputs"),
        }
    }
    pub(crate) fn poll_host_export(
        &mut self,
        index: usize,
        budget: &mut usize,
    ) -> Result<HostPoll<'_>, DataError> {
        self.poll_export_inner(index, budget, true)
    }
    fn poll_export_inner(
        &mut self,
        index: usize,
        budget: &mut usize,
        host: bool,
    ) -> Result<HostPoll<'_>, DataError> {
        self.checkpoint()?;
        if let Some(error) = self.state.failed {
            return Err(error);
        }
        if self.state.epoch.is_none() {
            return Err(DataError::NoEpoch);
        }
        let Some(node) = self.plan.export_node(index) else {
            return Ok(HostPoll::Ready(None));
        };
        let driven = self.state.drive(&self.plan, node, budget, host);
        if driven == Err(DataError::PullInProgress) {
            return Err(DataError::PullInProgress);
        }
        if let Err(error) = driven {
            self.state.discard_pass();
            self.state.failed = Some(error);
            return Err(error);
        }
        match driven {
            Ok(Drive::Pending) => return Ok(HostPoll::Pending),
            Ok(Drive::Input(index)) => return Ok(HostPoll::Input(index)),
            _ => {}
        }
        self.checkpoint()?;
        let export = &self.plan.exports()[index];
        let slot = export.slot.index();
        let outcome = self
            .state
            .slot(export.slot)
            .ok_or(DataError::KernelContract {
                node: self.plan.nodes()[node.index()].node(),
            })?;
        Ok(HostPoll::Ready(Some(EvaluatedExport {
            id: &export.id,
            ty: &self.plan.slot_types()[slot],
            source: &self.plan.slot_sources()[slot],
            outcome,
        })))
    }
}

enum Work {
    Enter(NodeIx),
    ZonePrefix { node: NodeIx, next: usize },
    ZoneBody { node: NodeIx, next: usize },
    ZoneNext(NodeIx),
    Gather { node: NodeIx, next: usize },
    Choose(NodeIx),
    Copy { node: NodeIx, input: usize },
}
struct Frame {
    zones: Vec<ZoneCell>,
    zone_bytes: usize,
    scopes: ActivationEntropy,
    prepared: Arc<PreparedData>,
    /// Outcomes and charges of dynamic slots, indexed by `dynamic_slots`.
    registers: Registers,
    retained: usize,
    memo: Vec<Option<u64>>,
    epoch: Option<u64>,
    operations: usize,
    fuel_used: usize,
    failed: Option<DataError>,
    work: Vec<Work>,
    pending: Option<NodeIx>,
    work_limit: usize,
    inputs: Vec<Value>,
    outputs: Vec<Value>,
    output_charges: Vec<usize>,
    limits: DataLimits,
    cancel: Option<CancelToken>,
}
impl Frame {
    /// Drop an unfinished pull: work, scratch values and zone machines. Published
    /// slots and memo stamps stay; stamps alone keep stale values from being read.
    fn discard_pass(&mut self) {
        self.clear_zones();
        self.pending = None;
        self.work.clear();
        self.inputs.clear();
        self.outputs.clear();
    }
    fn close_epoch(&mut self) {
        self.failed = None;
        self.epoch = None;
        self.discard_pass();
    }
    fn stamp(&self, node: &DataNode) -> Result<u64, DataError> {
        match node.zone() {
            None => self.epoch.ok_or(DataError::NoEpoch),
            Some(zone) => self
                .zones
                .get(zone.index())
                .and_then(ZoneCell::stamp)
                .ok_or(DataError::KernelContract { node: node.node() }),
        }
    }
    fn operation(&mut self) -> Result<(), DataError> {
        self.operations = self
            .operations
            .checked_add(1)
            .filter(|count| *count <= self.limits.max_operations)
            .ok_or(DataError::ResourceLimit)?;
        self.burn()
    }
    fn burn(&mut self) -> Result<(), DataError> {
        self.fuel_used = self
            .fuel_used
            .checked_add(1)
            .filter(|used| *used <= self.limits.max_activation_operations)
            .ok_or(DataError::FuelExhausted)?;
        Ok(())
    }
    fn push(&mut self, work: Work) -> Result<(), DataError> {
        if self.work.len() >= self.work_limit {
            return Err(DataError::ResourceLimit);
        }
        self.work.push(work);
        Ok(())
    }
    fn pull_input(&mut self, node: &DataNode, input: usize) -> Result<(), DataError> {
        let input = node
            .inputs()
            .get(input)
            .ok_or(DataError::KernelContract { node: node.node() })?;
        if let Some(producer) = input.producer {
            self.push(Work::Enter(producer))?;
        }
        Ok(())
    }
    fn slot(&self, slot: orange_compiler::SlotIx) -> Option<Outcome<'_>> {
        match self.prepared.dynamic_slots[slot.index()] {
            Some(index) => self.registers.get(index),
            None => self.prepared.plan.initial_values()[slot.index()]
                .as_ref()
                .map(registers::borrow),
        }
    }
    fn outcome(&self, node: &DataNode, input: usize) -> Result<Outcome<'_>, DataError> {
        node.inputs()
            .get(input)
            .and_then(|input| self.slot(input.source.slot))
            .ok_or(DataError::KernelContract { node: node.node() })
    }
    /// Strict operand walks pull one input per step; this is the fault of the input
    /// pulled before `next`, if the walk began at `first` and has pulled any.
    fn pulled_fault(
        &self,
        node: &DataNode,
        next: usize,
        first: usize,
    ) -> Result<Option<ConstantFault>, DataError> {
        if next <= first {
            return Ok(None);
        }
        Ok(self.outcome(node, next - 1)?.err().cloned())
    }
    fn old_charge(&self, node: &DataNode) -> Result<usize, DataError> {
        let mut bytes = 0usize;
        for slot in node.outputs() {
            let index = self.prepared.dynamic_slots[slot.index()]
                .ok_or(DataError::KernelContract { node: node.node() })?;
            bytes = bytes
                .checked_add(self.registers.charge(index))
                .ok_or(DataError::ResourceLimit)?;
        }
        Ok(bytes)
    }
    fn replace_charge(&mut self, old: usize, new: usize) -> Result<(), DataError> {
        let retained = self
            .retained
            .checked_sub(old)
            .and_then(|value| value.checked_add(new))
            .ok_or(DataError::ResourceLimit)?;
        if retained
            .checked_add(self.zone_bytes)
            .is_none_or(|sum| sum > self.limits.max_retained_bytes)
        {
            return Err(DataError::ResourceLimit);
        }
        self.retained = retained;
        Ok(())
    }
    fn publish_fault(
        &mut self,
        plan: &DataPlan,
        node: &DataNode,
        fault: ConstantFault,
    ) -> Result<(), DataError> {
        let stamp = self.stamp(node)?;
        let charge = plan
            .fault_charge(&fault)
            .ok_or(DataError::KernelContract { node: node.node() })?;
        let new = charge
            .checked_mul(node.outputs().len())
            .ok_or(DataError::ResourceLimit)?;
        self.replace_charge(self.old_charge(node)?, new)?;
        for slot in node.outputs() {
            let index = self.prepared.dynamic_slots[slot.index()]
                .expect("publication checked dynamic targets");
            self.registers.set_fault(index, fault.clone(), charge);
        }
        self.memo[node.index().index()] = Some(stamp);
        Ok(())
    }
    /// Publish one single-output outcome whose type and charge were already checked
    /// by its trusted producer (a host input or a private zone machine).
    fn publish_checked(
        &mut self,
        node: &DataNode,
        outcome: Result<Value, ConstantFault>,
        charge: usize,
    ) -> Result<(), DataError> {
        let contract = DataError::KernelContract { node: node.node() };
        let [slot] = node.outputs() else {
            return Err(contract);
        };
        let index = self.prepared.dynamic_slots[slot.index()].ok_or(contract)?;
        let stamp = self.stamp(node)?;
        self.replace_charge(self.registers.charge(index), charge)?;
        match outcome {
            Ok(value) => self.registers.set_value(index, value, charge),
            Err(fault) => self.registers.set_fault(index, fault, charge),
        }
        self.memo[node.index().index()] = Some(stamp);
        Ok(())
    }
    fn publish_values(&mut self, plan: &DataPlan, node: &DataNode) -> Result<(), DataError> {
        let published = self.commit_outputs(plan, node);
        // A rejected publication releases its payloads instead of keeping them uncharged.
        self.outputs.clear();
        published
    }
    fn commit_outputs(&mut self, plan: &DataPlan, node: &DataNode) -> Result<(), DataError> {
        if self.outputs.len() != node.outputs().len() {
            return Err(DataError::KernelContract { node: node.node() });
        }
        let stamp = self.stamp(node)?;
        let mut new = 0usize;
        self.output_charges.clear();
        for (slot, value) in node.outputs().iter().zip(&self.outputs) {
            let charge = plan
                .checked_value_charge(
                    *slot,
                    value,
                    self.limits.max_retained_bytes.saturating_sub(new),
                )
                .map_err(value_check(DataError::KernelContract { node: node.node() }))?;
            new = new.checked_add(charge).ok_or(DataError::ResourceLimit)?;
            self.output_charges.push(charge);
        }
        self.replace_charge(self.old_charge(node)?, new)?;
        for ((slot, value), charge) in node
            .outputs()
            .iter()
            .zip(self.outputs.drain(..))
            .zip(self.output_charges.drain(..))
        {
            let index = self.prepared.dynamic_slots[slot.index()]
                .expect("publication checked dynamic targets");
            self.registers.set_value(index, value, charge);
        }
        self.memo[node.index().index()] = Some(stamp);
        Ok(())
    }
    fn publish_value(
        &mut self,
        plan: &DataPlan,
        node: &DataNode,
        value: Value,
    ) -> Result<(), DataError> {
        self.outputs.clear();
        self.outputs.push(value);
        self.publish_values(plan, node)
    }
    fn callback(&mut self, plan: &DataPlan, node: &DataNode) -> Result<(), DataError> {
        self.inputs.clear();
        self.outputs.clear();
        if let DataOperation::Call { arguments } = node.operation() {
            if node.inputs().len().checked_sub(*arguments) != Some(node.outputs().len()) {
                return Err(DataError::KernelContract { node: node.node() });
            }
            // Gather already checked the strict prefix and every result in order.
            // Only returned values need copying; unused arguments stay borrowed.
            for input in *arguments..node.inputs().len() {
                match self.outcome(node, input)? {
                    Ok(value) => self.outputs.push(value.clone()),
                    Err(fault) => return self.publish_fault(plan, node, fault.clone()),
                }
            }
            return self.publish_values(plan, node);
        }
        self.outputs.resize(node.outputs().len(), Value::Unit);
        for input in 0..node.inputs().len() {
            match self.outcome(node, input)? {
                Ok(value) => self.inputs.push(value.clone()),
                Err(fault) => return self.publish_fault(plan, node, fault.clone()),
            }
        }
        checkpoint(self.cancel.as_ref())?;
        let result = match node.operation() {
            DataOperation::Pure(binding) => binding.kernel.fold(&self.inputs, &mut self.outputs),
            DataOperation::Entropy(binding) => {
                let slot = self.prepared.entropy_slots[node.index().index()]
                    .as_ref()
                    .ok_or(DataError::KernelContract { node: node.node() })?;
                // Outer nodes stamp with the epoch, body nodes with their iteration.
                let stamp = self.stamp(node)?;
                let key = self.scopes.invocation_key_dense_at(
                    slot.frame,
                    &slot.slot,
                    slot.counter,
                    stamp,
                )?;
                let mut source = ScopedSource {
                    stream: EntropyStream::new(key),
                    remaining: self.limits.max_entropy_draws,
                    cancel: self.cancel.as_ref(),
                    cancelled: false,
                    fault: None,
                };
                let result = binding
                    .kernel
                    .evaluate(&self.inputs, &mut self.outputs, &mut source);
                if source.cancelled {
                    return Err(DataError::Cancelled);
                }
                source.fault.map_or(result, Err)
            }
            _ => return Err(DataError::KernelContract { node: node.node() }),
        };
        checkpoint(self.cancel.as_ref())?;
        let publication = match result {
            Ok(()) => self.publish_values(plan, node),
            Err(fault) => self.publish_fault(
                plan,
                node,
                ConstantFault {
                    source: node.node(),
                    fault,
                },
            ),
        };
        self.inputs.clear();
        self.outputs.clear();
        publication
    }
    fn choose(&mut self, plan: &DataPlan, node: &DataNode) -> Result<(), DataError> {
        let value = match self.outcome(node, 0)? {
            Err(fault) => {
                if matches!(node.operation(), DataOperation::Try { .. }) {
                    let value = Value::Err(Arc::new(orange_model::value::NodeError {
                        code: fault.fault.code.into(),
                        node: Some(fault.source),
                    }));
                    return self.publish_value(plan, node, value);
                }
                return self.publish_fault(plan, node, fault.clone());
            }
            Ok(value) => value,
        };
        let selected = match (node.operation(), value) {
            (
                DataOperation::Select {
                    when_true,
                    when_false,
                },
                Value::Bool(flag),
            ) => {
                if *flag {
                    *when_true
                } else {
                    *when_false
                }
            }
            (DataOperation::And, Value::Bool(false)) | (DataOperation::Or, Value::Bool(true)) => {
                return self.publish_value(plan, node, value.clone());
            }
            (DataOperation::And | DataOperation::Or, Value::Bool(_)) => 1,
            (DataOperation::Coalesce, Value::None) => 1,
            (DataOperation::Coalesce, Value::Some(value)) => {
                return self.publish_value(plan, node, value.as_ref().clone());
            }
            (DataOperation::Coalesce, _) => return self.publish_value(plan, node, value.clone()),
            (DataOperation::Try { boxed }, _) => {
                let value = if *boxed {
                    Value::Ok(Arc::new(value.clone()))
                } else {
                    value.clone()
                };
                return self.publish_value(plan, node, value);
            }
            _ => return Err(DataError::KernelContract { node: node.node() }),
        };
        self.push(Work::Copy {
            node: node.index(),
            input: selected,
        })?;
        self.pull_input(node, selected)
    }
    fn drive(
        &mut self,
        plan: &DataPlan,
        root: NodeIx,
        budget: &mut usize,
        host: bool,
    ) -> Result<Drive, DataError> {
        match self.pending {
            Some(pending) if pending != root => return Err(DataError::PullInProgress),
            Some(_) => {}
            None => {
                self.push(Work::Enter(root))?;
                self.pending = Some(root);
            }
        }
        while !self.work.is_empty() {
            checkpoint(self.cancel.as_ref())?;
            if *budget == 0 {
                return Ok(Drive::Pending);
            }
            *budget -= 1;
            let work = self.work.pop().expect("nonempty work stack");
            match work {
                Work::Enter(index) => {
                    let node = &plan.nodes()[index.index()];
                    if node.stability() == DataStability::Const {
                        continue;
                    }
                    if self.memo[index.index()] == Some(self.stamp(node)?) {
                        continue;
                    }
                    // A supplied input is already stamped for this epoch.
                    if matches!(node.operation(), DataOperation::Input) {
                        if host {
                            self.push(Work::Enter(index))?;
                            if let Some(input) = self.prepared.host_indices[index.index()] {
                                return Ok(Drive::Input(input));
                            }
                        }
                        return Err(DataError::MissingInput { node: node.node() });
                    }
                    self.operation()?;
                    match node.operation() {
                        DataOperation::Pure(_)
                        | DataOperation::Entropy(_)
                        | DataOperation::Call { .. } => self.push(Work::Gather {
                            node: index,
                            next: 0,
                        })?,
                        DataOperation::Zone { .. } => self.push(Work::ZonePrefix {
                            node: index,
                            next: 0,
                        })?,
                        DataOperation::Constant | DataOperation::Iteration(_) => {
                            return Err(DataError::KernelContract { node: node.node() });
                        }
                        _ => {
                            self.push(Work::Choose(index))?;
                            self.pull_input(node, 0)?;
                        }
                    }
                }
                Work::Gather { node: index, next } => {
                    let node = &plan.nodes()[index.index()];
                    if let Some(fault) = self.pulled_fault(node, next, 0)? {
                        self.publish_fault(plan, node, fault)?;
                        continue;
                    }
                    if next == node.inputs().len() {
                        self.callback(plan, node)?;
                    } else {
                        self.push(Work::Gather {
                            node: index,
                            next: next + 1,
                        })?;
                        self.pull_input(node, next)?;
                    }
                }
                Work::ZonePrefix { node, next } => self.zone_prefix(plan, node, next)?,
                Work::ZoneNext(node) => self.zone_next(plan, node)?,
                Work::ZoneBody { node, next } => self.zone_body(plan, node, next)?,
                Work::Choose(index) => self.choose(plan, &plan.nodes()[index.index()])?,
                Work::Copy { node: index, input } => {
                    let node = &plan.nodes()[index.index()];
                    match self.outcome(node, input)? {
                        Ok(value) => self.publish_value(plan, node, value.clone())?,
                        Err(fault) => self.publish_fault(plan, node, fault.clone())?,
                    }
                }
            }
        }
        self.pending = None;
        Ok(Drive::Ready)
    }
}

struct ScopedSource<'a> {
    stream: EntropyStream,
    remaining: usize,
    cancel: Option<&'a CancelToken>,
    cancelled: bool,
    fault: Option<KernelFault>,
}
impl EntropySource for ScopedSource<'_> {
    fn next_u64(&mut self) -> Result<u64, KernelFault> {
        if self.cancel.is_some_and(CancelToken::is_cancelled) {
            self.cancelled = true;
            return Err(KernelFault {
                code: "core/Cancelled",
                message_key: "core/data-cancelled",
                message: "Data evaluation was cancelled.",
            });
        }
        if let Some(fault) = self.fault {
            return Err(fault);
        }
        if self.remaining == 0 {
            let fault = KernelFault {
                code: "core/LimitExceeded",
                message_key: "core/entropy-draw-limit",
                message: "The node entropy draw budget is exhausted.",
            };
            self.fault = Some(fault);
            return Err(fault);
        }
        self.remaining -= 1;
        self.stream.next_u64().map_err(|_| {
            let fault = KernelFault {
                code: "core/LimitExceeded",
                message_key: "core/entropy-stream-exhausted",
                message: "The entropy stream is exhausted.",
            };
            self.fault = Some(fault);
            fault
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orange_compiler::{
        CompileOptions, EntropyBinding, EntropyKernelResolver, KernelBinding, PureKernelResolver,
        ResolveError, compile_data,
    };
    use orange_model::document::{NodeInstance, SignatureSnapshot};
    use orange_model::validation::HostOrigin;

    /// Core intrinsics only; no provider kernel is ever resolved.
    struct NoKernels;
    impl PureKernelResolver for NoKernels {
        fn resolve(
            &self,
            _: &GraphDocument,
            _: &NodeInstance,
            _: &SignatureSnapshot,
            _: Arc<RegistrySnapshot>,
        ) -> Result<KernelBinding, ResolveError> {
            Err(ResolveError::Unavailable)
        }
    }
    impl EntropyKernelResolver for NoKernels {
        fn resolve_entropy(
            &self,
            _: &GraphDocument,
            _: &NodeInstance,
            _: &SignatureSnapshot,
            _: Arc<RegistrySnapshot>,
        ) -> Result<EntropyBinding, ResolveError> {
            Err(ResolveError::Unavailable)
        }
    }

    const NODE: &str = "10000000-0000-4000-8000-000000000001";

    /// One run-variable read exported directly: a single host input slot.
    fn input_activation() -> DataActivation {
        let digest = format!("sha256:{}", "a".repeat(64));
        let snapshot = RegistrySnapshot::core(HostOrigin::new("0.0.0", digest).unwrap());
        let variable = "10000000-0000-4000-8000-000000000014";
        let doc = orange_model::from_json(&format!(
            r#"{{"format":"orange.graph","format_version":1,
            "graph_id":"20000000-0000-4000-8000-000000000001","revision":0,
            "package_lock":[],"signatures":{{}},"edges":[],
            "nodes":[{{"id":"{NODE}","definition_id":"core/get-variable",
                "definition_version":1,"config_version":1,"config":{{"variable":"{variable}"}}}}],
            "exports":[{{"id":"out","node":"{NODE}","port":"value"}}],
            "variables":[{{"id":"{variable}","name":"v","type":"core/u64@1","scope":"run"}}]}}"#
        ))
        .unwrap();
        let analysis = orange_model::analyze(&doc, &snapshot);
        let plan = compile_data(
            &doc,
            snapshot.clone(),
            &analysis,
            &NoKernels,
            CompileOptions::default(),
        )
        .unwrap();
        assert_eq!(plan.host_inputs().len(), 1);
        DataActivation::start(
            Arc::new(plan),
            &doc,
            &snapshot,
            0,
            0,
            DataOptions::default(),
        )
        .unwrap()
    }

    #[test]
    fn dynamic_slot_registers_are_smaller_than_folded_cells() {
        use std::mem::size_of;
        // Value register + state byte + charge, versus the former cell + charge.
        let register = size_of::<Value>() + 1 + size_of::<usize>();
        let folded = size_of::<Option<FoldedValue>>() + size_of::<usize>();
        assert!(register < folded, "{register} >= {folded}");
    }

    #[test]
    fn fault_provenance_survives_value_fault_value_republish() {
        let mut frame = input_activation();
        let fault = ConstantFault {
            source: NODE.parse().unwrap(),
            fault: KernelFault::NEGATIVE_COUNT,
        };
        for epoch in 0..4 {
            frame.begin_epoch().unwrap();
            if epoch % 2 == 0 {
                frame.set_input(0, Value::U64(epoch)).unwrap();
                let export = frame.export(0).unwrap().unwrap();
                assert_eq!(export.outcome, Ok(&Value::U64(epoch)));
            } else {
                frame
                    .set_input_outcome(0, FoldedValue::Fault(fault.clone()))
                    .unwrap();
                let export = frame.export(0).unwrap().unwrap();
                assert_eq!(export.outcome, Err(&fault));
            }
        }
    }
}
