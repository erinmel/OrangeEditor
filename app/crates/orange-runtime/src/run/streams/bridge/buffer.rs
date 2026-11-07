use super::*;
enum Admission {
    Closed,
    Drop,
    Queue { evict: usize, bytes: usize },
}
impl Hub {
    /// All branch policies, allocation and joint nominal growth are admitted
    /// before any queue/counter changes. Staging is transferred, never duplicated.
    pub(super) fn fanout(
        &mut self,
        value: &Value,
        bytes: usize,
        budget: &Budget,
    ) -> Result<PushAdmission, StreamError> {
        if bytes > self.options.max_bytes {
            return Err(StreamError::Limit);
        }
        let mut actions = Vec::with_capacity(self.branches.len());
        let mut old_total = 0usize;
        let mut new_total = 0usize;
        for branch in &self.branches {
            let old = branch.retained.get();
            old_total = old_total.checked_add(old).ok_or(StreamError::Limit)?;
            let action = if !branch.live() {
                Admission::Closed
            } else {
                let full = branch.queue.len() >= self.options.capacity
                    || old
                        .checked_add(bytes)
                        .is_none_or(|n| n > self.options.max_bytes);
                match (full, self.options.overflow) {
                    (true, StreamOverflow::Backpressure) => return Err(StreamError::Full),
                    (true, StreamOverflow::Fault) => return Err(StreamError::Overflow),
                    (true, StreamOverflow::DropNewest) => Admission::Drop,
                    _ => {
                        let mut evict = 0;
                        let mut retained = old;
                        while branch.queue.len() - evict >= self.options.capacity
                            || retained
                                .checked_add(bytes)
                                .is_none_or(|n| n > self.options.max_bytes)
                        {
                            retained -= branch.queue.get(evict).ok_or(StreamError::Limit)?.1;
                            evict += 1;
                        }
                        Admission::Queue {
                            evict,
                            bytes: retained + bytes,
                        }
                    }
                }
            };
            new_total = new_total
                .checked_add(match action {
                    Admission::Queue { bytes, .. } => bytes,
                    _ => old,
                })
                .ok_or(StreamError::Limit)?;
            actions.push(action);
        }
        let credit = old_total
            .checked_add(self.retained.get())
            .ok_or(StreamError::Limit)?;
        if !budget.admits(new_total.saturating_sub(credit)) {
            return Err(StreamError::Limit);
        }
        for (branch, action) in self.branches.iter_mut().zip(&actions) {
            if matches!(action, Admission::Queue { evict: 0, .. }) {
                branch
                    .queue
                    .try_reserve(1)
                    .map_err(|_| StreamError::Limit)?;
            }
        }
        let _ = self.retained.try_set(0);
        // Apply every shrink first, so a sibling's admitted growth can use it.
        for (branch, action) in self.branches.iter_mut().zip(&actions) {
            if let Admission::Queue { bytes, .. } = action
                && *bytes < branch.retained.get()
            {
                let _ = branch.retained.try_set(*bytes);
            }
        }
        let mut dropped = false;
        for (branch, action) in self.branches.iter_mut().zip(actions) {
            match action {
                Admission::Closed => {}
                Admission::Drop => {
                    branch.accepted = branch.accepted.saturating_add(1);
                    branch.dropped = branch.dropped.saturating_add(1);
                    dropped = true;
                }
                Admission::Queue {
                    evict,
                    bytes: total,
                } => {
                    if !branch.retained.try_set(total) {
                        unreachable!("joint queue growth was admitted before mutation");
                    }
                    for _ in 0..evict {
                        let _ = branch.queue.pop_front();
                    }
                    branch.dropped = branch.dropped.saturating_add(evict as u64);
                    branch.accepted = branch.accepted.saturating_add(1);
                    branch.queue.push_back((value.clone(), bytes));
                }
            }
        }
        self.staged = None;
        Ok(if dropped {
            PushAdmission::Dropped
        } else {
            PushAdmission::Accepted
        })
    }
}
