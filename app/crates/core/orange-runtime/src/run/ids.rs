//! Typed identities of a run's strands, regions and function frames. Strands and regions
//! are drawn from the run's one owner counter; a frame has no identity of its own, only
//! that of the function region that owns it. The types keep the three from being mixed up.

use super::RunError;

macro_rules! owner_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub(super) struct $name(u64);
    };
}

owner_id!(
    /// One strand of control.
    StrandId
);
owner_id!(
    /// One ownership region: a root, body, branch, join or function region.
    RegionId
);
owner_id!(
    /// One function call frame.
    FrameId
);

impl StrandId {
    /// The first strand of activation `activation`.
    pub(super) const fn root(activation: u64) -> Self {
        Self(activation)
    }
}

impl RegionId {
    /// The root region of activation `activation`.
    pub(super) const fn root(activation: u64) -> Self {
        Self(activation)
    }
}

/// Mints every strand and region identity after the roots. Activation `n` owns the root
/// identities `n`, and the counter starts above every activation, so minted identities
/// never collide with roots and are never reused within a run.
pub(super) struct OwnerIds {
    next: u64,
}

impl OwnerIds {
    pub(super) const fn after_roots(activations: u64) -> Self {
        Self { next: activations }
    }
    /// Whether `count` more identities can still be minted.
    pub(super) fn has_room(&self, count: u64) -> bool {
        self.next.checked_add(count).is_some()
    }
    pub(super) fn strand(&mut self) -> Result<StrandId, RunError> {
        self.mint().map(StrandId)
    }
    pub(super) fn region(&mut self) -> Result<RegionId, RunError> {
        self.mint().map(RegionId)
    }
    /// Dynamic roots share one fresh owner identity, just like startup roots.
    pub(super) fn root(&mut self) -> Result<(u64, StrandId, RegionId), RunError> {
        let id = self.mint()?;
        Ok((id, StrandId(id), RegionId(id)))
    }
    fn mint(&mut self) -> Result<u64, RunError> {
        let id = self.next;
        self.next = id.checked_add(1).ok_or(RunError::ResourceLimit)?;
        Ok(id)
    }
}

impl FrameId {
    /// A call frame shares the identity of the function region that owns it.
    pub(super) const fn of_region(region: RegionId) -> Self {
        Self(region.0)
    }
}

impl RegionId {
    /// The function region that owns `frame`.
    pub(super) const fn of_frame(frame: FrameId) -> Self {
        Self(frame.0)
    }
}

/// Hasher for the run's own minted identities. They are sequential integers that never
/// come from graph, host or network input, so the standard SipHash's flooding resistance
/// buys nothing on these internal directories while costing most of each lookup. One odd
/// multiplication is a bijection on the low bits, so consecutive identities fill distinct
/// buckets, and it spreads them into the high bits the table's control bytes read.
#[derive(Default, Clone, Copy)]
pub(super) struct IdHasher(u64);

impl std::hash::Hasher for IdHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write_u64(&mut self, id: u64) {
        self.0 = (self.0 ^ id).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }
    /// Identities hash through `write_u64`; other input still mixes every byte.
    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.write_u64(u64::from(byte));
        }
    }
}

/// A directory keyed by run-minted identities (see [`IdHasher`]).
pub(super) type IdMap<K, V> =
    std::collections::HashMap<K, V, std::hash::BuildHasherDefault<IdHasher>>;
