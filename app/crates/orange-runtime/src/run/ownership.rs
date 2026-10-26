//! Actor-owned region lifetimes. Identities are never reused within a run.
use super::*;
use std::collections::BTreeSet;

/// Region ownership is usually a singleton. Keep that case allocation-free;
/// only Fork/cleanup fan-out needs an ordered spill set.
#[derive(Default)]
pub(super) enum IdSet {
    #[default]
    Empty,
    One(u64),
    // Only the uncommon spill is boxed, keeping every serial region header
    // compact. Inlining the collection enlarges both ownership fields.
    #[expect(
        clippy::box_collection,
        reason = "boxed spill keeps the common singleton representation compact"
    )]
    Many(Box<BTreeSet<u64>>),
}
impl IdSet {
    pub fn one(id: u64) -> Self {
        Self::One(id)
    }
    pub fn len(&self) -> usize {
        match self {
            Self::Empty => 0,
            Self::One(_) => 1,
            Self::Many(ids) => ids.len(),
        }
    }
    pub fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }
    pub fn iter(&self) -> impl Iterator<Item = u64> {
        let (one, many) = match self {
            Self::Empty => (None, None),
            Self::One(id) => (Some(*id), None),
            Self::Many(ids) => (None, Some(ids.as_ref())),
        };
        one.into_iter()
            .chain(many.into_iter().flat_map(|ids| ids.iter().copied()))
    }
    pub fn clear(&mut self) {
        *self = Self::Empty;
    }
    pub fn insert(&mut self, id: u64) -> bool {
        match self {
            Self::Empty => {
                *self = Self::One(id);
                true
            }
            Self::One(old) if *old == id => false,
            Self::One(old) => {
                *self = Self::Many(Box::new(BTreeSet::from([*old, id])));
                true
            }
            Self::Many(ids) => ids.insert(id),
        }
    }
    pub fn remove(&mut self, id: &u64) -> bool {
        match self {
            Self::Empty => false,
            Self::One(old) if old == id => {
                *self = Self::Empty;
                true
            }
            Self::One(_) => false,
            Self::Many(ids) => {
                let removed = ids.remove(id);
                if ids.len() == 1 {
                    *self = Self::One(*ids.first().expect("nonempty ownership spill"));
                }
                removed
            }
        }
    }
}
impl FromIterator<u64> for IdSet {
    fn from_iter<T: IntoIterator<Item = u64>>(values: T) -> Self {
        let mut out = Self::default();
        for id in values {
            out.insert(id);
        }
        out
    }
}

