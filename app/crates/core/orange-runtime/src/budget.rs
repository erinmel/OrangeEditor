//! Run-owned aggregate nominal reservation budget.
//!
//! One [`Budget`] belongs to a run and is shared by every activation it creates.
//! Each activation-owned counter that is part of the aggregate (data caches, local
//! cells, retained outputs, function and iteration holds) is a [`Lease`] or a
//! [`Tally`] wrapping one. The lease returns its reservation when it drops, so
//! retiring an activation, a failed publication or disposing the run releases
//! capacity without any manual decrement bookkeeping.
//!
//! The budget is nominal like the per-activation limits it sits beside: it adds
//! the same byte charges, not allocator or resident-set measurements.
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

pub(crate) struct Budget {
    used: AtomicUsize,
    limit: usize,
}
impl Budget {
    pub(crate) fn new(limit: usize) -> Arc<Self> {
        Arc::new(Self {
            used: AtomicUsize::new(0),
            limit,
        })
    }
    pub(crate) fn used(&self) -> usize {
        self.used.load(Ordering::Relaxed)
    }
    pub(crate) fn limit(&self) -> usize {
        self.limit
    }
    pub(crate) fn headroom(&self) -> usize {
        self.limit.saturating_sub(self.used())
    }
    /// Whether `extra` more nominal bytes fit. Never mutates.
    pub(crate) fn admits(&self, extra: usize) -> bool {
        self.used()
            .checked_add(extra)
            .is_some_and(|n| n <= self.limit)
    }
}

/// A reservation against an optional [`Budget`], released on drop. Without a budget
/// (standalone data activations) every request is admitted.
pub(crate) struct Lease {
    budget: Option<Arc<Budget>>,
    held: usize,
}
impl Lease {
    pub(crate) fn new(budget: Option<Arc<Budget>>) -> Self {
        Self { budget, held: 0 }
    }
    pub(crate) fn headroom(&self) -> usize {
        self.budget.as_ref().map_or(usize::MAX, |b| b.headroom())
    }
    /// Whether the reservation could change to `bytes`; shrinking always fits.
    pub(crate) fn admits(&self, bytes: usize) -> bool {
        match (&self.budget, bytes.checked_sub(self.held)) {
            (Some(budget), Some(growth)) => budget.admits(growth),
            _ => true,
        }
    }
    /// Change the reservation to `bytes`. Growth must have been admitted.
    pub(crate) fn set(&mut self, bytes: usize) {
        if let Some(budget) = &self.budget {
            if bytes >= self.held {
                budget.used.fetch_add(bytes - self.held, Ordering::Relaxed);
            } else {
                budget.used.fetch_sub(self.held - bytes, Ordering::Relaxed);
            }
        }
        self.held = bytes;
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.set(0);
    }
}

/// A nominal counter whose charge, beyond a shared `exempt` base, is also reserved
/// from the run budget. Counter and reservation can only change together.
pub(crate) struct Tally {
    bytes: usize,
    exempt: usize,
    lease: Lease,
}
impl Tally {
    /// `exempt` is the part of `bytes` that shared immutable plan data already owns.
    pub(crate) fn new(budget: Option<Arc<Budget>>, exempt: usize) -> Self {
        Self {
            bytes: exempt,
            exempt,
            lease: Lease::new(budget),
        }
    }
    pub(crate) fn get(&self) -> usize {
        self.bytes
    }
    fn charged(&self, bytes: usize) -> usize {
        bytes.saturating_sub(self.exempt)
    }
    pub(crate) fn admits(&self, bytes: usize) -> bool {
        self.lease.admits(self.charged(bytes))
    }
    /// Run-budget bytes still free; unbounded without a budget.
    pub(crate) fn headroom(&self) -> usize {
        self.lease.headroom()
    }
    /// Change the counter; growth must have been admitted.
    pub(crate) fn set(&mut self, bytes: usize) {
        self.lease.set(self.charged(bytes));
        self.bytes = bytes;
    }
    pub(crate) fn try_set(&mut self, bytes: usize) -> bool {
        let fits = self.admits(bytes);
        if fits {
            self.set(bytes);
        }
        fits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusal_changes_nothing_and_drop_returns_capacity() {
        let budget = Budget::new(10);
        let mut a = Tally::new(Some(budget.clone()), 0);
        let mut b = Tally::new(Some(budget.clone()), 0);
        assert!(a.try_set(6));
        assert!(!b.try_set(5));
        assert_eq!((b.get(), budget.used()), (0, 6));
        // Replacing credits what the counter already holds.
        assert!(a.try_set(10));
        assert!(a.try_set(4));
        assert!(b.try_set(6));
        assert_eq!(budget.used(), 10);
        drop(a);
        assert_eq!(budget.used(), 6);
        drop(b);
        assert_eq!((budget.used(), budget.headroom()), (0, 10));
    }

    #[test]
    fn the_shared_base_is_not_charged() {
        let budget = Budget::new(4);
        let mut tally = Tally::new(Some(budget.clone()), 100);
        assert_eq!((tally.get(), budget.used()), (100, 0));
        assert!(tally.try_set(104));
        assert!(!tally.try_set(105));
        assert_eq!((tally.get(), budget.used()), (104, 4));
    }

    #[test]
    fn without_a_budget_every_request_is_admitted() {
        let mut tally = Tally::new(None, 0);
        assert!(tally.try_set(usize::MAX));
        assert_eq!(tally.headroom(), usize::MAX);
    }
}
