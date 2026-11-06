//! Actor-owned region lifetimes. Identities are never reused within a run.
use super::*;

/// Region ownership is usually a singleton. Keep that case allocation-free;
/// only Fork/cleanup fan-out needs an ordered spill set.
#[derive(Default)]
pub(super) enum IdSet<T> {
    #[default]
    Empty,
    One(T),
    // Only the uncommon spill is boxed, keeping every serial region header
    // compact. Inlining the collection enlarges both ownership fields.
    Many(Box<Spill<T>>),
}

/// Ordered spill storage: ascending identities plus one removal bit each. Identities
/// are minted in increasing order, so a new child is an append, and removal is a binary
/// search that only sets its bit. Removed identities stay until fewer than half are
/// live, then one pass compacts them. Each slot costs one identity and one bit, so even
/// at the compaction bound the spill holds about 16 bytes per live member (a B-tree
/// holds a similar amount in partly filled nodes) while insertion is amortized O(1),
/// removal O(log n) and nothing allocates per member. An identity older than the newest
/// compacts and shifts the tail once; bulk construction sorts instead.
pub(super) struct Spill<T> {
    ids: Vec<T>,
    /// Removal bits of the first 64 slots, inline so a small spill allocates only `ids`.
    first: u64,
    /// Removal bits of every later slot.
    rest: Vec<u64>,
    live: usize,
}

impl<T: Copy + Ord> Spill<T> {
    fn sorted(ids: Vec<T>) -> Self {
        Self {
            live: ids.len(),
            first: 0,
            rest: vec![0; ids.len().saturating_sub(64).div_ceil(64)],
            ids,
        }
    }
    fn word(&mut self, i: usize) -> &mut u64 {
        match i.checked_sub(64) {
            None => &mut self.first,
            Some(later) => &mut self.rest[later / 64],
        }
    }
    fn is_removed(&self, i: usize) -> bool {
        let word = match i.checked_sub(64) {
            None => self.first,
            Some(later) => self.rest[later / 64],
        };
        word >> (i % 64) & 1 == 1
    }
    fn fit_bits(&mut self) {
        self.rest
            .resize(self.ids.len().saturating_sub(64).div_ceil(64), 0);
    }
    fn iter(&self) -> impl Iterator<Item = T> + '_ {
        self.ids
            .iter()
            .enumerate()
            .filter(|&(i, _)| !self.is_removed(i))
            .map(|(_, id)| *id)
    }
    fn compact(&mut self) {
        let mut kept = 0;
        for i in 0..self.ids.len() {
            if !self.is_removed(i) {
                self.ids[kept] = self.ids[i];
                kept += 1;
            }
        }
        self.ids.truncate(kept);
        self.first = 0;
        self.rest.clear();
        self.fit_bits();
    }
    fn insert(&mut self, id: T) -> bool {
        if self.ids.last().is_none_or(|last| *last < id) {
            self.ids.push(id);
            if self.ids.len() > 64 * (1 + self.rest.len()) {
                self.rest.push(0);
            }
        } else {
            match self.ids.binary_search(&id) {
                Ok(i) if !self.is_removed(i) => return false,
                Ok(i) => *self.word(i) &= !(1 << (i % 64)),
                Err(_) => {
                    if self.live < self.ids.len() {
                        self.compact();
                    }
                    let i = self.ids.binary_search(&id).unwrap_or_else(|i| i);
                    self.ids.insert(i, id);
                    self.fit_bits();
                }
            }
        }
        self.live += 1;
        true
    }
    fn remove(&mut self, id: &T) -> bool {
        let Ok(i) = self.ids.binary_search(id) else {
            return false;
        };
        if self.is_removed(i) {
            return false;
        }
        *self.word(i) |= 1 << (i % 64);
        self.live -= 1;
        if self.live * 2 < self.ids.len() {
            self.compact();
        }
        true
    }
}