pub(super) enum RegionKind {
    Root,
    Body {
        loop_owner: Option<ControlIx>,
    },
    Branch,
    Join {
        owner: ControlIx,
        mode: ForkJoin,
        fired: bool,
    },
}
pub(super) struct Region {
    pub parent: Option<u64>,
    pub activation: u64,
    pub depth: usize,
    pub live: IdSet,
    pub children: IdSet,
    pub continuation: Option<Strand>,
    pub kind: RegionKind,
}
impl Run {
    pub(super) fn owner_id(&mut self) -> Result<u64, RunError> {
        let id = self.next_owner;
        self.next_owner = id.checked_add(1).ok_or(RunError::ResourceLimit)?;
        Ok(id)
    }
    pub(super) fn enter_body(
        &mut self,
        mut caller: Strand,
        loop_owner: Option<ControlIx>,
    ) -> Result<(), RunError> {
        let parent = self
            .regions
            .get(&caller.region)
            .ok_or(RunError::InvalidState)?;
        let depth = parent.depth + 1;
        if self.live_strands >= self.options.limits.max_strands
            || self.regions.len() >= self.options.limits.max_regions
            || depth > self.options.limits.max_region_depth
            || self.next_owner.checked_add(2).is_none()
        {
            return self.fail(RunError::ResourceLimit);
        }
        let region = self.owner_id()?;
        let mut child = self.child_strand(&caller, region)?;
        // A structured body continues the same logical strand. Only Fork
        // creates sibling strand identities.
        child.id = caller.id;
        caller.pc = None;
        self.regions
            .get_mut(&caller.region)
            .unwrap()
            .children
            .insert(region);
        self.regions.insert(
            region,
            Region {
                parent: Some(caller.region),
                activation: caller.sequence,
                depth,
                live: IdSet::one(child.id),
                children: IdSet::default(),
                continuation: Some(caller),
                kind: RegionKind::Body { loop_owner },
            },
        );
        self.live_strands += 1;
        self.ready.push_front(child.id, child);
        Ok(())
    }
    pub(super) fn child_strand(
        &mut self,
        caller: &Strand,
        region: u64,
    ) -> Result<Strand, RunError> {
        Ok(Strand {
            cleanup_deadline: caller.cleanup_deadline,
            fork: None,
            id: self.owner_id()?,
            region,
            sequence: caller.sequence,
            entry: caller.entry,
            pc: caller.pc,
            pull: PullState::default(),
            stack: vec![],
            waiting: None,
            yielded: false,
            reenter: None,
            cleanup: caller.cleanup,
            cleanup_remaining: caller.cleanup_remaining,
            enter_body: None,
            unwind: None,
            faulted: false,
        })
    }
    pub(super) fn retire_strand(&mut self, strand: Strand) -> Result<(), RunError> {
        let region = self
            .regions
            .get_mut(&strand.region)
            .ok_or(RunError::InvalidState)?;
        if !region.live.remove(&strand.id) {
            return Err(RunError::InvalidState);
        }
        self.live_strands -= 1;
        self.complete_regions(strand.region)
    }
    pub(super) fn complete_regions(&mut self, mut id: u64) -> Result<(), RunError> {
        loop {
            let r = self.regions.get(&id).ok_or(RunError::InvalidState)?;
            if !r.live.is_empty() || !r.children.is_empty() {
                return Ok(());
            }
            let r = self.regions.remove(&id).unwrap();
            if let RegionKind::Join {
                owner,
                fired: false,
                ..
            } = r.kind
            {
                self.record(RunEvent {
                    activation: r.activation,
                    node: self.plan.as_ref().unwrap().nodes()[owner.index()].node(),
                    outcome: ControlOutcome::Completed,
                    fault: None,
                });
            }
            if let Some(caller) = r.continuation {
                self.ready.push_front(caller.id, caller);
            }
            if let Some(parent) = r.parent {
                if !self
                    .regions
                    .get_mut(&parent)
                    .ok_or(RunError::InvalidState)?
                    .children
                    .remove(&id)
                {
                    return Err(RunError::InvalidState);
                }
                self.release_first_join(parent)?;
                if !self.regions.contains_key(&parent) {
                    return Ok(());
                }
                id = parent;
            } else {
                self.activations[r.activation as usize] = None;
                return Ok(());
            }
        }
    }
    pub(super) fn spawn_fork(
        &mut self,
        mut caller: Strand,
        owner: ControlIx,
    ) -> Result<(), RunError> {
        let plan = self.plan.as_ref().ok_or(RunError::InvalidState)?.clone();
        let ControlOperation::Fork {
            branches,
            completed,
            join,
        } = plan.nodes()[owner.index()].operation()
        else {
            return Err(RunError::InvalidState);
        };
        let n = branches.len();
        let depth = self.regions[&caller.region].depth;
        if n > self
            .options
            .limits
            .max_strands
            .saturating_sub(self.live_strands)
            || n.checked_add(1).is_none_or(|count| {
                count
                    > self
                        .options
                        .limits
                        .max_regions
                        .saturating_sub(self.regions.len())
            })
            || depth.saturating_add(2) > self.options.limits.max_region_depth
            || u64::try_from(n)
                .ok()
                .and_then(|n| n.checked_mul(2))
                .and_then(|n| n.checked_add(1))
                .and_then(|n| self.next_owner.checked_add(n))
                .is_none()
        {
            return self.fail(RunError::ResourceLimit);
        }
        let group = self.owner_id()?;
        let mut children = Vec::with_capacity(n);
        for target in branches {
            let region = self.owner_id()?;
            let mut child = self.child_strand(&caller, region)?;
            child.pc = *target;
            self.regions.insert(
                region,
                Region {
                    parent: Some(group),
                    activation: caller.sequence,
                    depth: depth + 2,
                    live: IdSet::one(child.id),
                    children: IdSet::default(),
                    continuation: None,
                    kind: RegionKind::Branch,
                },
            );
            children.push(child);
        }
        caller.pc = *completed;
        self.regions
            .get_mut(&caller.region)
            .unwrap()
            .children
            .insert(group);
        let parent = caller.region;
        let activation = caller.sequence;
        let detach = *join == ForkJoin::Detach;
        let continuation = if detach {
            self.record(RunEvent {
                activation,
                node: plan.nodes()[owner.index()].node(),
                outcome: ControlOutcome::Completed,
                fault: None,
            });
            self.ready.push_front(caller.id, caller);
            None
        } else {
            Some(caller)
        };
        self.regions.insert(
            group,
            Region {
                parent: Some(parent),
                activation,
                depth: depth + 1,
                live: IdSet::default(),
                children: children.iter().map(|s| s.region).collect(),
                continuation,
                kind: RegionKind::Join {
                    owner,
                    mode: *join,
                    fired: detach,
                },
            },
        );
        self.live_strands += n;
        for child in children.into_iter().rev() {
            self.ready.push_front(child.id, child);
        }
        self.segment_owner = None;
        Ok(())
    }
    fn release_first_join(&mut self, id: u64) -> Result<(), RunError> {
        let r = self.regions.get_mut(&id).ok_or(RunError::InvalidState)?;
        let RegionKind::Join { owner, mode, fired } = &mut r.kind else {
            return Ok(());
        };
        if *fired || !matches!(mode, ForkJoin::Any | ForkJoin::Race) {
            return Ok(());
        }
        *fired = true;
        let race = *mode == ForkJoin::Race;
        let owner = *owner;
        let activation = r.activation;
        let caller = r.continuation.take();
        self.record(RunEvent {
            activation,
            node: self.plan.as_ref().unwrap().nodes()[owner.index()].node(),
            outcome: ControlOutcome::Completed,
            fault: None,
        });
        if race {
            // One subtree walk retires all losers. Cancelling each sibling
            // separately would rescan the same ownership tree quadratically.
            self.cancel_region(id)?;
        }
        if let Some(caller) = caller {
            self.ready.push_front(caller.id, caller);
        }
        Ok(())
    }

