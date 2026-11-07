//! Run-owned resources (C08a): a bounded, generation-checked table of host objects that
//! the run closes exactly once. Graph handle values and activation owners are later work.
use super::{Run, RunState};
use orange_model::id::TypeExpr;
use orange_model::type_term::TypeTerm;
use orange_model::types::TypeKind;
use orange_model::value::HandleId;
use std::any::Any;
use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use thiserror::Error;

/// A host object owned by one run. The run calls `close` exactly once: when the host
/// closes its handle, when the run reaches `Stopped`, or when the run is dropped.
/// `close` runs on the host's call stack, never inside graph execution. `Send + Sync`
/// keeps `Run` movable and shareable across host threads.
pub trait RunResource: Any + Send + Sync {
    fn close(self: Box<Self>, reason: CloseReason);
}

/// Why a run closed one of its resources.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    /// The host closed the handle.
    Closed,
    /// The run reached `Stopped` without a fault.
    RunStopped,
    /// The run reached `Stopped` after a fault or an interrupted pump.
    RunFaulted,
    /// The run was dropped while it still owned the resource.
    RunDropped,
}

/// Opaque handle to a resource of one run. A closed handle stays stale for good: its
/// slot generation advances and a slot whose generation would wrap is retired.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ResourceHandle {
    run: u64,
    id: HandleId,
}
impl ResourceHandle {
    /// Table identity; its `kind` is the run-local index of the handle-kind type.
    pub const fn id(self) -> HandleId {
        self.id
    }
}
impl std::fmt::Debug for ResourceHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResourceHandle")
            .field("run", &self.run)
            .field("index", &self.id.index)
            .field("generation", &self.id.generation)
            .field("kind", &self.id.kind)
            .finish()
    }
}

/// Bounded, payload-free refusal from the resource boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ResourceError {
    #[error("The resource handle belongs to another run.")]
    WrongRun,
    #[error("The run is not running and cannot acquire a resource.")]
    NotRunning,
    #[error("The type is not a registered handle kind in the run's registry.")]
    UnknownKind,
    #[error("The run has reached its live resource limit.")]
    ResourceLimit,
    #[error("The run's resource identity space is exhausted.")]
    IdentityExhausted,
    #[error("The resource handle is closed or was never issued by this run.")]
    StaleHandle,
    #[error("The resource is not of the requested kind.")]
    WrongKind,
}
impl ResourceError {
    pub const fn message_key(self) -> &'static str {
        match self {
            Self::WrongRun => "core/resource-wrong-run",
            Self::NotRunning => "core/resource-not-running",
            Self::UnknownKind => "core/resource-unknown-kind",
            Self::ResourceLimit => "core/resource-limit",
            Self::IdentityExhausted => "core/resource-identity-exhausted",
            Self::StaleHandle => "core/resource-stale",
            Self::WrongKind => "core/resource-wrong-kind",
        }
    }
}

/// A refused acquisition. The run never took ownership, so the host still owns the
/// resource and decides how to release it.
pub struct AcquireRefused {
    pub error: ResourceError,
    pub resource: Box<dyn RunResource>,
}
impl std::fmt::Debug for AcquireRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AcquireRefused")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

struct Entry {
    kind: u32,
    /// Acquisition order; terminal closing runs newest first.
    seq: u64,
    resource: Box<dyn RunResource>,
}
struct Slot {
    generation: u32,
    entry: Option<Entry>,
}

