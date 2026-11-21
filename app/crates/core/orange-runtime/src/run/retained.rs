//! Scoped nominal value leases shared by iteration and function ownership.
use super::*;
use crate::budget::Lease;
use std::sync::atomic::AtomicUsize;

/// A frame or iteration hold. It counts against the activation's `max_state_bytes`
/// and, through its lease, reserves the same bytes from the run's aggregate budget
/// until it drops.
pub(super) struct Hold {
    pub(super) bytes: usize,
    total: Arc<AtomicUsize>,
    _lease: Lease,
}
impl Hold {
    pub(super) fn new(
        shared: &ActivationState,
        bytes: usize,
        credit: usize,
        limit: usize,
    ) -> Result<Self, RunError> {
        Self::with_retained(shared, bytes, credit, shared.retained_bytes.get(), limit)
    }
    /// `retained` is the activation's output charge after the same publication. The
    /// replaced hold (`credit`, still alive until the caller swaps it) and any
    /// growth from `retained` are admitted against the run budget together, so a
    /// refusal leaves every counter unchanged and a later swap cannot fail.
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
            .and_then(|n| n.checked_add(shared.local_bytes.get()))
            .and_then(|n| n.checked_add(retained))
            .filter(|n| *n <= limit)
            .ok_or(RunError::ResourceLimit)?;
        let growth = bytes
            .saturating_sub(credit)
            .saturating_add(retained.saturating_sub(shared.retained_bytes.get()));
        if !shared.budget.admits(growth) {
            return Err(RunError::AggregateLimit);
        }
        let mut lease = Lease::new(Some(shared.budget.clone()));
        lease.set(bytes);
        shared.owned_bytes.fetch_add(bytes, Ordering::Relaxed);
        Ok(Self {
            bytes,
            total: shared.owned_bytes.clone(),
            _lease: lease,
        })
    }
}
impl Drop for Hold {
    fn drop(&mut self) {
        let old = self.total.fetch_sub(self.bytes, Ordering::Relaxed);
        debug_assert!(old >= self.bytes);
    }
}
