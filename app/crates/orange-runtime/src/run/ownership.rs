//! Actor-owned region lifetimes. Identities are never reused within a run.
use super::*;

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
    pub live: usize,
    pub children: usize,
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
        {
            return self.fail(RunError::ResourceLimit);
        }
        let region = self.owner_id()?;
        let mut child = self.child_strand(&caller, region)?;
        // A structured body continues the same logical strand. Only Fork
        // creates sibling strand identities.
        child.id = caller.id;
        caller.pc = None;
        self.regions.get_mut(&caller.region).unwrap().children += 1;
        self.regions.insert(
            region,
            Region {
                parent: Some(caller.region),
                activation: caller.sequence,
                depth,
                live: 1,
                children: 0,
                continuation: Some(caller),
                kind: RegionKind::Body { loop_owner },
            },
        );
        self.live_strands += 1;
        self.ready.push_front(child);
        Ok(())
    }
    pub(super) fn child_strand(
        &mut self,
        caller: &Strand,
        region: u64,
    ) -> Result<Strand, RunError> {
        Ok(Strand {
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
        region.live = region.live.checked_sub(1).ok_or(RunError::InvalidState)?;
        self.live_strands -= 1;
        self.complete_regions(strand.region)
    }
    pub(super) fn complete_regions(&mut self, mut id: u64) -> Result<(), RunError> {
        loop {
            let r = self.regions.get(&id).ok_or(RunError::InvalidState)?;
            if r.live != 0 || r.children != 0 {
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
                self.ready.push_front(caller);
            }
            if let Some(parent) = r.parent {
                self.regions
                    .get_mut(&parent)
                    .ok_or(RunError::InvalidState)?
                    .children -= 1;
                self.release_first_join(parent)?;
                if !self.regions.contains_key(&parent) {
                    return Ok(());
                }
                id = parent;
            } else {
                self.activations.remove(&r.activation);
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
                    live: 1,
                    children: 0,
                    continuation: None,
                    kind: RegionKind::Branch,
                },
            );
            children.push(child);
        }
        caller.pc = *completed;
        self.regions.get_mut(&caller.region).unwrap().children += 1;
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
            self.ready.push_front(caller);
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
                live: 0,
                children: n,
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
            self.ready.push_front(child);
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
            let children: Vec<_> = self
                .regions
                .iter()
                .filter(|(_, r)| r.parent == Some(id))
                .map(|(id, _)| *id)
                .collect();
            for child in children {
                self.cancel_region(child)?;
                self.complete_regions(child)?;
            }
        }
        if let Some(caller) = caller {
            self.ready.push_front(caller);
        }
        Ok(())
    }

    /// Retire normal descendants immediately, retaining cancelled wait cleanup
    /// in the same region so ancestors still observe its lifetime barrier.
    pub(super) fn cancel_region(&mut self, root: u64) -> Result<(), RunError> {
        let ids: std::collections::BTreeSet<_> = self
            .regions
            .keys()
            .copied()
            .filter(|id| self.descendant(*id, root))
            .collect();
        self.ready.retain(|s| !ids.contains(&s.region));
        let mut cleanup = Vec::new();
        let timers = std::mem::take(&mut self.timers);
        let plan = self.plan.as_ref().ok_or(RunError::InvalidState)?.clone();
        for (key, mut timer) in timers {
            if !ids.contains(&timer.activation.region) {
                self.timers.insert(key, timer);
                continue;
            }
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
        r.live = cleanup.len();
        r.children = 0;
        let activation = r.activation;
        self.live_strands = self.regions.values().map(|r| r.live).sum();
        if let Some(shared) = self.activations.get_mut(&activation) {
            shared.pull_owner = None;
        }
        for strand in cleanup {
            self.ready.push_back(strand);
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
    pub(super) fn descendant(&self, mut id: u64, ancestor: u64) -> bool {
        loop {
            if id == ancestor {
                return true;
            }
            let Some(parent) = self.regions.get(&id).and_then(|r| r.parent) else {
                return false;
            };
            id = parent;
        }
    }
    pub(super) fn discard_region(&mut self, root: u64) {
        let ids: std::collections::BTreeSet<_> = self
            .regions
            .keys()
            .copied()
            .filter(|id| self.descendant(*id, root))
            .collect();
        self.ready.retain(|s| !ids.contains(&s.region));
        self.timers
            .retain(|_, t| !ids.contains(&t.activation.region));
        for id in ids {
            self.regions.remove(&id);
        }
        self.live_strands = self.regions.values().map(|r| r.live).sum();
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
        let parent = self.regions[&region].parent.ok_or(RunError::InvalidState)?;
        self.discard_region(region);
        self.regions.get_mut(&parent).unwrap().children -= 1;
        if is_break {
            caller.stack.pop().ok_or(RunError::InvalidState)?;
            caller.pc = match self.plan.as_ref().unwrap().nodes()[owner.index()].operation() {
                ControlOperation::Count { completed, .. }
                | ControlOperation::While { completed, .. } => *completed,
                _ => return Err(RunError::InvalidState),
            };
            let shared = self
                .activations
                .get_mut(&strand.sequence)
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
        self.ready.push_front(caller);
        Ok(())
    }
    pub(super) fn retire_failed(&mut self, strand: Strand) -> Result<(), RunError> {
        let mut root = strand.region;
        while let Some(parent) = self.regions[&root].parent {
            root = parent;
        }
        self.discard_region(root);
        self.activations.remove(&strand.sequence);
        Ok(())
    }
}
