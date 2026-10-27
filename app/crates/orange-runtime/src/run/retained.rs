//! Scoped nominal value leases shared by iteration and function ownership.
use super::*;
use std::sync::atomic::AtomicUsize;
pub(super) struct Hold {
    pub(super) bytes: usize,
    total: Arc<AtomicUsize>,
}
impl Hold {
    pub(super) fn new(
        shared: &ActivationState,
        bytes: usize,
        credit: usize,
        limit: usize,
    ) -> Result<Self, RunError> {
        Self::with_retained(shared, bytes, credit, shared.retained_bytes, limit)
    }
    pub(super) fn with_retained(
        shared: &ActivationState,
        bytes: usize,
        credit: usize,
        retained: usize,
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
            .and_then(|n| n.checked_add(retained))
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