impl<T: Copy + Ord> IdSet<T> {
    pub fn one(id: T) -> Self {
        Self::One(id)
    }
    pub fn len(&self) -> usize {
        match self {
            Self::Empty => 0,
            Self::One(_) => 1,
            Self::Many(ids) => ids.live,
        }
    }
    pub fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }
    pub fn iter(&self) -> impl Iterator<Item = T> {
        let (one, many) = match self {
            Self::Empty => (None, None),
            Self::One(id) => (Some(*id), None),
            Self::Many(ids) => (None, Some(ids.as_ref())),
        };
        one.into_iter()
            .chain(many.into_iter().flat_map(|ids| ids.iter()))
    }
    pub fn clear(&mut self) {
        *self = Self::Empty;
    }
    pub fn insert(&mut self, id: T) -> bool {
        match self {
            Self::Empty => {
                *self = Self::One(id);
                true
            }
            Self::One(old) if *old == id => false,
            Self::One(old) => {
                let (a, b) = if *old < id { (*old, id) } else { (id, *old) };
                *self = Self::Many(Box::new(Spill::sorted(vec![a, b])));
                true
            }
            Self::Many(ids) => ids.insert(id),
        }
    }
    pub fn remove(&mut self, id: &T) -> bool {
        match self {
            Self::Empty => false,
            Self::One(old) if old == id => {
                *self = Self::Empty;
                true
            }
            Self::One(_) => false,
            Self::Many(ids) => {
                let removed = ids.remove(id);
                let only = (ids.live == 1).then(|| ids.iter().next()).flatten();
                if let Some(only) = only {
                    *self = Self::One(only);
                }
                removed
            }
        }
    }
}
impl<T: Copy + Ord> FromIterator<T> for IdSet<T> {
    /// Sorts once, so identities arriving in any order never shift a large spill.
    fn from_iter<I: IntoIterator<Item = T>>(values: I) -> Self {
        let mut values = values.into_iter();
        let Some(first) = values.next() else {
            return Self::Empty;
        };
        let Some(second) = values.next() else {
            return Self::One(first);
        };
        let mut ids: Vec<T> = [first, second].into_iter().chain(values).collect();
        ids.sort_unstable();
        ids.dedup();
        match ids.as_slice() {
            [only] => Self::One(*only),
            _ => Self::Many(Box::new(Spill::sorted(ids))),
        }
    }
}

