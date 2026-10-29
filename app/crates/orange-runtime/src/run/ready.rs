//! Ordered ready work with direct strand removal. Reusable slots keep ordinary
//! front/back scheduling on dense links; hashing is only the identity directory.
use super::ids::StrandId;
use std::collections::HashMap;
struct Link<T> {
    id: StrandId,
    previous: Option<usize>,
    next: Option<usize>,
    value: T,
}
pub(super) struct ReadyQueue<T> {
    nodes: Vec<Option<Link<T>>>,
    free: Vec<usize>,
    index: HashMap<StrandId, usize>,
    single: Option<(StrandId, T)>,
    indexed: bool,
    first: Option<usize>,
    last: Option<usize>,
}
impl<T> ReadyQueue<T> {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            nodes: Vec::with_capacity(if capacity > 1 { capacity } else { 0 }),
            free: Vec::new(),
            index: HashMap::with_capacity(if capacity > 1 { capacity } else { 0 }),
            single: None,
            indexed: false,
            first: None,
            last: None,
        }
    }
    pub fn len(&self) -> usize {
        if self.indexed {
            self.index.len()
        } else {
            usize::from(self.single.is_some())
        }
    }
    pub fn is_empty(&self) -> bool {
        if self.indexed {
            self.first.is_none()
        } else {
            self.single.is_none()
        }
    }
    pub fn front(&self) -> Option<&T> {
        if self.indexed {
            Some(&self.nodes[self.first?].as_ref()?.value)
        } else {
            self.single.as_ref().map(|(_, value)| value)
        }
    }
    fn insert(
        &mut self,
        id: StrandId,
        value: T,
        previous: Option<usize>,
        next: Option<usize>,
    ) -> usize {
        let entry = if self.indexed {
            match self.index.entry(id) {
                std::collections::hash_map::Entry::Vacant(entry) => Some(entry),
                std::collections::hash_map::Entry::Occupied(_) => {
                    panic!("a strand is queued at most once")
                }
            }
        } else {
            None
        };
        let node = Some(Link {
            id,
            previous,
            next,
            value,
        });
        let slot = if let Some(slot) = self.free.pop() {
            self.nodes[slot] = node;
            slot
        } else {
            self.nodes.push(node);
            self.nodes.len() - 1
        };
        if let Some(entry) = entry {
            entry.insert(slot);
        }
        slot
    }
    #[expect(clippy::expect_used, reason = "a linked slot always holds a node")]
    pub fn push_front(&mut self, id: StrandId, value: T) {
        if !self.indexed {
            if let Some((old_id, old)) = self.single.take() {
                assert_ne!(old_id, id, "a strand is queued at most once");
                self.indexed = true;
                self.push_back(old_id, old);
            } else {
                self.single = Some((id, value));
                return;
            }
        }

        let slot = self.insert(id, value, None, self.first);
        if let Some(first) = self.first {
            self.nodes[first].as_mut().expect("linked front").previous = Some(slot);
        } else {
            self.last = Some(slot);
        }
        self.first = Some(slot);
    }
    #[expect(clippy::expect_used, reason = "a linked slot always holds a node")]
    pub fn push_back(&mut self, id: StrandId, value: T) {
        if !self.indexed {
            if let Some((old_id, old)) = self.single.take() {
                assert_ne!(old_id, id, "a strand is queued at most once");
                self.indexed = true;
                self.push_back(old_id, old);
            } else {
                self.single = Some((id, value));
                return;
            }
        }

        let slot = self.insert(id, value, self.last, None);
        if let Some(last) = self.last {
            self.nodes[last].as_mut().expect("linked back").next = Some(slot);
        } else {
            self.first = Some(slot);
        }
        self.last = Some(slot);
    }
    #[expect(clippy::expect_used, reason = "a linked slot always holds a node")]
    fn take(&mut self, slot: usize) -> Option<T> {
        let node = self.nodes[slot].take()?;
        if let Some(previous) = node.previous {
            self.nodes[previous]
                .as_mut()
                .expect("linked predecessor")
                .next = node.next;
        } else {
            self.first = node.next;
        }
        if let Some(next) = node.next {
            self.nodes[next]
                .as_mut()
                .expect("linked successor")
                .previous = node.previous;
        } else {
            self.last = node.previous;
        }
        self.free.push(slot);
        Some(node.value)
    }
    pub fn pop_front(&mut self) -> Option<T> {
        if !self.indexed {
            return self.single.take().map(|(_, value)| value);
        }
        let first = self.first?;
        if self.indexed {
            self.index.remove(&self.nodes[first].as_ref()?.id);
        }
        self.take(first)
    }
    pub fn remove(&mut self, id: StrandId) -> Option<T> {
        if !self.indexed {
            if self.single.as_ref().is_none_or(|(queued, _)| *queued != id) {
                return None;
            }
            return self.single.take().map(|(_, value)| value);
        }
        let slot = self.index.remove(&id)?;
        self.take(slot)
    }
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.free.clear();
        self.index.clear();
        self.single = None;
        self.indexed = false;
        self.first = None;
        self.last = None;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    #[test]
    fn indexed_removal_preserves_order_and_reuses_retired_identities() {
        let mut queue = ReadyQueue::with_capacity(16);
        let mut reference = VecDeque::new();
        let mut random = 19u64;
        for _ in 0..20_000 {
            random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
            let id = random >> 32 & 63;
            match random & 7 {
                0 | 1 if !reference.contains(&id) => {
                    queue.push_front(StrandId::root(id), id);
                    reference.push_front(id);
                }
                2 | 3 if !reference.contains(&id) => {
                    queue.push_back(StrandId::root(id), id);
                    reference.push_back(id);
                }
                4 | 5 => {
                    assert_eq!(
                        queue.remove(StrandId::root(id)),
                        reference
                            .iter()
                            .position(|&n| n == id)
                            .and_then(|i| reference.remove(i))
                    );
                }
                6 => assert_eq!(queue.pop_front(), reference.pop_front()),
                7 => {
                    queue.clear();
                    reference.clear();
                }
                _ => {}
            }
            let mut actual = Vec::new();
            if let Some((_, value)) = &queue.single {
                actual.push(*value);
            }
            let mut current = queue.first;
            let mut previous = None;
            while let Some(id) = current {
                let node = queue.nodes[id].as_ref().unwrap();
                assert_eq!(node.previous, previous);
                actual.push(node.value);
                previous = current;
                current = node.next;
                assert!(actual.len() <= 64);
            }
            assert_eq!(queue.last, previous);
            assert_eq!(actual, reference.iter().copied().collect::<Vec<_>>());
            assert_eq!(queue.len(), reference.len());
            assert_eq!(queue.nodes.len(), queue.free.len() + queue.index.len());
            assert_eq!(queue.front(), reference.front());
        }
    }
}
