//! Typed identities of a run's strands, regions and function frames. Strands and regions
//! are drawn from the run's one owner counter; a frame has no identity of its own, only
//! that of the function region that owns it. The types keep the three from being mixed up.

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
    /// Wraps a value from the owner counter.
    pub(super) const fn new(raw: u64) -> Self {
        Self(raw)
    }
}

impl RegionId {
    /// Wraps a value from the owner counter.
    pub(super) const fn new(raw: u64) -> Self {
        Self(raw)
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
