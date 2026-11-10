//! Authenticated graph commands for one host-owned workspace actor.
use super::*;
use crate::{Workspace, WorkspaceCommit, WorkspaceError, WorkspaceId, WorkspaceSnapshot};
use orange_compiler::SharedWrite;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SharedError {
    #[error("The command or snapshot belongs to another workspace.")]
    WrongWorkspace,
    #[error("The command belongs to another run.")]
    WrongRun,
    #[error("The shared command has retired.")]
    Stale,
    #[error("The run is not accepting workspace input.")]
    NotRunning,
    #[error("The workspace input violates its checked contract.")]
    Contract,
    #[error("The workspace input exceeds a retained-value limit.")]
    Limit,
    #[error("An older workspace snapshot cannot replace a newer one.")]
    SnapshotRegression,
}
impl SharedError {
    pub const fn message_key(self) -> &'static str {
        match self {
            Self::WrongWorkspace => "core/shared-wrong-workspace",
            Self::WrongRun => "core/shared-wrong-run",
            Self::Stale => "core/shared-stale",
            Self::NotRunning => "core/shared-not-running",
            Self::Contract => "core/shared-contract",
            Self::Limit => "core/shared-limit",
            Self::SnapshotRegression => "core/shared-snapshot-regression",
        }
    }
}
impl From<EffectError> for SharedError {
    fn from(e: EffectError) -> Self {
        match e {
            EffectError::WrongRun => Self::WrongRun,
            EffectError::Stale => Self::Stale,
            EffectError::NotRunning => Self::NotRunning,
            EffectError::Contract => Self::Contract,
            EffectError::Limit => Self::Limit,
        }
    }
}
/// Refusal retains the original single-use input so the host can retry or reject it.
#[derive(Debug)]
pub struct SharedRefused<T> {
    pub error: SharedError,
    pub value: T,
}
/// Safe downstream code cannot construct or clone a shared mutation command.
///
/// ```compile_fail
/// fn duplicate(request: orange_runtime::SharedRequest) { let _ = request.clone(); }
/// ```
pub struct SharedRequest(Box<RequestData>);
pub(super) struct RequestData {
    id: EffectId,
    node: NodeId,
    workspace: WorkspaceId,
    write: SharedWrite,
    inputs: Arc<[Value]>,
    lease: EffectLease,
    // Host-held commands remain charged even after cancellation removes the pending strand.
    _hold: Tally,
}
impl std::fmt::Debug for SharedRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedRequest")
            .field("id", &self.0.id)
            .field("node", &self.0.node)
            .field("workspace", &self.0.workspace)
            .field("variable", &self.0.write.variable)
            .field("input_count", &self.0.inputs.len())
            .field("lease", &self.0.lease)
            .finish()
    }
}
impl SharedRequest {
    pub fn node(&self) -> NodeId {
        self.0.node
    }
    pub fn workspace(&self) -> WorkspaceId {
        self.0.workspace
    }
    pub fn variable(&self) -> VariableId {
        self.0.write.variable
    }
    pub fn operation(&self) -> Option<UpdateOperation> {
        self.0.write.operation
    }
    pub fn inputs(&self) -> &[Value] {
        &self.0.inputs
    }
    pub fn lease(&self) -> &EffectLease {
        &self.0.lease
    }
}
/// Only the owning Workspace can mint a successful mutation receipt.
/// Cancellation may retire the graph continuation after the external commit;
/// it never rolls that committed state back.
pub struct SharedReceipt {
    request: SharedRequest,
    outcome: Result<WorkspaceCommit, WorkspaceError>,
}
impl std::fmt::Debug for SharedReceipt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedReceipt")
            .field("request", &self.request)
            .field("outcome", &self.outcome)
            .finish()
    }
}
impl SharedReceipt {
    pub fn commit(&self) -> Result<&WorkspaceCommit, WorkspaceError> {
        self.outcome.as_ref().map_err(Clone::clone)
    }
}
pub(super) struct Attachment {
    pub snapshot: WorkspaceSnapshot,
    _hold: Tally,
}
impl Attachment {
    pub fn prepare(
        plan: &ControlPlan,
        snapshot: &WorkspaceSnapshot,
        budget: &Arc<Budget>,
        limit: usize,
    ) -> Result<Self, SharedError> {
        let mut bytes = 0usize;
        for v in plan
            .variables()
            .iter()
            .filter(|v| v.scope() == VarScope::Workspace)
        {
            if !snapshot.matches(v.id(), v.ty()) {
                return Err(SharedError::Contract);
            }
            bytes = bytes
                .checked_add(
                    match snapshot.get(v.id()).map_err(|_| SharedError::Contract)? {
                        Some(value) => v
                            .checked_charge(value, limit)
                            .map_err(|_| SharedError::Limit)?,
                        None => 16,
                    },
                )
                .ok_or(SharedError::Limit)?;
        }
        if bytes > limit {
            return Err(SharedError::Limit);
        }
        let mut hold = Tally::new(Some(budget.clone()), 0);
        if !hold.try_set(bytes) {
            return Err(SharedError::Limit);
        }
        Ok(Self {
            snapshot: snapshot.clone(),
            _hold: hold,
        })
    }
}
impl Workspace {
    pub fn apply_shared(
        &mut self,
        request: SharedRequest,
    ) -> Result<SharedReceipt, SharedRefused<SharedRequest>> {
        let error = if request.0.workspace != self.snapshot().id() {
            Some(SharedError::WrongWorkspace)
        } else if request.0.lease.is_finished() {
            Some(SharedError::Stale)
        } else {
            None
        };
        if let Some(error) = error {
            return Err(SharedRefused {
                error,
                value: request,
            });
        }
        let outcome = match request.0.write.operation {
            Some(operation) => self.update(request.0.write.variable, operation, &request.0.inputs),
            None => match request.0.inputs.as_ref() {
                [value] => self.set(request.0.write.variable, value),
                _ => Err(WorkspaceError::Contract),
            },
        };
        if matches!(outcome, Err(WorkspaceError::Limit)) {
            return Err(SharedRefused {
                error: SharedError::Limit,
                value: request,
            });
        }
        Ok(SharedReceipt { request, outcome })
    }
}
fn fault(error: WorkspaceError) -> KernelFault {
    match error {
        WorkspaceError::Failed(f) => f,
        WorkspaceError::Uninitialized => KernelFault {
            code: orange_compiler::FaultText::Static("core/UninitializedVariable"),
            message_key: orange_compiler::FaultText::Static("core/uninitialized-variable"),
            message: "The variable has not been initialized.",
        },
        _ => KernelFault {
            code: orange_compiler::FaultText::Static("core/WorkspaceFailure"),
            message_key: error.message_key().to_owned().into(),
            message: "The workspace operation failed.",
        },
    }
}
impl Run {
    pub fn workspace_snapshot(&self) -> Option<&WorkspaceSnapshot> {
        self.workspace.as_ref().map(|a| &a.snapshot)
    }
    fn prepare_workspace(&self, snapshot: &WorkspaceSnapshot) -> Result<Attachment, SharedError> {
        if self.pump_open || !self.accepts_results() {
            return Err(SharedError::NotRunning);
        }
        let old = self.workspace.as_ref().ok_or(SharedError::WrongWorkspace)?;
        if old.snapshot.id() != snapshot.id() {
            return Err(SharedError::WrongWorkspace);
        }
        if old.snapshot.version() > snapshot.version() {
            return Err(SharedError::SnapshotRegression);
        }
        Attachment::prepare(
            self.plan.as_ref().ok_or(SharedError::NotRunning)?,
            snapshot,
            &self.budget,
            self.options.limits.max_state_bytes,
        )
    }
    /// Stage a newer immutable publication at an actor boundary. An already-open
    /// consumption epoch retains its previously frozen inputs.
    pub fn publish_workspace(&mut self, snapshot: &WorkspaceSnapshot) -> Result<(), SharedError> {
        let attachment = self.prepare_workspace(snapshot)?;
        self.workspace = Some(attachment);
        Ok(())
    }
    pub fn take_shared(&mut self) -> Result<Option<SharedRequest>, SharedError> {
        let Some(hold) = self.reserve_shared_command()? else {
            return Ok(None);
        };
        let workspace = self
            .workspace
            .as_ref()
            .ok_or(SharedError::WrongWorkspace)?
            .snapshot
            .id();
        let request = self.take_effect_kind(true).ok_or(SharedError::Contract)?;
        let write = *request
            .context
            .downcast_ref::<SharedWrite>()
            .ok_or(SharedError::Contract)?;
        Ok(Some(SharedRequest(Box::new(RequestData {
            id: request.id,
            node: request.node,
            workspace,
            write,
            inputs: request.inputs,
            lease: request.lease,
            _hold: hold,
        }))))
    }
    pub fn fail_shared(
        &mut self,
        request: SharedRequest,
        fault: KernelFault,
    ) -> Result<(), SharedRefused<SharedRequest>> {
        let result = self
            .complete_effect_kind(request.0.id, Err(fault), true)
            .map_err(SharedError::from);
        result.map_err(|error| SharedRefused {
            error,
            value: request,
        })
    }
    /// Retire a committed receipt with a graph fault when its successful result
    /// cannot be admitted. The external mutation stays committed; the receipt
    /// never becomes another executable request.
    pub fn fail_shared_receipt(
        &mut self,
        receipt: SharedReceipt,
        fault: KernelFault,
    ) -> Result<(), SharedRefused<SharedReceipt>> {
        let result = self
            .complete_effect_kind(receipt.request.0.id, Err(fault), true)
            .map_err(SharedError::from);
        result.map_err(|error| SharedRefused {
            error,
            value: receipt,
        })
    }
    pub fn complete_shared(
        &mut self,
        receipt: SharedReceipt,
    ) -> Result<(), SharedRefused<SharedReceipt>> {
        let result = (|| {
            // A later publication already installed by the host must never regress.
            let attachment = if let Ok(commit) = &receipt.outcome {
                let current = self.workspace.as_ref().ok_or(SharedError::WrongWorkspace)?;
                if current.snapshot.id() != commit.snapshot().id() {
                    return Err(SharedError::WrongWorkspace);
                }
                if current.snapshot.version() < commit.snapshot().version() {
                    Some(self.prepare_workspace(commit.snapshot())?)
                } else {
                    None
                }
            } else {
                None
            };
            let mut values = Vec::new();
            let outcome = match &receipt.outcome {
                Ok(commit) => {
                    if receipt.request.0.write.operation.is_some() {
                        values.push(commit.old().ok_or(SharedError::Contract)?.clone());
                        values.push(commit.new_value().ok_or(SharedError::Contract)?.clone());
                        if let Some(exchanged) = commit.exchanged() {
                            values.push(Value::Bool(exchanged));
                        }
                    }
                    Ok(values.as_slice())
                }
                Err(error) => Err(fault(error.clone())),
            };
            self.complete_effect_kind(receipt.request.0.id, outcome, true)?;
            // Completion is only queued. Install before any continuation can run.
            if let Some(attachment) = attachment {
                self.workspace = Some(attachment);
            }
            Ok(())
        })();
        result.map_err(|error| SharedRefused {
            error,
            value: receipt,
        })
    }
}