    /// Retire normal descendants immediately, retaining cancelled wait cleanup
    /// in the same region so ancestors still observe its lifetime barrier.
    pub(super) fn cancel_region(&mut self, root: u64) -> Result<(), RunError> {
        // Traverse only owned descendants. Logical strand IDs may also identify
        // held serial continuations; only one queued/timed strand has that ID.
        let mut ids = Vec::new();
        let mut work = vec![root];
        let mut retired = 0usize;
        let mut timer_keys = BTreeSet::new();
        while let Some(id) = work.pop() {
            let region = self.regions.get(&id).ok_or(RunError::InvalidState)?;
            work.extend(region.children.iter());
            retired = retired
                .checked_add(region.live.len())
                .ok_or(RunError::InvalidState)?;
            for strand in region.live.iter() {
                if let Some(queued) = self.ready.remove(strand) {
                    self.cleanup_truncated = self
                        .cleanup_truncated
                        .saturating_add(u64::from(queued.cleanup));
                }
                if let Some(key) = self.timer_keys.remove(&strand) {
                    timer_keys.insert(key);
                }
            }
            ids.push(id);
        }
        let mut cleanup = Vec::new();
        let plan = self.plan.as_ref().ok_or(RunError::InvalidState)?.clone();
        // Timer keys preserve the original cancellation/outcome order.
        for key in timer_keys {
            let mut timer = self.timers.remove(&key).ok_or(RunError::InvalidState)?;
            let node = &plan.nodes()[timer.owner.index()];
            self.record(RunEvent {
                activation: timer.activation.sequence,
                node: node.node(),
                outcome: ControlOutcome::Cancelled,
                fault: None,
            });
            if let Some(target) = node.cancelled() {
                let s = &mut timer.activation;
                s.region = root;
                s.pc = Some(target);
                s.stack.clear();
                s.pull.reset();
                s.reenter = None;
                s.waiting = None;
                s.yielded = false;
                s.cleanup = true;
                let deadline = self.now.saturating_add(self.options.cleanup_timeout_ns);
                s.cleanup_deadline = Some(self.deadline.map_or(deadline, |old| old.min(deadline)));
                s.cleanup_remaining = self.options.max_cleanup_operations;
                cleanup.push(timer.activation);
            }
        }
        for id in ids {
            if id != root {
                self.regions.remove(&id);
            }
        }
        let r = self.regions.get_mut(&root).ok_or(RunError::InvalidState)?;
        r.live = cleanup.iter().map(|s| s.id).collect();
        r.children.clear();
        let activation = r.activation;
        self.live_strands = self
            .live_strands
            .checked_sub(retired)
            .and_then(|n| n.checked_add(cleanup.len()))
            .ok_or(RunError::InvalidState)?;
        if let Some(shared) = self
            .activations
            .get_mut(activation as usize)
            .and_then(Option::as_mut)
        {
            if let Some(scope) = shared.cleanup_scope.take() {
                shared.data.finish_cleanup(scope);
            }
            shared.pull_owner = None;
        }
        for strand in cleanup {
            self.ready.push_back(strand.id, strand);
        }
        // Completion is performed by the caller after its own bookkeeping.
        Ok(())
    }
    pub(super) fn loop_region(&self, mut id: u64, owner: ControlIx) -> Result<u64, RunError> {
        loop {
            let r = self.regions.get(&id).ok_or(RunError::InvalidState)?;
            if matches!(r.kind, RegionKind::Body { loop_owner: Some(active) } if active == owner) {
                return Ok(id);
            }
            id = r.parent.ok_or(RunError::InvalidState)?;
        }
    }
    pub(super) fn unwind_loop(
        &mut self,
        strand: Strand,
        region: u64,
        is_break: bool,
    ) -> Result<(), RunError> {
        let mut caller = self
            .regions
            .get_mut(&region)
            .and_then(|r| r.continuation.take())
            .ok_or(RunError::InvalidState)?;
        let owner = match self.regions[&region].kind {
            RegionKind::Body {
                loop_owner: Some(owner),
            } => owner,
            _ => return Err(RunError::InvalidState),
        };
        if is_break {
            caller.stack.pop().ok_or(RunError::InvalidState)?;
            caller.pc = match self.plan.as_ref().unwrap().nodes()[owner.index()].operation() {
                ControlOperation::Count { completed, .. }
                | ControlOperation::While { completed, .. } => *completed,
                _ => return Err(RunError::InvalidState),
            };
            let shared = self
                .activations
                .get_mut(strand.sequence as usize)
                .and_then(Option::as_mut)
                .ok_or(RunError::InvalidState)?;
            Self::publish_index(
                self.plan.as_ref().unwrap(),
                &self.layout,
                shared,
                owner,
                None,
            );
        } else {
            caller.yielded = true;
        }
        self.regions.get_mut(&region).unwrap().continuation = Some(caller);
        self.cancel_region(region)?;
        self.complete_regions(region)
    }
    pub(super) fn retire_failed(&mut self, strand: Strand) -> Result<(), RunError> {
        let mut root = strand.region;
        while let Some(parent) = self.regions[&root].parent {
            root = parent;
        }
        self.cancel_region(root)?;
        self.complete_regions(root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inline_ownership_ids_preserve_order_through_spill_and_removal() {
        assert!(std::mem::size_of::<IdSet>() <= 2 * std::mem::size_of::<u64>());
        let mut actual = IdSet::default();
        let mut expected = BTreeSet::new();
        let mut random = 11u64;
        for _ in 0..20_000 {
            random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
            let id = random >> 32 & 31;
            match random & 15 {
                0 => {
                    actual.clear();
                    expected.clear();
                }
                1..=8 => assert_eq!(actual.insert(id), expected.insert(id)),
                _ => assert_eq!(actual.remove(&id), expected.remove(&id)),
            }
            assert_eq!(
                actual.iter().collect::<Vec<_>>(),
                expected.iter().copied().collect::<Vec<_>>()
            );
            assert_eq!(actual.len(), expected.len());
            assert_eq!(actual.is_empty(), expected.is_empty());
        }
    }
}
