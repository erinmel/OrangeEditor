//! Host-owned serialized workspace values and immutable versioned publications.
use crate::budget::{Budget, Tally};
use orange_compiler::{KernelFault, UpdateOperation};
use orange_model::catalog::RegistrySnapshot;
use orange_model::document::VarScope;
use orange_model::id::{TypeExpr, VariableId};
use orange_model::value::shape::{ValueCheckError, ValueShape};
use orange_model::{GraphDocument, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use thiserror::Error;
mod bus;
mod schema;
pub(crate) use bus::{BusLease, Emission};
pub use bus::{
    WorkspaceBus, WorkspaceBusError, WorkspaceBusId, WorkspaceBusLimits, WorkspaceBusMeter,
    WorkspaceBusRetention, WorkspaceBusStats, WorkspaceDelivery, WorkspaceDeliveryRefused,
    WorkspaceEnvelope, WorkspaceOrigin, WorkspaceQueue,
};
static NEXT_WORKSPACE: AtomicU64 = AtomicU64::new(1);
fn identity() -> Result<WorkspaceId, WorkspaceError> {
    let mut old = NEXT_WORKSPACE.load(Ordering::Relaxed);
    loop {
        let next = old
            .checked_add(1)
            .ok_or(WorkspaceError::IdentityExhausted)?;
        match NEXT_WORKSPACE.compare_exchange_weak(old, next, Ordering::Relaxed, Ordering::Relaxed)
        {
            Ok(_) => return Ok(WorkspaceId(old)),
            Err(current) => old = current,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkspaceId(u64);
#[derive(Debug, Clone, Copy)]
pub struct WorkspaceLimits {
    pub max_variables: usize,
    pub max_schema_bytes: usize,
    pub max_value_bytes: usize,
    pub max_state_bytes: usize,
    /// Includes old generations/commit views retained outside the actor.
    pub max_retained_bytes: usize,
}
impl Default for WorkspaceLimits {
    fn default() -> Self {
        Self {
            max_variables: 4096,
            max_schema_bytes: 16 * 1024 * 1024,
            max_value_bytes: 16 * 1024 * 1024,
            max_state_bytes: 16 * 1024 * 1024,
            max_retained_bytes: 64 * 1024 * 1024,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum WorkspaceError {
    #[error("The workspace schema or value violates its checked contract.")]
    Contract,
    #[error("The workspace variable is unavailable.")]
    UnknownVariable,
    #[error("The workspace variable is uninitialized.")]
    Uninitialized,
    #[error("The workspace exceeds a configured bound.")]
    Limit,
    #[error("The workspace identity or version space is exhausted.")]
    IdentityExhausted,
    #[error("The workspace schema is incompatible with this document.")]
    SchemaMismatch,
    #[error("The document uses an unsupported execution domain.")]
    DomainUnsupported,
    #[error("The workspace update failed.")]
    Failed(KernelFault),
}
impl WorkspaceError {
    pub const fn message_key(self) -> &'static str {
        match self {
            Self::Contract => "core/workspace-contract",
            Self::UnknownVariable => "core/workspace-unknown-variable",
            Self::Uninitialized => "core/workspace-uninitialized",
            Self::Limit => "core/workspace-limit",
            Self::IdentityExhausted => "core/workspace-identity-exhausted",
            Self::SchemaMismatch => "core/workspace-schema-mismatch",
            Self::DomainUnsupported => "core/workspace-domain-unsupported",
            Self::Failed(f) => f.message_key,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkspaceRetention {
    pub reserved_bytes: usize,
    pub limit_bytes: usize,
}
/// Read-only accounting observer; owns no values or snapshot generation.
#[derive(Clone)]
pub struct WorkspaceMeter(Arc<Budget>);
impl WorkspaceMeter {
    pub fn retention(&self) -> WorkspaceRetention {
        WorkspaceRetention {
            reserved_bytes: self.0.used(),
            limit_bytes: self.0.limit(),
        }
    }
}
impl std::fmt::Debug for WorkspaceMeter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.retention().fmt(f)
    }
}
struct Variable {
    ty: TypeExpr,
    shape: ValueShape,
}
struct Schema {
    id: WorkspaceId,
    variables: BTreeMap<VariableId, Variable>,
    registry: Arc<RegistrySnapshot>,
    budget: Arc<Budget>,
    _hold: Tally,
}
struct Cell {
    value: Option<Value>,
    bytes: usize,
    _hold: Tally,
}
struct SnapshotData {
    schema: Arc<Schema>,
    version: u64,
    values: BTreeMap<VariableId, Arc<Cell>>,
    value_bytes: usize,
    _hold: Tally,
}
/// Immutable shared values. Old roots/cells retain budget leases until last drop.
#[derive(Clone)]
pub struct WorkspaceSnapshot(Arc<SnapshotData>);
impl std::fmt::Debug for WorkspaceSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkspaceSnapshot")
            .field("id", &self.id())
            .field("version", &self.version())
            .field("variables", &self.0.values.len())
            .finish()
    }
}
impl WorkspaceSnapshot {
    pub(crate) fn matches(&self, id: VariableId, shape: &ValueShape) -> bool {
        self.0
            .schema
            .variables
            .get(&id)
            .is_some_and(|v| &v.shape == shape)
    }
    pub fn registry(&self) -> &Arc<RegistrySnapshot> {
        &self.0.schema.registry
    }
    pub fn id(&self) -> WorkspaceId {
        self.0.schema.id
    }
    pub fn version(&self) -> u64 {
        self.0.version
    }
    pub fn get(&self, id: VariableId) -> Result<Option<&Value>, WorkspaceError> {
        self.0
            .values
            .get(&id)
            .map(|c| c.value.as_ref())
            .ok_or(WorkspaceError::UnknownVariable)
    }
    pub fn variable_type(&self, id: VariableId) -> Option<&TypeExpr> {
        self.0.schema.variables.get(&id).map(|v| &v.ty)
    }
    pub fn value_bytes(&self) -> usize {
        self.0.value_bytes
    }
    pub fn meter(&self) -> WorkspaceMeter {
        WorkspaceMeter(self.0.schema.budget.clone())
    }
    /// A document may use a compatible subset. Initializers/names do not reset a
    /// host workspace that already exists. Schema mutation is a separate operation.
    pub fn check_schema(
        &self,
        document: &GraphDocument,
        registry: &RegistrySnapshot,
    ) -> Result<(), WorkspaceError> {
        check_domains(document, registry)?;
        for declaration in document
            .variables()
            .values()
            .filter(|d| d.scope == VarScope::Workspace)
        {
            let variable = self
                .0
                .schema
                .variables
                .get(&declaration.id)
                .ok_or(WorkspaceError::SchemaMismatch)?;
            let shape = ValueShape::of_in(&declaration.ty, registry.types().as_ref())
                .ok_or(WorkspaceError::SchemaMismatch)?;
            if variable.ty != declaration.ty || variable.shape != shape {
                return Err(WorkspaceError::SchemaMismatch);
            }
        }
        Ok(())
    }
}
/// Read-only result views retain old/new cell leases, not an executable request.
pub struct WorkspaceCommit {
    snapshot: WorkspaceSnapshot,
    variable: VariableId,
    old: Arc<Cell>,
    new: Arc<Cell>,
    written: bool,
    exchanged: Option<bool>,
    changed: Option<bool>,
}
impl std::fmt::Debug for WorkspaceCommit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkspaceCommit")
            .field("snapshot", &self.snapshot)
            .field("variable", &self.variable)
            .field("written", &self.written)
            .field("exchanged", &self.exchanged)
            .field("changed", &self.changed)
            .finish()
    }
}
impl WorkspaceCommit {
    pub fn snapshot(&self) -> &WorkspaceSnapshot {
        &self.snapshot
    }
    pub fn variable(&self) -> VariableId {
        self.variable
    }
    pub fn old(&self) -> Option<&Value> {
        self.old.value.as_ref()
    }
    pub fn new_value(&self) -> Option<&Value> {
        self.new.value.as_ref()
    }
    pub fn written(&self) -> bool {
        self.written
    }
    pub fn exchanged(&self) -> Option<bool> {
        self.exchanged
    }
    /// None means the host has no language Eq implementation for this shape.
    pub fn changed(&self) -> Option<bool> {
        self.changed
    }
}
pub struct Workspace {
    current: WorkspaceSnapshot,
    limits: WorkspaceLimits,
}
impl std::fmt::Debug for Workspace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Workspace")
            .field("current", &self.current)
            .field("retention", &self.meter())
            .finish()
    }
}
fn hold(budget: &Arc<Budget>, bytes: usize) -> Result<Tally, WorkspaceError> {
    let mut hold = Tally::new(Some(budget.clone()), 0);
    if !hold.try_set(bytes) {
        return Err(WorkspaceError::Limit);
    }
    Ok(hold)
}
fn check_domains(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
) -> Result<(), WorkspaceError> {
    if !orange_model::validation::validate_body_domains(document, registry.origin())
        .unsupported_bodies
        .is_empty()
    {
        return Err(WorkspaceError::DomainUnsupported);
    }
    Ok(())
}
fn charge(shape: &ValueShape, value: &Value, limit: usize) -> Result<usize, WorkspaceError> {
    shape
        .checked_charge(value, limit)
        .map_err(|error| match error {
            ValueCheckError::ResourceLimit => WorkspaceError::Limit,
            _ => WorkspaceError::Contract,
        })
}
fn root_bytes(count: usize) -> Result<usize, WorkspaceError> {
    count
        .checked_mul(std::mem::size_of::<(VariableId, Arc<Cell>)>())
        .and_then(|n| n.checked_add(std::mem::size_of::<SnapshotData>()))
        .ok_or(WorkspaceError::Limit)
}
impl Workspace {
    pub fn new(
        document: &GraphDocument,
        registry: Arc<RegistrySnapshot>,
        limits: WorkspaceLimits,
    ) -> Result<Self, WorkspaceError> {
        Self::with_values(document, registry, limits, None)
    }
    pub(crate) fn with_values(
        document: &GraphDocument,
        registry: Arc<RegistrySnapshot>,
        limits: WorkspaceLimits,
        restored: Option<&crate::checkpoint::Values>,
    ) -> Result<Self, WorkspaceError> {
        check_domains(document, &registry)?;
        let budget = Budget::new(limits.max_retained_bytes);
        let (variables, values, value_bytes, schema_bytes) =
            schema::prepare(document, &registry, limits, &budget, restored)?;
        let id = identity()?;
        let schema = Arc::new(Schema {
            id,
            variables,
            registry,
            budget: budget.clone(),
            _hold: hold(&budget, schema_bytes)?,
        });
        let root = hold(&budget, root_bytes(values.len())?)?;
        Ok(Self {
            current: WorkspaceSnapshot(Arc::new(SnapshotData {
                schema,
                version: 0,
                values,
                value_bytes,
                _hold: root,
            })),
            limits,
        })
    }
    pub fn snapshot(&self) -> WorkspaceSnapshot {
        self.current.clone()
    }
    pub fn meter(&self) -> WorkspaceMeter {
        self.current.meter()
    }
    pub fn set(
        &mut self,
        id: VariableId,
        value: &Value,
    ) -> Result<WorkspaceCommit, WorkspaceError> {
        self.publish(id, value, true, None)
    }
    pub fn update(
        &mut self,
        id: VariableId,
        operation: UpdateOperation,
        operands: &[Value],
    ) -> Result<WorkspaceCommit, WorkspaceError> {
        let metadata = self
            .current
            .0
            .schema
            .variables
            .get(&id)
            .ok_or(WorkspaceError::UnknownVariable)?;
        let cas = matches!(operation, UpdateOperation::CompareAndSet);
        if operands.len() != if cas { 2 } else { 1 } || (cas && !metadata.shape.host_eq()) {
            return Err(WorkspaceError::Contract);
        }
        if matches!(
            operation,
            UpdateOperation::Numeric {
                operation: orange_model::scalar::Operation::Multiply
                    | orange_model::scalar::Operation::Divide
                    | orange_model::scalar::Operation::Not
                    | orange_model::scalar::Operation::Xor,
                ..
            }
        ) {
            return Err(WorkspaceError::Contract);
        }
        for value in operands {
            charge(&metadata.shape, value, self.limits.max_value_bytes)?;
        }
        let old = self.current.get(id)?.ok_or(WorkspaceError::Uninitialized)?;
        let (value, written) = crate::run::workspace_update(
            &metadata.shape,
            operation,
            old,
            &operands[0],
            operands.get(1),
            self.limits.max_value_bytes,
        )
        .map_err(WorkspaceError::Failed)?;
        self.publish(id, &value, written, cas.then_some(written))
    }
    fn publish(
        &mut self,
        id: VariableId,
        value: &Value,
        written: bool,
        exchanged: Option<bool>,
    ) -> Result<WorkspaceCommit, WorkspaceError> {
        let schema = &self.current.0.schema;
        let metadata = schema
            .variables
            .get(&id)
            .ok_or(WorkspaceError::UnknownVariable)?;
        let bytes = charge(&metadata.shape, value, self.limits.max_value_bytes)?;
        let old = self
            .current
            .0
            .values
            .get(&id)
            .ok_or(WorkspaceError::UnknownVariable)?
            .clone();
        let changed = if !written {
            Some(false)
        } else if old.value.is_none() {
            Some(true)
        } else if metadata.shape.host_eq() {
            metadata
                .shape
                .equal_values(old.value.as_ref().ok_or(WorkspaceError::Contract)?, value)
                .map(|same| !same)
        } else {
            None
        };
        if !written {
            return Ok(WorkspaceCommit {
                snapshot: self.current.clone(),
                variable: id,
                new: old.clone(),
                old,
                written,
                exchanged,
                changed,
            });
        }
        let value_bytes = self
            .current
            .0
            .value_bytes
            .checked_sub(old.bytes)
            .and_then(|n| n.checked_add(bytes))
            .filter(|n| *n <= self.limits.max_state_bytes)
            .ok_or(WorkspaceError::Limit)?;
        let version = self
            .current
            .version()
            .checked_add(1)
            .ok_or(WorkspaceError::IdentityExhausted)?;
        // Retain the old generation through staging. This conservative admission
        // also accounts for snapshots/receipts held by another host or Run.
        let new_hold = hold(
            &schema.budget,
            bytes
                .checked_add(std::mem::size_of::<Cell>())
                .ok_or(WorkspaceError::Limit)?,
        )?;
        let root_hold = hold(&schema.budget, root_bytes(self.current.0.values.len())?)?;
        let new = Arc::new(Cell {
            value: Some(value.clone()),
            bytes,
            _hold: new_hold,
        });
        let mut values = self.current.0.values.clone();
        values.insert(id, new.clone());
        let snapshot = WorkspaceSnapshot(Arc::new(SnapshotData {
            schema: schema.clone(),
            version,
            values,
            value_bytes,
            _hold: root_hold,
        }));
        self.current = snapshot.clone();
        Ok(WorkspaceCommit {
            snapshot,
            variable: id,
            old,
            new,
            written,
            exchanged,
            changed,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exhausted_version_never_wraps_or_publishes_new_values() {
        let document=orange_model::from_json(r#"{"format":"orange.graph","format_version":1,"graph_id":"20000000-0000-4000-8000-000000000001","revision":0,"nodes":[],"edges":[],"variables":[{"id":"10000000-0000-4000-8000-000000000001","name":"v","type":"core/u64@1","scope":"workspace","init":{"type":"core/u64@1","value":"1"}}]}"#).unwrap();
        let registry = orange_model::catalog::RegistryBuilder::core(
            orange_model::validation::HostOrigin::new(
                "0.0.0",
                format!("sha256:{}", "a".repeat(64)),
            )
            .unwrap(),
        )
        .build()
        .unwrap();
        let mut workspace =
            Workspace::new(&document, registry, WorkspaceLimits::default()).unwrap();
        Arc::get_mut(&mut workspace.current.0).unwrap().version = u64::MAX;
        let before = workspace.meter().retention();
        let id = "10000000-0000-4000-8000-000000000001".parse().unwrap();
        assert_eq!(
            workspace.set(id, &Value::U64(2)).unwrap_err(),
            WorkspaceError::IdentityExhausted
        );
        assert_eq!(workspace.snapshot().version(), u64::MAX);
        assert_eq!(workspace.snapshot().get(id).unwrap(), Some(&Value::U64(1)));
        assert_eq!(workspace.meter().retention(), before);
    }
}
