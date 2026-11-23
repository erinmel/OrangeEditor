//! Shared aggregate resource admission for every guest store.
use crate::{GuestError, GuestLimits};
use std::sync::{Arc, Mutex, PoisonError};
use wasmtime::ResourceLimiter;

pub(crate) struct InstanceLimits {
    aggregate: Arc<Mutex<Aggregate>>,
    policy: GuestLimits,
    memory: usize,
    elements: usize,
    pending_memory: usize,
    pending_elements: usize,
}

#[derive(Default)]
pub(crate) struct Aggregate {
    stores: usize,
    memory: usize,
    elements: usize,
}

impl InstanceLimits {
    pub(crate) fn new(
        policy: GuestLimits,
        aggregate: &Arc<Mutex<Aggregate>>,
    ) -> Result<Self, GuestError> {
        let mut counts = aggregate.lock().unwrap_or_else(PoisonError::into_inner);
        if counts.stores >= policy.stores {
            return Err(GuestError::Capacity);
        }
        counts.stores += 1;
        Ok(Self {
            aggregate: Arc::clone(aggregate),
            policy,
            memory: 0,
            elements: 0,
            pending_memory: 0,
            pending_elements: 0,
        })
    }
    fn reserve(
        used: &mut usize,
        pending: &mut usize,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
        limit: usize,
    ) -> bool {
        // A later request means the preceding synchronous allocation succeeded.
        // Only a permitted request can subsequently call *_grow_failed.
        *pending = 0;
        if maximum.is_some_and(|max| desired > max) {
            return false;
        }
        let Some(growth) = desired.checked_sub(current) else {
            return false;
        };
        let Some(next) = used.checked_add(growth).filter(|n| *n <= limit) else {
            return false;
        };
        *used = next;
        *pending = growth;
        true
    }
}

impl Drop for InstanceLimits {
    fn drop(&mut self) {
        let mut totals = self
            .aggregate
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        totals.stores -= 1;
        totals.memory -= self.memory;
        totals.elements -= self.elements;
    }
}

impl ResourceLimiter for InstanceLimits {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        let mut totals = self
            .aggregate
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let limit = self
            .policy
            .total_memory_bytes
            .saturating_sub(totals.memory - self.memory)
            .min(self.policy.memory_bytes);
        let allowed = Self::reserve(
            &mut self.memory,
            &mut self.pending_memory,
            current,
            desired,
            maximum,
            limit,
        );
        totals.memory += self.pending_memory;
        Ok(allowed)
    }
    fn memory_grow_failed(&mut self, _: wasmtime::Error) -> wasmtime::Result<()> {
        self.memory -= self.pending_memory;
        self.aggregate
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .memory -= self.pending_memory;
        self.pending_memory = 0;
        Ok(())
    }
    fn table_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        let mut totals = self
            .aggregate
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let limit = self
            .policy
            .total_table_elements
            .saturating_sub(totals.elements - self.elements)
            .min(self.policy.table_elements);
        let allowed = Self::reserve(
            &mut self.elements,
            &mut self.pending_elements,
            current,
            desired,
            maximum,
            limit,
        );
        totals.elements += self.pending_elements;
        Ok(allowed)
    }
    fn table_grow_failed(&mut self, _: wasmtime::Error) -> wasmtime::Result<()> {
        self.elements -= self.pending_elements;
        self.aggregate
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .elements -= self.pending_elements;
        self.pending_elements = 0;
        Ok(())
    }
    fn instances(&self) -> usize {
        self.policy.instances
    }
    fn tables(&self) -> usize {
        self.policy.tables
    }
    fn memories(&self) -> usize {
        self.policy.memories
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_engine_limits_total_guest_store_usage_and_reclaims_it_on_drop() {
        let engine = Arc::default();
        let policy = GuestLimits {
            stores: 2,
            memory_bytes: 100,
            total_memory_bytes: 100,
            table_elements: 16,
            total_table_elements: 16,
            ..GuestLimits::default()
        };
        let mut first = InstanceLimits::new(policy, &engine).unwrap();
        let mut second = InstanceLimits::new(policy, &engine).unwrap();
        assert!(matches!(
            InstanceLimits::new(policy, &engine),
            Err(GuestError::Capacity)
        ));
        assert!(first.memory_growing(0, 60, None).unwrap());
        assert!(!second.memory_growing(0, 41, None).unwrap());
        assert!(second.memory_growing(0, 40, None).unwrap());
        assert!(first.table_growing(0, 10, None).unwrap());
        assert!(!second.table_growing(0, 7, None).unwrap());
        drop(first);
        let _replacement = InstanceLimits::new(policy, &engine).unwrap();
        assert!(second.memory_growing(40, 100, None).unwrap());
        assert!(second.table_growing(0, 16, None).unwrap());
    }
    #[test]
    fn failed_allocations_refund_reservations_without_crediting_a_previous_success() {
        let mut limits = InstanceLimits::new(
            GuestLimits {
                memory_bytes: 100,
                table_elements: 100,
                ..Default::default()
            },
            &Arc::default(),
        )
        .unwrap();
        assert!(limits.memory_growing(0, 60, None).unwrap());
        assert!(!limits.memory_growing(60, 101, None).unwrap());
        assert!(!limits.memory_growing(60, 90, Some(80)).unwrap());
        assert!(limits.memory_growing(0, 40, None).unwrap());
        limits
            .memory_grow_failed(wasmtime::Error::msg("synthetic allocation failure"))
            .unwrap();
        assert_eq!(limits.memory, 60);
        assert!(limits.memory_growing(0, 40, None).unwrap());
        assert!(!limits.memory_growing(0, 1, None).unwrap());
        assert!(limits.table_growing(0, 60, None).unwrap());
        assert!(!limits.table_growing(60, 101, None).unwrap());
        assert!(!limits.table_growing(60, 90, Some(80)).unwrap());
        assert!(limits.table_growing(0, 40, None).unwrap());
        limits
            .table_grow_failed(wasmtime::Error::msg("synthetic allocation failure"))
            .unwrap();
        assert_eq!(limits.elements, 60);
        assert!(limits.table_growing(0, 40, None).unwrap());
        assert!(!limits.table_growing(0, 1, None).unwrap());
    }
}
