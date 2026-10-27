//! Lexical immutable iteration frames and automatic activation-budget release.
use super::*;
use std::sync::atomic::AtomicUsize;
struct Hold {
    bytes: usize,
    total: Arc<AtomicUsize>,
}
impl Hold {
    fn new(
        shared: &ActivationState,
        bytes: usize,
        credit: usize,
        limit: usize,
    ) -> Result<Self, RunError> {
        let reserved = shared
            .owned_bytes
            .load(Ordering::Relaxed)
            .checked_add(bytes)
            .ok_or(RunError::ResourceLimit)?;
        reserved
            .checked_sub(credit)
            .and_then(|n| n.checked_add(shared.local_bytes))
            .and_then(|n| n.checked_add(shared.retained_bytes))
            .filter(|n| *n <= limit)
            .ok_or(RunError::ResourceLimit)?;
        shared.owned_bytes.fetch_add(bytes, Ordering::Relaxed);
        Ok(Self {
            bytes,
            total: shared.owned_bytes.clone(),
        })
    }
}
impl Drop for Hold {
    fn drop(&mut self) {
        let old = self.total.fetch_sub(self.bytes, Ordering::Relaxed);
        debug_assert!(old >= self.bytes);
    }
}
pub(super) struct IterationFrame {
    owner: ControlIx,
    parent: Option<Arc<IterationFrame>>,
    index: i64,
    first: Value,
    second: Option<Value>,
    hold: Hold,
}
impl Drop for IterationFrame {
    fn drop(&mut self) {
        let mut parent = self.parent.take();
        while let Some(frame) = parent {
            match Arc::try_unwrap(frame) {
                Ok(mut frame) => parent = frame.parent.take(),
                Err(_shared) => break,
            }
        }
    }
}
impl IterationFrame {
    pub(super) fn find(mut context: Option<&Self>, owner: ControlIx) -> Option<&Self> {
        while let Some(frame) = context {
            if frame.owner == owner {
                return Some(frame);
            }
            context = frame.parent.as_deref();
        }
        None
    }
    pub(super) fn value(&self, projection: IterationProjection) -> Option<Value> {
        Some(match projection {
            IterationProjection::Index => Value::I64(self.index),
            IterationProjection::Element | IterationProjection::Key => self.first.clone(),
            IterationProjection::Value => self.second.as_ref()?.clone(),
        })
    }
}
pub(super) struct ForEachCursor {
    owner: ControlIx,
    next: usize,
    collection: Value,
    _hold: Hold,
    frame: Arc<IterationFrame>,
}
impl ForEachCursor {
    pub(super) fn context(&self, owner: ControlIx) -> Result<Arc<IterationFrame>, RunError> {
        if self.owner != owner {
            return Err(RunError::InvalidState);
        }
        Ok(self.frame.clone())
    }
}
pub(super) fn collection_len(value: &Value) -> Option<usize> {
    match value {
        Value::List(items) => Some(items.len()),
        Value::Map(map) => Some(map.entries.len()),
        _ => None,
    }
}
struct FrameSpec<'a> {
    owner: ControlIx,
    collection: &'a Value,
    parent: Option<Arc<IterationFrame>>,
    index: usize,
    credit: usize,
}
fn frame(
    plan: &ControlPlan,
    shared: &ActivationState,
    spec: FrameSpec<'_>,
    limit: usize,
) -> Result<Arc<IterationFrame>, RunError> {
    let FrameSpec {
        owner,
        collection,
        parent,
        index,
        credit,
    } = spec;
    let index = i64::try_from(index).map_err(|_| RunError::ResourceLimit)?;
    let (first, second) = match collection {
        Value::List(items) => (
            items.get(index as usize).ok_or(RunError::InvalidState)?,
            None,
        ),
        Value::Map(map) => {
            let (k, v) = map
                .entries
                .get(index as usize)
                .ok_or(RunError::InvalidState)?;
            (k.clone(), Some(v.clone()))
        }
        _ => return Err(RunError::InvalidState),
    };
    let ControlOperation::ForEach { items, .. } = plan.nodes()[owner.index()].operation() else {
        return Err(RunError::InvalidState);
    };
    let ty = &plan.data().slot_types()[plan.data().exports()[*items].slot.index()];
    let (a, b) = match ty {
        orange_compiler::PlanType::List(inner) => (
            inner
                .checked_charge(&first, limit)
                .map_err(|_| RunError::ResourceLimit)?,
            0,
        ),
        orange_compiler::PlanType::Map(key, value) => (
            key.checked_charge(&first, limit)
                .map_err(|_| RunError::ResourceLimit)?,
            value
                .checked_charge(second.as_ref().ok_or(RunError::InvalidState)?, limit)
                .map_err(|_| RunError::ResourceLimit)?,
        ),
        _ => return Err(RunError::InvalidState),
    };
    let hold = Hold::new(
        shared,
        a.checked_add(b)
            .and_then(|n| n.checked_add(16))
            .ok_or(RunError::ResourceLimit)?,
        credit,
        limit,
    )?;
    Ok(Arc::new(IterationFrame {
        owner,
        parent,
        index,
        first,
        second,
        hold,
    }))
}
impl Run {
    pub(super) fn begin_iteration(
        &self,
        plan: &ControlPlan,
        shared: &mut ActivationState,
        owner: ControlIx,
        root: usize,
        collection: Value,
        parent: Option<Arc<IterationFrame>>,
    ) -> Result<ForEachCursor, RunError> {
        let bytes = plan
            .data()
            .checked_value_charge(
                plan.data().exports()[root].slot,
                &collection,
                self.options.limits.max_state_bytes,
            )
            .map_err(|_| RunError::ResourceLimit)?;
        let hold = Hold::new(shared, bytes, 0, self.options.limits.max_state_bytes)?;
        let current = frame(
            plan,
            shared,
            FrameSpec {
                owner,
                collection: &collection,
                parent,
                index: 0,
                credit: 0,
            },
            self.options.limits.max_state_bytes,
        )?;
        Ok(ForEachCursor {
            owner,
            next: 1,
            collection,
            _hold: hold,
            frame: current,
        })
    }
    pub(super) fn resume_iteration(
        &mut self,
        plan: &ControlPlan,
        activation: &mut Strand,
        shared: &mut ActivationState,
        mut cursor: Box<ForEachCursor>,
    ) -> Result<(), RunError> {
        let owner = cursor.owner;
        let node = &plan.nodes()[owner.index()];
        let ControlOperation::ForEach {
            body, completed, ..
        } = node.operation()
        else {
            return Err(RunError::InvalidState);
        };
        if cursor.next < collection_len(&cursor.collection).ok_or(RunError::InvalidState)? {
            let credit = if Arc::strong_count(&cursor.frame) == 1 {
                cursor.frame.hold.bytes
            } else {
                0
            };
            let current = frame(
                plan,
                shared,
                FrameSpec {
                    owner,
                    collection: &cursor.collection,
                    parent: activation.iteration_context.clone(),
                    index: cursor.next,
                    credit,
                },
                self.options.limits.max_state_bytes,
            )?;
            cursor.frame = current;
            cursor.next += 1;
            activation.stack.push(Cursor::ForEach(cursor));
            activation.enter_body = Some(Some(owner));
            activation.pc = *body;
            activation.yielded = true;
            self.record(RunEvent {
                activation: activation.sequence,
                node: node.node(),
                outcome: ControlOutcome::Body,
                fault: None,
            });
        } else {
            activation.pc = *completed;
            self.record(RunEvent {
                activation: activation.sequence,
                node: node.node(),
                outcome: ControlOutcome::Completed,
                fault: None,
            });
        }
        Ok(())
    }
}
