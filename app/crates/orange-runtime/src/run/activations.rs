//! Startup roots stay dense; later roots retain only live frames under unique IDs.
//! Semantic identities never double as a growing storage-slot directory.
use super::{ActivationState, RunError};
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct Activations {
    roots: Vec<Option<Box<ActivationState>>>,
    root_live: Vec<bool>,
    dynamic: BTreeMap<u64, Option<Box<ActivationState>>>,
    live_roots: usize,
    high_water: usize,
}
impl Activations {
    pub fn new(roots: Vec<Option<Box<ActivationState>>>) -> Self {
        let live = roots.iter().flatten().count();
        Self {
            root_live: vec![true; roots.len()],
            roots,
            dynamic: BTreeMap::new(),
            live_roots: live,
            high_water: live,
        }
    }
    pub fn len(&self) -> usize {
        self.live_roots + self.dynamic.len()
    }
    pub fn high_water(&self) -> usize {
        self.high_water
    }
    pub fn get(&self, id: u64) -> Option<&Option<Box<ActivationState>>> {
        if let Ok(index) = usize::try_from(id)
            && index < self.roots.len()
        {
            return self.root_live[index].then(|| &self.roots[index]);
        }
        self.dynamic.get(&id)
    }
    pub fn get_mut(&mut self, id: u64) -> Option<&mut Option<Box<ActivationState>>> {
        if let Ok(index) = usize::try_from(id)
            && index < self.roots.len()
        {
            return self.root_live[index].then(|| &mut self.roots[index]);
        }
        self.dynamic.get_mut(&id)
    }
    pub fn insert(
        &mut self,
        id: u64,
        frame: Box<ActivationState>,
        max: usize,
    ) -> Result<(), RunError> {
        if self.len() >= max {
            return Err(RunError::ResourceLimit);
        }
        if id < self.roots.len() as u64 || self.dynamic.contains_key(&id) {
            return Err(RunError::InvalidState);
        }
        self.dynamic.insert(id, Some(frame));
        self.high_water = self.high_water.max(self.len());
        Ok(())
    }
    /// A live owner may have no frame after a provider unwound through its checkout.
    pub fn remove(&mut self, id: u64) -> Result<Option<Box<ActivationState>>, RunError> {
        if let Ok(index) = usize::try_from(id)
            && index < self.roots.len()
        {
            if !self.root_live[index] {
                return Err(RunError::InvalidState);
            }
            self.root_live[index] = false;
            self.live_roots -= 1;
            return Ok(self.roots[index].take());
        }
        self.dynamic.remove(&id).ok_or(RunError::InvalidState)
    }
    pub fn clear(&mut self) {
        self.roots.clear();
        self.root_live.clear();
        self.dynamic.clear();
        self.live_roots = 0;
    }
}
