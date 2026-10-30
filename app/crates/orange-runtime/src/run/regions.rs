//! The run's ownership tree: its regions, the identities they and their strands receive and
//! the live strand count. Only this module changes them, so the tree invariants hold in
//! one place: a parent lists each of its regions as a child, identities are never reused,
//! and the live count equals the strands that regions hold.
use super::RunError;
use super::RunLimits;
use super::ids::{FrameId, OwnerIds, RegionId, StrandId};
use super::ownership::{IdSet, Region, RegionKind};
use orange_compiler::ControlIx;
use std::collections::BTreeMap;

pub(super) struct Regions {
    map: BTreeMap<RegionId, Region>,
    ids: OwnerIds,
    live_strands: usize,
    /// Ownership work beyond one unit per step, not yet spent from a pump budget.
    work: usize,
}

impl Regions {
    /// An empty tree whose minted identities start above `activations` root identities.
    pub fn after_roots(activations: u64) -> Self {
        Self {
            map: BTreeMap::new(),
            ids: OwnerIds::after_roots(activations),
            live_strands: 0,
            work: 0,
        }
    }
    pub fn len(&self) -> usize {
        self.map.len()
    }
    pub fn live_strands(&self) -> usize {
        self.live_strands
    }
    pub fn contains(&self, id: RegionId) -> bool {
        self.map.contains_key(&id)
    }
    pub fn get(&self, id: RegionId) -> Result<&Region, RunError> {
        self.map.get(&id).ok_or(RunError::InvalidState)
    }
    pub fn get_mut(&mut self, id: RegionId) -> Result<&mut Region, RunError> {
        self.map.get_mut(&id).ok_or(RunError::InvalidState)
    }
    /// Whether `strands` more live strands and `regions` more regions fit the limits, with
    /// identities left for both.
    pub fn has_room(&self, strands: usize, regions: usize, limits: &RunLimits) -> bool {
        strands <= limits.max_strands.saturating_sub(self.live_strands)
            && regions <= limits.max_regions.saturating_sub(self.map.len())
            && strands
                .checked_add(regions)
                .and_then(|n| u64::try_from(n).ok())
                .is_some_and(|n| self.ids.has_room(n))
    }
    /// Records `units` of ownership work for the pump to spend.
    pub fn charge(&mut self, units: usize) {
        self.work = self.work.saturating_add(units);
    }
    /// The ownership work recorded since the last call.
    pub fn take_work(&mut self) -> usize {
        std::mem::take(&mut self.work)
    }
    pub fn mint_strand(&mut self) -> Result<StrandId, RunError> {
        self.ids.strand()
    }
    pub fn mint_region(&mut self) -> Result<RegionId, RunError> {
        self.ids.region()
    }
    /// Adds a region; a child region is also listed under its parent. Its live strands
    /// join the live count.
    pub fn open(&mut self, id: RegionId, region: Region) -> Result<(), RunError> {
        if let Some(parent) = region.parent {
            self.get_mut(parent)?.children.insert(id);
        }
        self.live_strands = self
            .live_strands
            .checked_add(region.live.len())
            .ok_or(RunError::InvalidState)?;
        self.map.insert(id, region);
        Ok(())
    }
    /// Removes `strand` from `region`'s live strands and returns the region.
    pub fn retire_strand(
        &mut self,
        region: RegionId,
        strand: StrandId,
    ) -> Result<&mut Region, RunError> {
        let r = self.map.get_mut(&region).ok_or(RunError::InvalidState)?;
        if !r.live.remove(&strand) {
            return Err(RunError::InvalidState);
        }
        self.live_strands = self
            .live_strands
            .checked_sub(1)
            .ok_or(RunError::InvalidState)?;
        Ok(r)
    }
    /// Removes `id` once it holds no live strands and no child regions. The caller then
    /// detaches it from its parent with [`detach`](Self::detach).
    pub fn take_if_done(&mut self, id: RegionId) -> Result<Option<Region>, RunError> {
        let r = self.get(id)?;
        if !r.live.is_empty() || !r.children.is_empty() {
            return Ok(None);
        }
        Ok(self.map.remove(&id))
    }
    /// Removes a completed `child` from `parent`'s children.
    pub fn detach(&mut self, parent: RegionId, child: RegionId) -> Result<(), RunError> {
        if self.get_mut(parent)?.children.remove(&child) {
            Ok(())
        } else {
            Err(RunError::InvalidState)
        }
    }
    /// Every root region, in identity order.
    pub fn roots(&self) -> Vec<RegionId> {
        self.map
            .iter()
            .filter(|(_, r)| r.parent.is_none())
            .map(|(id, _)| *id)
            .collect()
    }
    /// The root region above `id`.
    pub fn root_of(&self, mut id: RegionId) -> Result<RegionId, RunError> {
        while let Some(parent) = self.get(id)?.parent {
            id = parent;
        }
        Ok(id)
    }
    /// The nearest region at or above `id` that is the body of the loop `owner`.
    pub fn loop_region(&self, mut id: RegionId, owner: ControlIx) -> Result<RegionId, RunError> {
        loop {
            let r = self.get(id)?;
            if matches!(r.kind, RegionKind::Body { loop_owner: Some(active) } if active == owner) {
                return Ok(id);
            }
            id = r.parent.ok_or(RunError::InvalidState)?;
        }
    }
    /// Fills `ids` with `root` and its descendants, breadth first, and returns how many
    /// live strands they hold. `ids` must be empty.
    pub fn subtree(&self, root: RegionId, ids: &mut Vec<RegionId>) -> Result<usize, RunError> {
        ids.push(root);
        let mut next = 0;
        let mut live = 0usize;
        while let Some(&id) = ids.get(next) {
            next += 1;
            let region = self.get(id)?;
            ids.extend(region.children.iter());
            live = live
                .checked_add(region.live.len())
                .ok_or(RunError::InvalidState)?;
        }
        Ok(live)
    }
    /// Removes every region of `subtree` but its first, the root, after [`subtree`]
    /// returned `retired` live strands for it. The root keeps the frames of the removed
    /// function regions until it completes, and `cleanup` becomes its only live strands.
    /// Returns the root's activation.
    ///
    /// [`subtree`]: Self::subtree
    pub fn prune(
        &mut self,
        subtree: &[RegionId],
        retired: usize,
        cleanup: IdSet<StrandId>,
    ) -> Result<u64, RunError> {
        let (&root, descendants) = subtree.split_first().ok_or(RunError::InvalidState)?;
        // One unit per retired descendant region, so a wide cancellation spends the budget.
        self.charge(descendants.len());
        let mut retained_frames = IdSet::default();
        for &id in descendants {
            if let Some(region) = self.map.remove(&id) {
                if matches!(region.kind, RegionKind::Function { .. }) {
                    retained_frames.insert(FrameId::of_region(id));
                }
                for frame in region.frames.iter().flat_map(|frames| frames.iter()) {
                    retained_frames.insert(frame);
                }
            }
        }
        let live = self
            .live_strands
            .checked_sub(retired)
            .and_then(|n| n.checked_add(cleanup.len()))
            .ok_or(RunError::InvalidState)?;
        let r = self.get_mut(root)?;
        for frame in retained_frames.iter() {
            r.frames
                .get_or_insert_with(|| Box::new(IdSet::default()))
                .insert(frame);
        }
        r.live = cleanup;
        r.children.clear();
        let activation = r.activation;
        self.live_strands = live;
        Ok(activation)
    }
    /// Drops every region; identities stay unused.
    pub fn clear(&mut self) {
        self.map.clear();
        self.live_strands = 0;
    }
    /// Drops every region and releases the map's storage.
    pub fn release(&mut self) {
        self.map = BTreeMap::new();
        self.live_strands = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region(parent: Option<RegionId>, live: IdSet<StrandId>) -> Region {
        Region {
            frames: None,
            retired_cursor: None,
            parent,
            activation: 0,
            depth: 0,
            live,
            children: IdSet::default(),
            continuation: None,
            kind: RegionKind::Branch,
        }
    }

    #[test]
    fn the_tree_keeps_children_and_the_live_count_in_step() {
        let limits = RunLimits {
            max_strands: 3,
            max_regions: 3,
            ..RunLimits::default()
        };
        let mut regions = Regions::after_roots(1);
        let root = RegionId::root(0);
        regions
            .open(root, region(None, IdSet::one(StrandId::root(0))))
            .unwrap();
        let child = regions.mint_region().unwrap();
        let strand = regions.mint_strand().unwrap();
        regions
            .open(child, region(Some(root), IdSet::one(strand)))
            .unwrap();
        assert_eq!((regions.len(), regions.live_strands()), (2, 2));
        assert_eq!(
            regions
                .get(root)
                .unwrap()
                .children
                .iter()
                .collect::<Vec<_>>(),
            [child]
        );
        assert!(regions.has_room(1, 1, &limits));
        assert!(!regions.has_room(2, 1, &limits));
        assert_eq!(regions.root_of(child).unwrap(), root);

        // A region with live strands stays; once empty it is taken and detached.
        assert!(regions.take_if_done(child).unwrap().is_none());
        regions.retire_strand(child, strand).unwrap();
        assert!(regions.retire_strand(child, strand).is_err());
        assert!(regions.take_if_done(child).unwrap().is_some());
        regions.detach(root, child).unwrap();
        assert!(regions.detach(root, child).is_err());
        assert_eq!((regions.len(), regions.live_strands()), (1, 1));
    }

    #[test]
    fn pruning_a_subtree_keeps_only_the_root_with_its_cleanup() {
        let mut regions = Regions::after_roots(1);
        let root = RegionId::root(0);
        regions
            .open(root, region(None, IdSet::one(StrandId::root(0))))
            .unwrap();
        let mut ids = Vec::new();
        for _ in 0..3 {
            let id = regions.mint_region().unwrap();
            let strand = regions.mint_strand().unwrap();
            regions
                .open(id, region(Some(root), IdSet::one(strand)))
                .unwrap();
        }
        let retired = regions.subtree(root, &mut ids).unwrap();
        assert_eq!((ids.len(), retired), (4, 4));
        let cleanup = regions.mint_strand().unwrap();
        regions.prune(&ids, retired, IdSet::one(cleanup)).unwrap();
        assert_eq!((regions.len(), regions.live_strands()), (1, 1));
        let r = regions.get(root).unwrap();
        assert!(r.children.is_empty());
        assert_eq!(r.live.iter().collect::<Vec<_>>(), [cleanup]);
    }
}
