//! Shared aggregate resource admission for every guest store.
use crate::GuestLimits;
use wasmtime::ResourceLimiter;

pub(crate) struct InstanceLimits {
    policy: GuestLimits,
    memory: usize,
    elements: usize,
    pending_memory: usize,
    pending_elements: usize,
}

impl InstanceLimits {
    pub(crate) fn new(policy: GuestLimits) -> Self {
        Self {
            policy,
            memory: 0,
            elements: 0,
            pending_memory: 0,
            pending_elements: 0,
        }
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

impl ResourceLimiter for InstanceLimits {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        Ok(Self::reserve(
            &mut self.memory,
            &mut self.pending_memory,
            current,
            desired,
            maximum,
            self.policy.memory_bytes,
        ))
    }
    fn memory_grow_failed(&mut self, _: wasmtime::Error) -> wasmtime::Result<()> {
        self.memory -= self.pending_memory;
        self.pending_memory = 0;
        Ok(())
    }
    fn table_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        Ok(Self::reserve(
            &mut self.elements,
            &mut self.pending_elements,
            current,
            desired,
            maximum,
            self.policy.table_elements,
        ))
    }
    fn table_grow_failed(&mut self, _: wasmtime::Error) -> wasmtime::Result<()> {
        self.elements -= self.pending_elements;
        self.pending_elements = 0;
        Ok(())
    }
    fn instances(&self) -> usize {
        self.policy.instances
    }
    fn memories(&self) -> usize {
        self.policy.memories
    }
    fn tables(&self) -> usize {
        self.policy.tables
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_allocations_refund_reservations_without_crediting_a_previous_success() {
        let mut limits = InstanceLimits::new(GuestLimits {
            memory_bytes: 100,
            table_elements: 100,
            ..Default::default()
        });
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
