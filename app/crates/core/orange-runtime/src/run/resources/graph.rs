//! Issued graph capabilities and dynamic region ownership, using the C08a slab.
use super::*;
use crate::budget::Lease;
use crate::run::*;
use orange_compiler::KernelFault;
use orange_model::Value;

struct GraphToken {
    run: u64,
    id: HandleId,
    stream: Option<Arc<std::sync::atomic::AtomicU8>>,
    _metadata: Lease,
}

/// A refused one-result resource completion. Ownership never escapes the host on
/// refusal: the object is returned unclosed. A still-valid pending effect can be
/// retried after correcting a type or quota refusal.
pub struct EffectResourceRefused {
    pub error: EffectError,
    pub resource: Box<dyn RunResource>,
}
impl std::fmt::Debug for EffectResourceRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EffectResourceRefused")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

impl Resources {
    pub(in crate::run) fn set_owner(
        &mut self,
        id: HandleId,
        owner: Option<RegionId>,
    ) -> Result<(), ResourceError> {
        let old = self.entry(id)?.owner;
        if old == owner {
            return Ok(());
        }
        if let Some(old) = old
            && let Some(items) = self.owners.get_mut(&old)
        {
            items.remove(&id);
            if items.is_empty() {
                self.owners.remove(&old);
            }
        }
        self.entry(id)?.owner = owner;
        if let Some(owner) = owner {
            self.owners.entry(owner).or_default().insert(id);
        }
        Ok(())
    }
    pub(in crate::run) fn reparent_resources(
        &mut self,
        ids: &[RegionId],
        root: RegionId,
    ) -> Result<(), RunError> {
        for &owner in ids {
            if owner == root {
                continue;
            }
            let Some(items) = self.owners.remove(&owner) else {
                continue;
            };
            for id in items {
                self.entry(id).map_err(|_| RunError::InvalidState)?.owner = Some(root);
                self.owners.entry(root).or_default().insert(id);
            }
        }
        Ok(())
    }
    pub(in crate::run) fn close_region_resources(
        &mut self,
        owner: RegionId,
    ) -> Result<usize, RunError> {
        let Some(items) = self.owners.get(&owner) else {
            return Ok(0);
        };
        let mut entries: Vec<_> = items
            .iter()
            .map(|id| {
                self.slots[id.index as usize]
                    .entry
                    .as_ref()
                    .map(|e| (e.seq, *id))
                    .ok_or(RunError::InvalidState)
            })
            .collect::<Result<_, _>>()?;
        entries.sort_unstable_by_key(|(sequence, _)| std::cmp::Reverse(*sequence));
        let count = entries.len();
        let mut resources = Vec::with_capacity(count);
        for (_, id) in entries {
            resources.push(self.remove(id).map_err(|_| RunError::InvalidState)?);
        }
        let mut panic = None;
        for resource in resources {
            if let Err(payload) =
                catch_unwind(AssertUnwindSafe(|| resource.close(CloseReason::ScopeEnded)))
            {
                panic.get_or_insert(payload);
            }
        }
        if let Some(payload) = panic
            && !std::thread::panicking()
        {
            resume_unwind(payload);
        }
        Ok(count)
    }
}
impl Run {
    /// Expose an authentic value for an existing run-owned host resource. Cloning
    /// the value does not transfer ownership or prevent explicit close/revocation.
    pub fn resource_value(&mut self, handle: ResourceHandle) -> Result<Value, ResourceError> {
        let kind = self.resource_kind(handle)?.clone();
        let term = TypeTerm::parse(&kind).map_err(|_| ResourceError::UnknownKind)?;
        if self
            .plan
            .as_ref()
            .is_none_or(|p| p.data().snapshot().types().kind_of(&term) != Ok(TypeKind::HandleKind))
        {
            return Err(ResourceError::UnknownKind);
        }
        self.issue_resource_value(handle, None)
    }
    pub(in crate::run) fn issue_resource_value(
        &mut self,
        handle: ResourceHandle,
        stream: Option<Arc<std::sync::atomic::AtomicU8>>,
    ) -> Result<Value, ResourceError> {
        let kind = self.resource_kind(handle)?.clone();
        let wrap = |value| {
            if stream.is_some() {
                Value::Stream(value)
            } else {
                Value::Handle(value)
            }
        };
        if let Some(value) = self.resources.entry(handle.id)?.value.as_ref() {
            return Ok(wrap(value.clone()));
        }
        let bytes = std::mem::size_of::<HandleValue>()
            .checked_add(std::mem::size_of::<GraphToken>())
            .and_then(|b| b.checked_add(4 * std::mem::size_of::<usize>()))
            .and_then(|b| b.checked_add(kind.as_str().len()))
            .and_then(|b| {
                b.checked_add(if stream.is_some() {
                    3 * std::mem::size_of::<usize>()
                } else {
                    0
                })
            })
            .ok_or(ResourceError::ResourceLimit)?;
        if !self.budget.admits(bytes) {
            return Err(ResourceError::ResourceLimit);
        }
        let mut lease = Lease::new(Some(self.budget.clone()));
        lease.set(bytes);
        let token = Arc::new(GraphToken {
            run: self.id,
            id: handle.id,
            stream: stream.clone(),
            _metadata: lease,
        });
        let value = Arc::new(HandleValue::new(kind, handle.id, token));
        self.resources.entry(handle.id)?.value = Some(value.clone());
        Ok(wrap(value))
    }
    pub(in crate::run) fn issued_handle(
        &self,
        value: &Value,
        kind: &TypeExpr,
    ) -> Result<ResourceHandle, ResourceError> {
        let (value, stream) = match value {
            Value::Handle(value) => (value, false),
            Value::Stream(value) => (value, true),
            _ => return Err(ResourceError::WrongKind),
        };
        let token = value
            .evidence::<GraphToken>()
            .ok_or(ResourceError::Forged)?;
        if token.run != self.id {
            return Err(ResourceError::WrongRun);
        }
        if token.id != value.id() {
            return Err(ResourceError::Forged);
        }
        if token.stream.is_some() != stream {
            return Err(ResourceError::Forged);
        }
        if value.kind() != kind {
            return Err(ResourceError::WrongKind);
        }
        Ok(ResourceHandle::for_run(self.id, token.id))
    }
    /// Resolve a live graph reference for a trusted host adapter. Graph execution
    /// additionally checks the executing region's owner ancestry.
    pub fn resource_handle(
        &self,
        value: &Value,
        kind: &TypeExpr,
    ) -> Result<ResourceHandle, ResourceError> {
        let handle = self.issued_handle(value, kind)?;
        self.resource_kind(handle)?;
        let entry = self
            .resources
            .slots
            .get(handle.id.index as usize)
            .and_then(|s| s.entry.as_ref())
            .ok_or(ResourceError::StaleHandle)?;
        let (Value::Handle(value) | Value::Stream(value)) = value else {
            return Err(ResourceError::WrongKind);
        };
        if !entry
            .value
            .as_ref()
            .is_some_and(|issued| Arc::ptr_eq(issued, value))
        {
            return Err(ResourceError::Forged);
        }
        Ok(handle)
    }
    pub(in crate::run) fn check_graph_resource(
        &self,
        value: &Value,
        region: RegionId,
    ) -> Result<(), ResourceError> {
        let (Value::Handle(value_ref) | Value::Stream(value_ref)) = value else {
            return Ok(());
        };
        let handle = self.resource_handle(value, value_ref.kind())?;
        let owner = self.resources.slots[handle.id.index as usize]
            .entry
            .as_ref()
            .ok_or(ResourceError::StaleHandle)?
            .owner;
        if let Some(owner) = owner
            && !self.regions.is_within(region, owner)
        {
            return Err(ResourceError::WrongOwner);
        }
        Ok(())
    }
    pub(in crate::run) fn close_graph_resource(
        &mut self,
        value: &Value,
        region: RegionId,
    ) -> Result<(), ResourceError> {
        let (Value::Handle(reference) | Value::Stream(reference)) = value else {
            return Err(ResourceError::WrongKind);
        };
        let handle = self.issued_handle(value, reference.kind())?;
        match self.resource_kind(handle) {
            Err(ResourceError::StaleHandle) => return Ok(()),
            Err(error) => return Err(error),
            Ok(_) => {}
        }
        self.check_graph_resource(value, region)?;
        self.close_resource(handle)
    }
    /// Complete an Async whose sole result is a direct `core/handle<K>`. The
    /// resource is acquired by that pending strand's region only after all checks;
    /// every refusal returns it unclosed. A valid pending effect permits retry
    /// after a type/quota refusal; stale identities cannot be revived.
    pub fn complete_effect_resource(
        &mut self,
        id: EffectId,
        kind: &TypeExpr,
        resource: Box<dyn RunResource>,
    ) -> Result<(), EffectResourceRefused> {
        let refuse = |error, resource| Err(EffectResourceRefused { error, resource });
        let region = match self.effect_resource_target(id, kind) {
            Ok(region) => region,
            Err(error) => return refuse(error, resource),
        };
        let identity =
            match self
                .resources
                .insert(kind, resource, self.options.limits.max_resources)
            {
                Ok(id) => id,
                Err(refused) => return refuse(EffectError::Limit, refused.resource),
            };
        let handle = ResourceHandle::for_run(self.id, identity);
        let outcome = (|| {
            self.resources
                .set_owner(identity, Some(region))
                .map_err(|_| EffectError::Contract)?;
            let value = self
                .resource_value(handle)
                .map_err(|_| EffectError::Limit)?;
            self.complete_effect(id, Ok(&[value]))
        })();
        match outcome {
            Ok(()) => Ok(()),
            Err(error) => match self.resources.remove(identity) {
                Ok(resource) => refuse(error, resource),
                Err(_) => unreachable!("unpublished resource remains in its issuing slab"),
            },
        }
    }
    pub(in crate::run) fn graph_stream_handle(
        &self,
        value: &Value,
        region: RegionId,
    ) -> Result<(ResourceHandle, bool), ResourceError> {
        let Value::Stream(reference) = value else {
            return Err(ResourceError::WrongKind);
        };
        let handle = self.issued_handle(value, reference.kind())?;
        let token = reference
            .evidence::<GraphToken>()
            .ok_or(ResourceError::Forged)?;
        let ended = token
            .stream
            .as_ref()
            .is_some_and(|state| state.load(Ordering::Relaxed) == 2);
        if !ended {
            self.check_graph_resource(value, region)?;
        }
        Ok((handle, ended))
    }
    pub(in crate::run) fn transfer_stream_arguments(
        &mut self,
        values: &[Value],
        region: RegionId,
    ) -> Result<(), RunError> {
        for value in values {
            if let Value::Stream(reference) = value {
                let handle = self
                    .issued_handle(value, reference.kind())
                    .map_err(|_| RunError::InvalidState)?;
                self.resources
                    .set_owner(handle.id, Some(region))
                    .map_err(|_| RunError::InvalidState)?;
            }
        }
        Ok(())
    }
    pub(in crate::run) fn transfer_return_resources(
        &mut self,
        strand: &Strand,
        result: &FunctionReturn,
        callee: RegionId,
        caller: RegionId,
    ) -> Result<(), RunError> {
        let FunctionReturn::Values { values, .. } = result else {
            return Ok(());
        };
        let mut handles = Vec::new();
        for value in values {
            if let Value::Handle(reference) | Value::Stream(reference) = value {
                self.check_graph_resource(value, strand.region)
                    .map_err(|_| RunError::InvalidState)?;
                let handle = self
                    .resource_handle(value, reference.kind())
                    .map_err(|_| RunError::InvalidState)?;
                let owner = self
                    .resources
                    .entry(handle.id)
                    .map_err(|_| RunError::InvalidState)?
                    .owner;
                if owner.is_some_and(|owner| self.regions.is_within(owner, callee)) {
                    handles.push(handle.id);
                }
            }
        }
        for id in handles {
            self.resources
                .set_owner(id, Some(caller))
                .map_err(|_| RunError::InvalidState)?;
        }
        Ok(())
    }
}
pub(in crate::run) fn resource_fault(error: ResourceError) -> KernelFault {
    KernelFault {
        code: (match error {
            ResourceError::StaleHandle => "core/StaleHandle",
            ResourceError::WrongOwner => "core/ResourceOwner",
            _ => "core/InvalidHandle",
        })
        .into(),
        message_key: error.message_key().into(),
        message: "The resource reference is unavailable in this scope.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn region_retirement_removes_every_entry_before_closing_newest_first_despite_panic() {
        struct Probe(Arc<Mutex<Vec<u8>>>, u8);
        impl RunResource for Probe {
            fn close(self: Box<Self>, reason: CloseReason) {
                assert_eq!(reason, CloseReason::ScopeEnded);
                self.0.lock().unwrap().push(self.1);
                if self.1 == 1 {
                    panic!("closer failed");
                }
            }
        }
        let log = Arc::new(Mutex::new(vec![]));
        let mut table = Resources::default();
        let owner = RegionId::root(1);
        for tag in 0..3 {
            let id = table
                .insert(
                    &"acme/session@1".parse().unwrap(),
                    Box::new(Probe(log.clone(), tag)),
                    3,
                )
                .unwrap();
            table.set_owner(id, Some(owner)).unwrap();
        }
        assert!(catch_unwind(AssertUnwindSafe(|| table.close_region_resources(owner))).is_err());
        assert_eq!(*log.lock().unwrap(), [2, 1, 0]);
        assert_eq!(table.len(), 0);
        assert!(table.owners.is_empty());
        assert_eq!(table.close_region_resources(owner).unwrap(), 0);
    }
}