/// Generational slab. Generations start at 1, so a zeroed handle is never live.
#[derive(Default)]
pub(super) struct Resources {
    slots: Vec<Slot>,
    free: Vec<u32>,
    live: usize,
    kinds: Vec<TypeExpr>,
    kind_index: HashMap<TypeExpr, u32>,
    next_seq: u64,
}
impl Resources {
    pub(super) fn len(&self) -> usize {
        self.live
    }
    fn kind(&mut self, kind: &TypeExpr) -> Option<u32> {
        if let Some(&index) = self.kind_index.get(kind) {
            return Some(index);
        }
        let index = u32::try_from(self.kinds.len()).ok()?;
        self.kinds.push(kind.clone());
        self.kind_index.insert(kind.clone(), index);
        Some(index)
    }
    fn insert(
        &mut self,
        kind: &TypeExpr,
        resource: Box<dyn RunResource>,
        limit: usize,
    ) -> Result<HandleId, AcquireRefused> {
        let refuse = |error, resource| Err(AcquireRefused { error, resource });
        if self.live >= limit {
            return refuse(ResourceError::ResourceLimit, resource);
        }
        let (Some(kind), Some(seq)) = (self.kind(kind), self.next_seq.checked_add(1)) else {
            return refuse(ResourceError::IdentityExhausted, resource);
        };
        let index = match self.free.pop() {
            Some(index) => index,
            None => match u32::try_from(self.slots.len()) {
                Ok(index) => {
                    self.slots.push(Slot {
                        generation: 1,
                        entry: None,
                    });
                    index
                }
                Err(_) => return refuse(ResourceError::IdentityExhausted, resource),
            },
        };
        self.next_seq = seq;
        let slot = &mut self.slots[index as usize];
        slot.entry = Some(Entry {
            kind,
            seq,
            resource,
        });
        self.live += 1;
        Ok(HandleId {
            index,
            generation: slot.generation,
            kind,
        })
    }
    fn contains(&self, id: HandleId) -> bool {
        self.slots
            .get(id.index as usize)
            .filter(|slot| slot.generation == id.generation)
            .and_then(|slot| slot.entry.as_ref())
            .is_some_and(|entry| entry.kind == id.kind)
    }
    fn entry(&mut self, id: HandleId) -> Result<&mut Entry, ResourceError> {
        self.slots
            .get_mut(id.index as usize)
            .filter(|slot| slot.generation == id.generation)
            .and_then(|slot| slot.entry.as_mut())
            .filter(|entry| entry.kind == id.kind)
            .ok_or(ResourceError::StaleHandle)
    }
    fn remove(&mut self, id: HandleId) -> Result<Box<dyn RunResource>, ResourceError> {
        self.entry(id)?;
        Ok(self.release(id.index).resource)
    }
    /// Empties a live slot. Its generation advances; a slot at the last generation is
    /// retired instead of reused, so no handle value is ever issued twice.
    fn release(&mut self, index: u32) -> Entry {
        let slot = &mut self.slots[index as usize];
        let entry = slot.entry.take().unwrap_or_else(|| unreachable!());
        self.live -= 1;
        if let Some(next) = slot.generation.checked_add(1) {
            slot.generation = next;
            self.free.push(index);
        }
        entry
    }
    /// Closes every live resource newest first, each once. A panicking closer does not
    /// skip the others; the first panic resumes after all of them ran.
    pub(super) fn close_all(&mut self, reason: CloseReason) {
        if self.live == 0 {
            return;
        }
        let live: Vec<u32> = (0..self.slots.len() as u32)
            .filter(|&i| self.slots[i as usize].entry.is_some())
            .collect();
        let mut taken: Vec<Entry> = live.into_iter().map(|i| self.release(i)).collect();
        taken.sort_unstable_by_key(|entry| std::cmp::Reverse(entry.seq));
        let mut panic = None;
        for entry in taken {
            if let Err(payload) = catch_unwind(AssertUnwindSafe(|| entry.resource.close(reason))) {
                panic.get_or_insert(payload);
            }
        }
        if let Some(payload) = panic
            && !std::thread::panicking()
        {
            resume_unwind(payload);
        }
    }
}
impl Drop for Resources {
    fn drop(&mut self) {
        self.close_all(CloseReason::RunDropped);
    }
}

impl Run {
    /// Take ownership of a host resource of a registered handle kind. Only a running
    /// run acquires; a refusal hands the resource back unclosed.
    pub fn acquire_resource(
        &mut self,
        kind: &TypeExpr,
        resource: Box<dyn RunResource>,
    ) -> Result<ResourceHandle, AcquireRefused> {
        if self.pump_open || self.state != RunState::Running {
            return Err(AcquireRefused {
                error: ResourceError::NotRunning,
                resource,
            });
        }
        let registered = self.plan.as_ref().is_some_and(|plan| {
            TypeTerm::parse(kind).is_ok_and(|term| {
                let types = plan.data().snapshot().types();
                types.validate(&term).is_ok() && types.kind_of(&term) == Ok(TypeKind::HandleKind)
            })
        });
        if !registered {
            return Err(AcquireRefused {
                error: ResourceError::UnknownKind,
                resource,
            });
        }
        let id = self
            .resources
            .insert(kind, resource, self.options.limits.max_resources)?;
        Ok(ResourceHandle { run: self.id, id })
    }