pub(super) enum RegionKind {
    Root,
    Function {
        owner: ControlIx,
        result: Option<FunctionReturn>,
    },
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
    pub frames: Option<Box<IdSet<FrameId>>>,
    pub retired_cursor: Option<Box<Cursor>>,
    pub parent: Option<RegionId>,
    pub activation: u64,
    pub depth: usize,
    pub live: IdSet<StrandId>,
    pub children: IdSet<RegionId>,
    pub continuation: Option<Strand>,
    pub kind: RegionKind,
}
impl Run {
    pub(super) fn enter_body(
        &mut self,
        mut caller: Strand,
        loop_owner: Option<ControlIx>,
    ) -> Result<(), RunError> {
        let depth = self.regions.get(caller.region)?.depth + 1;
        if !self.regions.has_room(1, 1, &self.options.limits)
            || depth > self.options.limits.max_region_depth
        {
            return self.fail(RunError::ResourceLimit);
        }
        let region = self.regions.mint_region()?;
        let mut child = self.child_strand(&caller, region)?;
        if let Some(owner) = loop_owner
            && let Some(Cursor::ForEach(cursor)) = caller.stack.last()
        {
            child.iteration_context = Some(cursor.context(owner)?);
        }

        // A structured body continues the same logical strand. Only Fork
        // creates sibling strand identities.
        child.id = caller.id;
        child.pull = caller.pull.lend();
        caller.pc = None;
        self.regions.open(
            region,
            Region {
                frames: None,
                retired_cursor: None,
                parent: Some(caller.region),
                activation: caller.sequence,
                depth,
                live: IdSet::one(child.id),
                children: IdSet::default(),
                continuation: Some(caller),
                kind: RegionKind::Body { loop_owner },
            },
        )?;
        self.ready.push_front(child.id, child);
        Ok(())
    }
    pub(super) fn child_strand(
        &mut self,
        caller: &Strand,
        region: RegionId,
    ) -> Result<Strand, RunError> {
        Ok(Strand {
            function: caller.function,
            transfer: Transfer::None,
            cleanup_deadline: caller.cleanup_deadline,
            iteration_context: caller.iteration_context.clone(),
            id: self.regions.mint_strand()?,
            region,
            sequence: caller.sequence,
            entry: caller.entry,
            pc: caller.pc,
            pull: PullState::default(),
            stack: vec![],
            yielded: false,
            reenter: None,
            cleanup: caller.cleanup,
            cleanup_remaining: caller.cleanup_remaining,
        })
    }
    pub(super) fn retire_strand(&mut self, strand: Strand) -> Result<(), RunError> {
        let region = self.regions.retire_strand(strand.region, strand.id)?;
        if let Some(caller) = region.continuation.as_mut() {
            caller.pull.recycle(strand.pull);
        }
        self.complete_regions(strand.region)
    }
    /// Retires `frames` of `activation`, releasing their call projections.
    fn retire_frames(
        &mut self,
        frames: impl Iterator<Item = FrameId>,
        activation: u64,
    ) -> Result<(), RunError> {
        let plan = self.plan.as_deref().ok_or(RunError::InvalidState)?;
        for frame in frames {
            let shared = self
                .activations
                .get_mut(activation)
                .and_then(Option::as_deref_mut)
                .ok_or(RunError::InvalidState)?;
            self.frames.retire(frame, plan, &self.layout, shared)?;
        }
        Ok(())
    }
    /// Opens the callee region and frame for `call`. The caller waits as the region's
    /// continuation and its strand identity passes to the callee.
    pub(super) fn enter_function(
        &mut self,
        mut caller: Strand,
        call: PendingCall,
    ) -> Result<(), RunError> {
        let plan = self.plan.as_ref().ok_or(RunError::InvalidState)?.clone();
        let (owner, scope) = call.target();
        let entry = plan.function(scope).ok_or(RunError::InvalidState)?.entry();
        let depth = self.regions.get(caller.region)?.depth + 1;
        if depth > self.options.limits.max_region_depth
            || !self.regions.has_room(1, 1, &self.options.limits)
        {
            return self.fail(RunError::ResourceLimit);
        }
        let shared = self
            .activations
            .get_mut(caller.sequence)
            .ok_or(RunError::InvalidState)?
            .as_deref()
            .ok_or(RunError::InvalidState)?;
        let frame = Frames::prepare(
            &plan,
            call,
            caller.function,
            shared,
            self.options.limits.max_state_bytes,
        )?;
        let region = self.regions.mint_region()?;
        let mut child = self.child_strand(&caller, region)?;
        child.id = caller.id;
        child.pull = caller.pull.lend();
        child.function = Some(FrameId::of_region(region));
        child.pc = entry;
        caller.pc = None;
        self.frames.insert(FrameId::of_region(region), frame);
        self.regions.open(
            region,
            Region {
                frames: None,
                retired_cursor: None,
                parent: Some(caller.region),
                activation: caller.sequence,
                depth,
                live: IdSet::one(child.id),
                children: IdSet::default(),
                continuation: Some(caller),
                kind: RegionKind::Function {
                    owner,
                    result: None,
                },
            },
        )?;
        self.ready.push_front(child.id, child);
        Ok(())
    }
    /// Ends the callee's region with `result`; the caller resumes once its cleanup ends.
    pub(super) fn leave_function(
        &mut self,
        strand: Strand,
        result: FunctionReturn,
    ) -> Result<(), RunError> {
        let region = RegionId::of_frame(strand.function.ok_or(RunError::InvalidState)?);
        let frame = self.regions.get_mut(region)?;
        let RegionKind::Function {
            result: pending, ..
        } = &mut frame.kind
        else {
            return Err(RunError::InvalidState);
        };
        if pending.is_some() {
            return Err(RunError::InvalidState);
        }
        *pending = Some(result);
        if let Some(caller) = frame.continuation.as_mut() {
            caller.pull.recycle(strand.pull);
        }
        self.cancel_region(region)?;
        self.complete_regions(region)
    }
    /// Completes the caller's Call node `owner` with the callee's `result`.
    fn finish_function(
        &mut self,
        caller: &mut Strand,
        owner: ControlIx,
        result: FunctionReturn,
    ) -> Result<(), RunError> {
        let plan = self.plan.as_ref().ok_or(RunError::InvalidState)?.clone();
        let mut shared = self
            .activations
            .get_mut(caller.sequence)
            .ok_or(RunError::InvalidState)?
            .take()
            .ok_or(RunError::InvalidState)?;
        let result = call_returned(
            &plan,
            &self.layout,
            caller,
            &mut shared,
            owner,
            result,
            self.options.limits.max_state_bytes,
        )
        .and_then(|step| self.complete(&plan, caller, &mut shared, owner, step));
        *self
            .activations
            .get_mut(caller.sequence)
            .ok_or(RunError::InvalidState)? = Some(shared);
        result
    }
    pub(super) fn complete_regions(&mut self, mut id: RegionId) -> Result<(), RunError> {
        loop {
            let Some(mut r) = self.regions.take_if_done(id)? else {
                return Ok(());
            };
            let function =
                matches!(r.kind, RegionKind::Function { .. }).then(|| FrameId::of_region(id));
            let retired = r.frames.iter().flat_map(|frames| frames.iter());
            self.retire_frames(retired.chain(function), r.activation)?;
            if let RegionKind::Function { owner, result } = r.kind {
                let result = match result {
                    Some(result) => result,
                    None => {
                        let plan = self.plan.as_ref().ok_or(RunError::InvalidState)?;
                        let node = &plan.nodes()[owner.index()];
                        let ControlOperation::Call { scope, .. } = node.operation() else {
                            return Err(RunError::InvalidState);
                        };
                        if plan
                            .function(*scope)
                            .ok_or(RunError::InvalidState)?
                            .outputs()
                            == 0
                        {
                            FunctionReturn::empty()
                        } else {
                            FunctionReturn::Fault(ConstantFault {
                                source: node.node(),
                                fault: KernelFault {
                                    code: "core/MissingReturn",
                                    message_key: "core/function-missing-return",
                                    message: "The function ended without returning its declared results.",
                                },
                            })
                        }
                    }
                };
                let mut caller = r.continuation.take().ok_or(RunError::InvalidState)?;
                self.finish_function(&mut caller, owner, result)?;
                r.continuation = Some(caller);
                r.kind = RegionKind::Body { loop_owner: None };
            }
            if let Some(cursor) = r.retired_cursor.as_deref() {
                let owner = match cursor {
                    Cursor::Count { owner, .. } | Cursor::While { owner, .. } => Some(*owner),
                    _ => None,
                };
                if let Some(owner) = owner {
                    let shared = self
                        .activations
                        .get_mut(r.activation)
                        .and_then(Option::as_mut)
                        .ok_or(RunError::InvalidState)?;
                    Self::publish_index(
                        self.plan.as_ref().ok_or(RunError::InvalidState)?,
                        &self.layout,
                        shared,
                        owner,
                        None,
                    )?;
                }
            }

            if let RegionKind::Join {
                owner,
                fired: false,
                ..
            } = r.kind
            {
                self.record(RunEvent {
                    activation: r.activation,
                    node: self.plan.as_ref().ok_or(RunError::InvalidState)?.nodes()[owner.index()]
                        .node(),
                    outcome: ControlOutcome::Completed,
                    fault: None,
                });
            }
            if let Some(caller) = r.continuation {
                self.ready.push_front(caller.id, caller);
            }
            if let Some(parent) = r.parent {
                self.regions.detach(parent, id)?;
                self.release_first_join(parent)?;
                if !self.regions.contains(parent) {
                    return Ok(());
                }
                id = parent;
            } else {
                self.retire_activation(r.activation)?;
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
        let depth = self.regions.get(caller.region)?.depth;
        if n.checked_add(1)
            .is_none_or(|regions| !self.regions.has_room(n, regions, &self.options.limits))
            || depth.saturating_add(2) > self.options.limits.max_region_depth
        {
            return self.fail(RunError::ResourceLimit);
        }
        // Each branch beyond the first is extra ownership work for the pump budget.
        self.regions.charge(n.saturating_sub(1));
        self.regions.reserve(n + 1);
        let group = self.regions.mint_region()?;
        let mut children = std::mem::take(&mut self.fork_children);
        children.reserve(n);
        for target in branches {
            let region = self.regions.mint_region()?;
            let mut child = self.child_strand(&caller, region)?;
            child.pc = *target;
            children.push(child);
        }
        caller.pc = *completed;
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
        self.regions.open(
            group,
            Region {
                frames: None,
                retired_cursor: None,
                parent: Some(parent),
                activation,
                depth: depth + 1,
                live: IdSet::default(),
                children: IdSet::default(),
                continuation,
                kind: RegionKind::Join {
                    owner,
                    mode: *join,
                    fired: detach,
                },
            },
        )?;
        for child in &children {
            self.regions.open(
                child.region,
                Region {
                    frames: None,
                    retired_cursor: None,
                    parent: Some(group),
                    activation,
                    depth: depth + 2,
                    live: IdSet::one(child.id),
                    children: IdSet::default(),
                    continuation: None,
                    kind: RegionKind::Branch,
                },
            )?;
        }
        for child in children.drain(..).rev() {
            self.ready.push_front(child.id, child);
        }
        self.fork_children = children;
        self.segment_owner = None;
        Ok(())
    }
    fn release_first_join(&mut self, id: RegionId) -> Result<(), RunError> {
        let r = self.regions.get_mut(id)?;
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
            node: self.plan.as_ref().ok_or(RunError::InvalidState)?.nodes()[owner.index()].node(),
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
    pub(super) fn cancel_region(&mut self, root: RegionId) -> Result<(), RunError> {
        // Reuse the actor's traversal buffers; they are empty between calls.
        let mut ids = std::mem::take(&mut self.cancel_regions);
        let mut timer_keys = std::mem::take(&mut self.cancel_timers);
        let result = self.cancel_owned_regions(root, &mut ids, &mut timer_keys);
        ids.clear();
        timer_keys.clear();
        self.cancel_regions = ids;
        self.cancel_timers = timer_keys;
        result
    }
    fn cancel_owned_regions(
        &mut self,
        root: RegionId,
        ids: &mut Vec<RegionId>,
        timer_keys: &mut Vec<(u64, u64)>,
    ) -> Result<(), RunError> {
        // Traverse only owned descendants. Logical strand IDs may also identify
        // held serial continuations; only one queued/timed strand has that ID.
        let retired = self.regions.subtree(root, ids)?;
        for &id in ids.iter() {
            for strand in self.regions.get(id)?.live.iter() {
                if let Some(queued) = self.ready.remove(strand) {
                    self.cleanup_truncated = self
                        .cleanup_truncated
                        .saturating_add(u64::from(queued.cleanup));
                }
                if let Some(key) = self.timers.detach(strand) {
                    timer_keys.push(key);
                }
            }
        }
        // Timer keys are unique, so sorting reproduces the ordered-set walk.
        timer_keys.sort_unstable();
        let mut cleanup = Vec::new();
        let plan = self.plan.as_ref().ok_or(RunError::InvalidState)?.clone();
        // Timer keys preserve the original cancellation/outcome order.
        for &key in timer_keys.iter() {
            let mut timer = self.timers.remove(key).ok_or(RunError::InvalidState)?;
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
                s.transfer = Transfer::None;
                s.yielded = false;
                s.cleanup = true;
                let deadline = self.now.saturating_add(self.options.cleanup_timeout_ns);
                s.cleanup_deadline = Some(self.deadline.map_or(deadline, |old| old.min(deadline)));
                s.cleanup_remaining = self.options.max_cleanup_operations;
                cleanup.push(timer.activation);
            }
        }
        let activation =
            self.regions
                .prune(ids, retired, cleanup.iter().map(|s| s.id).collect())?;
        if let Some(shared) = self
            .activations
            .get_mut(activation)
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
    pub(super) fn unwind_loop(
        &mut self,
        strand: Strand,
        region: RegionId,
        is_break: bool,
    ) -> Result<(), RunError> {
        let mut caller = self
            .regions
            .get_mut(region)?
            .continuation
            .take()
            .ok_or(RunError::InvalidState)?;
        caller.pull.recycle(strand.pull);
        let owner = match self.regions.get(region)?.kind {
            RegionKind::Body {
                loop_owner: Some(owner),
            } => owner,
            _ => return Err(RunError::InvalidState),
        };
        if is_break {
            let retired = caller.stack.pop().ok_or(RunError::InvalidState)?;
            if !matches!(retired, Cursor::ForEach(_)) {
                self.regions.get_mut(region)?.retired_cursor = Some(Box::new(retired));
            }
            caller.pc = match self.plan.as_ref().ok_or(RunError::InvalidState)?.nodes()
                [owner.index()]
            .operation()
            {
                ControlOperation::Count { completed, .. }
                | ControlOperation::While { completed, .. }
                | ControlOperation::ForEach { completed, .. } => *completed,
                _ => return Err(RunError::InvalidState),
            };
        } else {
            caller.yielded = true;
        }
        self.regions.get_mut(region)?.continuation = Some(caller);
        self.cancel_region(region)?;
        self.complete_regions(region)
    }
    pub(super) fn retire_failed(&mut self, strand: Strand) -> Result<(), RunError> {
        let root = self.regions.root_of(strand.region)?;
        self.cancel_region(root)?;
        self.complete_regions(root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn wide_spills_compact_tombstones_and_keep_appends_ordered() {
        let mut actual = IdSet::<u64>::default();
        let mut expected = BTreeSet::new();
        // Fork-shaped: minted children append, then complete in arbitrary order.
        for id in 0..4_000 {
            assert!(actual.insert(id));
            expected.insert(id);
        }
        let mut random = 7u64;
        for _ in 0..3_990 {
            random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
            let id = (random >> 33) % 4_000;
            assert_eq!(actual.remove(&id), expected.remove(&id));
            if let IdSet::Many(spill) = &actual {
                assert!(
                    spill.ids.len() <= 2 * spill.live.max(1),
                    "tombstones compact"
                );
            }
        }
        // Re-inserting older identities, live or removed, keeps one ordered copy.
        for id in [3_999, 0, 2_000, 1, 3_999] {
            assert_eq!(actual.insert(id), expected.insert(id));
        }
        assert!(actual.iter().eq(expected.iter().copied()));
        assert_eq!(actual.len(), expected.len());
        let collected: IdSet<u64> = [9, 3, 9, 1, 3].into_iter().collect();
        assert!(collected.iter().eq([1, 3, 9]));
        let single: IdSet<u64> = [5, 5].into_iter().collect();
        assert!(matches!(single, IdSet::One(5)));
    }

    #[test]
    fn inline_ownership_ids_preserve_order_through_spill_and_removal() {
        assert!(std::mem::size_of::<IdSet<u64>>() <= 2 * std::mem::size_of::<u64>());
        let mut actual = IdSet::<u64>::default();
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
