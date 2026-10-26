use crate::{
    ActivationEntropy, CancelToken, EntropyError, EntropyLayout, EntropySlot, EntropyStream,
};
use orange_compiler::{
    ConstantFault, DataNode, DataOperation, DataPlan, DataStability, EntropySource,
    EvaluatedExport, FoldedValue, KernelFault, NodeIx,
};
use orange_model::catalog::RegistrySnapshot;
use orange_model::id::NodeId;
use orange_model::{GraphDocument, Value};
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Clone, Copy)]
pub struct DataLimits {
    pub max_slots: usize,
    pub max_nodes: usize,
    pub max_exports: usize,
    pub max_work_items: usize,
    /// Per consumption epoch.
    pub max_operations: usize,
    /// Fuel for the whole activation: each epoch start and each evaluated node
    /// costs one unit. Never refilled, so epoch-per-iteration loops still end.
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

/// Immutable execution metadata shared by all frames of one retained plan.
pub(crate) struct PreparedData {
    plan: Arc<DataPlan>,
    retained: usize,
    layout: EntropyLayout,
    entropy_slots: Box<[Option<EntropySlot>]>,
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
                .map(|n| n.node()),
            limits.max_nodes,
        )?;
        let entropy_slots = plan
            .nodes()
            .iter()
            .map(|n| {
                if matches!(n.operation(), DataOperation::Entropy(_)) {
                    layout.slot(n.node())
                } else {
                    None
                }
            })
            .collect::<Box<[_]>>();

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
        if resources.slots > limits.max_slots
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
        // Root data plans have no callee frames.
        let scopes = ActivationEntropy::new(&prepared.layout, seed, activation_sequence, 0);
        // Immutable constants remain in the shared plan. Only dynamic slots
        // need activation-owned cells, avoiding per-entry constant copies/drop.
        let slots = vec![None; prepared.dynamic_count];
        let state = Frame {
            scopes,
            prepared,
            slots,
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
        self.state.failed = None;
        self.state.epoch = None;
        self.state.pending = None;
        self.state.work.clear();
        self.state.inputs.clear();
        self.state.outputs.clear();
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
        self.state.failed = None;
        self.state.epoch = None;
        self.state.pending = None;
        self.state.work.clear();
        self.state.inputs.clear();
        self.state.outputs.clear();
    }
    fn checkpoint(&mut self) -> Result<(), DataError> {
        if let Err(error) = checkpoint(self.state.cancel.as_ref()) {
            if self.state.epoch.is_some() {
                self.state.failed = Some(error);
            }
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
        let epoch = self
            .state
            .burn()
            .and_then(|()| Ok(self.state.scopes.begin_epoch()?))
            .inspect_err(|error| self.state.failed = Some(*error))?;
        self.state.epoch = Some(epoch);
        self.state.operations = 0;
        self.state.failed = None;
        self.state.work.clear();
        self.state.pending = None;
        self.state.inputs.clear();
        self.state.outputs.clear();
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
                FoldedValue::Value(value) => self.plan.value_charge(*slot, value),
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
            self.state.work.clear();
            self.state.inputs.clear();
            self.state.outputs.clear();
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
        let outcome = match self
            .state
            .slot(export.slot)
            .ok_or(DataError::KernelContract {
                node: self.plan.nodes()[node.index()].node(),
            })? {
            FoldedValue::Value(value) => Ok(value),
            FoldedValue::Fault(fault) => Err(fault),
        };
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
    Gather { node: NodeIx, next: usize },
    Choose(NodeIx),
    Copy { node: NodeIx, input: usize },
}
struct Frame {
    scopes: ActivationEntropy,
    prepared: Arc<PreparedData>,
    slots: Vec<Option<FoldedValue>>,
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
    limits: DataLimits,
    cancel: Option<CancelToken>,
}
impl Frame {
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
    fn slot(&self, slot: orange_compiler::SlotIx) -> Option<&FoldedValue> {
        match self.prepared.dynamic_slots[slot.index()] {
            Some(index) => self.slots[index].as_ref(),
            None => self.prepared.plan.initial_values()[slot.index()].as_ref(),
        }
    }
    fn outcome(&self, node: &DataNode, input: usize) -> Result<&FoldedValue, DataError> {
        node.inputs()
            .get(input)
            .and_then(|input| self.slot(input.source.slot))
            .ok_or(DataError::KernelContract { node: node.node() })
    }
    fn old_charge(&self, plan: &DataPlan, node: &DataNode) -> Result<usize, DataError> {
        let mut bytes = 0usize;
        for slot in node.outputs() {
            let index = self.prepared.dynamic_slots[slot.index()]
                .ok_or(DataError::KernelContract { node: node.node() })?;
            let value = match self.slots[index].as_ref() {
                Some(FoldedValue::Value(value)) => plan.value_charge(*slot, value),
                Some(FoldedValue::Fault(fault)) => plan.fault_charge(fault),
                None => Some(0),
            }
            .ok_or(DataError::KernelContract { node: node.node() })?;
            bytes = bytes.checked_add(value).ok_or(DataError::ResourceLimit)?;
        }
        Ok(bytes)
    }
    fn replace_charge(&mut self, old: usize, new: usize) -> Result<(), DataError> {
        let retained = self
            .retained
            .checked_sub(old)
            .and_then(|value| value.checked_add(new))
            .ok_or(DataError::ResourceLimit)?;
        if retained > self.limits.max_retained_bytes {
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
        let charge = plan
            .fault_charge(&fault)
            .ok_or(DataError::KernelContract { node: node.node() })?;
        let new = charge
            .checked_mul(node.outputs().len())
            .ok_or(DataError::ResourceLimit)?;
        self.replace_charge(self.old_charge(plan, node)?, new)?;
        for slot in node.outputs() {
            let index = self.prepared.dynamic_slots[slot.index()]
                .expect("publication checked dynamic targets");
            self.slots[index] = Some(FoldedValue::Fault(fault.clone()));
        }
        self.memo[node.index().index()] = self.epoch;
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
        let mut new = 0usize;
        for (slot, value) in node.outputs().iter().zip(&self.outputs) {
            let charge = plan
                .value_charge(*slot, value)
                .ok_or(DataError::KernelContract { node: node.node() })?;
            new = new.checked_add(charge).ok_or(DataError::ResourceLimit)?;
        }
        self.replace_charge(self.old_charge(plan, node)?, new)?;
        for (slot, value) in node.outputs().iter().zip(self.outputs.drain(..)) {
            let index = self.prepared.dynamic_slots[slot.index()]
                .expect("publication checked dynamic targets");
            self.slots[index] = Some(FoldedValue::Value(value));
        }
        self.memo[node.index().index()] = self.epoch;
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
        self.outputs.resize(node.outputs().len(), Value::Unit);
        for input in 0..node.inputs().len() {
            match self.outcome(node, input)? {
                FoldedValue::Value(value) => self.inputs.push(value.clone()),
                FoldedValue::Fault(fault) => return self.publish_fault(plan, node, fault.clone()),
            }
        }
        checkpoint(self.cancel.as_ref())?;
        let result = match node.operation() {
            DataOperation::Pure(binding) => binding.kernel.fold(&self.inputs, &mut self.outputs),
            DataOperation::Entropy(binding) => {
                let slot = self.prepared.entropy_slots[node.index().index()]
                    .as_ref()
                    .ok_or(DataError::KernelContract { node: node.node() })?;
                let key = self.scopes.invocation_key(slot)?;
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
            FoldedValue::Fault(fault) => {
                if matches!(node.operation(), DataOperation::Try { .. }) {
                    let value = Value::Err(Arc::new(orange_model::value::NodeError {
                        code: fault.fault.code.into(),
                        node: Some(fault.source),
                    }));
                    return self.publish_value(plan, node, value);
                }
                return self.publish_fault(plan, node, fault.clone());
            }
            FoldedValue::Value(value) => value,
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
                    if node.stability() == DataStability::Const
                        || self.memo[index.index()] == self.epoch
                    {
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
                    self.operations = self
                        .operations
                        .checked_add(1)
                        .filter(|count| *count <= self.limits.max_operations)
                        .ok_or(DataError::ResourceLimit)?;
                    self.burn()?;
                    match node.operation() {
                        DataOperation::Pure(_) | DataOperation::Entropy(_) => {
                            self.push(Work::Gather {
                                node: index,
                                next: 0,
                            })?
                        }
                        DataOperation::Constant => {
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
                    if next > 0
                        && let FoldedValue::Fault(fault) = self.outcome(node, next - 1)?
                    {
                        self.publish_fault(plan, node, fault.clone())?;
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
                Work::Choose(index) => self.choose(plan, &plan.nodes()[index.index()])?,
                Work::Copy { node: index, input } => {
                    let node = &plan.nodes()[index.index()];
                    match self.outcome(node, input)?.clone() {
                        FoldedValue::Value(value) => self.publish_value(plan, node, value)?,
                        FoldedValue::Fault(fault) => self.publish_fault(plan, node, fault)?,
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