    /// Close one resource now. Allowed in every state while the handle is live.
    pub fn close_resource(&mut self, handle: ResourceHandle) -> Result<(), ResourceError> {
        if handle.run != self.id {
            return Err(ResourceError::WrongRun);
        }
        self.resources.remove(handle.id)?.close(CloseReason::Closed);
        Ok(())
    }

    /// The registered handle kind of a live resource.
    pub fn resource_kind(&self, handle: ResourceHandle) -> Result<&TypeExpr, ResourceError> {
        if handle.run != self.id {
            return Err(ResourceError::WrongRun);
        }
        if !self.resources.contains(handle.id) {
            return Err(ResourceError::StaleHandle);
        }
        Ok(&self.resources.kinds[handle.id.kind as usize])
    }

    /// Borrow a live resource as its concrete host type, checking the handle kind too.
    pub fn resource_mut<T: RunResource>(
        &mut self,
        handle: ResourceHandle,
        kind: &TypeExpr,
    ) -> Result<&mut T, ResourceError> {
        if handle.run != self.id {
            return Err(ResourceError::WrongRun);
        }
        let wanted = self.resources.kind_index.get(kind).copied();
        let entry = self.resources.entry(handle.id)?;
        if Some(entry.kind) != wanted {
            return Err(ResourceError::WrongKind);
        }
        let resource: &mut dyn Any = entry.resource.as_mut();
        resource.downcast_mut().ok_or(ResourceError::WrongKind)
    }

    /// Terminal closing once the run reaches `Stopped`.
    pub(super) fn close_resources(&mut self) {
        let reason = if self.last_error.is_some() || self.transitions.contains(&RunState::Faulted) {
            CloseReason::RunFaulted
        } else {
            CloseReason::RunStopped
        };
        self.resources.close_all(reason);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct Probe(Arc<Mutex<Vec<(u8, CloseReason)>>>, u8);
    impl RunResource for Probe {
        fn close(self: Box<Self>, reason: CloseReason) {
            self.0.lock().unwrap().push((self.1, reason));
        }
    }
    fn kind() -> TypeExpr {
        "acme/session@1".parse().unwrap()
    }

    #[test]
    fn a_slot_at_the_last_generation_is_retired_not_reused() {
        let log = Arc::new(Mutex::new(vec![]));
        let mut table = Resources::default();
        let first = table
            .insert(&kind(), Box::new(Probe(log.clone(), 0)), 4)
            .unwrap();
        table.slots[0].generation = u32::MAX;
        let last = HandleId {
            generation: u32::MAX,
            ..first
        };
        table.remove(last).unwrap().close(CloseReason::Closed);
        assert_eq!(table.slots[0].generation, u32::MAX);
        assert!(table.free.is_empty());
        let next = table
            .insert(&kind(), Box::new(Probe(log.clone(), 1)), 4)
            .unwrap();
        assert_eq!((next.index, next.generation), (1, 1));
        assert_eq!(table.remove(last).err(), Some(ResourceError::StaleHandle));
        assert_eq!(table.remove(first).err(), Some(ResourceError::StaleHandle));
    }

    #[test]
    fn closing_all_runs_newest_first_and_survives_a_panicking_closer() {
        struct Panics;
        impl RunResource for Panics {
            fn close(self: Box<Self>, _: CloseReason) {
                panic!("closer failed");
            }
        }
        let log = Arc::new(Mutex::new(vec![]));
        let mut table = Resources::default();
        table
            .insert(&kind(), Box::new(Probe(log.clone(), 0)), 4)
            .unwrap();
        table.insert(&kind(), Box::new(Panics), 4).unwrap();
        table
            .insert(&kind(), Box::new(Probe(log.clone(), 2)), 4)
            .unwrap();
        let unwound = catch_unwind(AssertUnwindSafe(|| {
            table.close_all(CloseReason::RunStopped);
        }));
        assert!(unwound.is_err());
        assert_eq!(
            *log.lock().unwrap(),
            [(2, CloseReason::RunStopped), (0, CloseReason::RunStopped)]
        );
        assert_eq!(table.len(), 0);
    }
}
