//! Dominator trees over dense directed graphs, after Cooper, Harvey and Kennedy, "A Simple,
//! Fast Dominance Algorithm". Every traversal is iterative, so graph depth never grows the
//! stack. On the reducible graphs of structured control flow the fixed point takes a few
//! linear passes; irreducible graphs stay correct and only need more passes.

const UNDEFINED: u32 = u32::MAX;

/// Immediate dominators of every vertex reachable from one start vertex.
pub(crate) struct DomTree {
    /// Reverse-postorder position per vertex, or `UNDEFINED` when unreachable.
    order: Vec<u32>,
    /// Tree preorder entry and exit stamps per reverse-postorder position.
    enter: Vec<u32>,
    leave: Vec<u32>,
}

impl DomTree {
    /// `succ[v]` lists the successors of vertex `v`; every listed vertex must be in range.
    pub(crate) fn new(succ: &[Vec<u32>], start: u32) -> Self {
        let rpo = reverse_postorder(succ, start);
        let mut order = vec![UNDEFINED; succ.len()];
        for (position, vertex) in rpo.iter().enumerate() {
            order[*vertex as usize] = position as u32;
        }
        let mut preds = vec![Vec::new(); rpo.len()];
        for (position, vertex) in rpo.iter().enumerate() {
            for next in &succ[*vertex as usize] {
                preds[order[*next as usize] as usize].push(position as u32);
            }
        }
        let idom = immediate_dominators(&preds);
        let (enter, leave) = intervals(&idom);
        Self {
            order,
            enter,
            leave,
        }
    }

    pub(crate) fn reachable(&self, vertex: u32) -> bool {
        self.order[vertex as usize] != UNDEFINED
    }

    /// Every path from the start to `b` passes through `a`. False if either is unreachable.
    pub(crate) fn dominates(&self, a: u32, b: u32) -> bool {
        let (a, b) = (self.order[a as usize], self.order[b as usize]);
        if a == UNDEFINED || b == UNDEFINED {
            return false;
        }
        let (a, b) = (a as usize, b as usize);
        self.enter[a] <= self.enter[b] && self.leave[b] <= self.leave[a]
    }
}

fn reverse_postorder(succ: &[Vec<u32>], start: u32) -> Vec<u32> {
    let mut visited = vec![false; succ.len()];
    let mut post = Vec::new();
    let mut stack = vec![(start, 0usize)];
    visited[start as usize] = true;
    while let Some((vertex, next)) = stack.last_mut() {
        if let Some(child) = succ[*vertex as usize].get(*next).copied() {
            *next += 1;
            if !visited[child as usize] {
                visited[child as usize] = true;
                stack.push((child, 0));
            }
        } else {
            post.push(*vertex);
            stack.pop();
        }
    }
    post.reverse();
    post
}

/// `preds` is indexed by reverse-postorder position, with the start at position 0.
fn immediate_dominators(preds: &[Vec<u32>]) -> Vec<u32> {
    let mut idom = vec![UNDEFINED; preds.len()];
    idom[0] = 0;
    let intersect = |idom: &[u32], mut a: u32, mut b: u32| {
        while a != b {
            while a > b {
                a = idom[a as usize];
            }
            while b > a {
                b = idom[b as usize];
            }
        }
        a
    };
    let mut changed = true;
    while changed {
        changed = false;
        for block in 1..preds.len() {
            // The DFS parent precedes `block` in reverse postorder, so one pred is known.
            let mut known = preds[block]
                .iter()
                .copied()
                .filter(|pred| idom[*pred as usize] != UNDEFINED);
            let Some(first) = known.next() else { continue };
            let new = known.fold(first, |acc, pred| intersect(&idom, pred, acc));
            if idom[block] != new {
                idom[block] = new;
                changed = true;
            }
        }
    }
    idom
}

/// Preorder entry and exit stamps of the dominator tree, for constant-time ancestry.
fn intervals(idom: &[u32]) -> (Vec<u32>, Vec<u32>) {
    let mut children = vec![Vec::new(); idom.len()];
    for (block, parent) in idom.iter().enumerate().skip(1) {
        children[*parent as usize].push(block as u32);
    }
    let (mut enter, mut leave) = (vec![0; idom.len()], vec![0; idom.len()]);
    if idom.is_empty() {
        return (enter, leave);
    }
    let mut clock = 0;
    let mut stack = vec![(0u32, 0usize)];
    enter[0] = clock;
    while let Some((block, next)) = stack.last_mut() {
        if let Some(child) = children[*block as usize].get(*next).copied() {
            *next += 1;
            clock += 1;
            enter[child as usize] = clock;
            stack.push((child, 0));
        } else {
            clock += 1;
            leave[*block as usize] = clock;
            stack.pop();
        }
    }
    (enter, leave)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn graph(vertices: usize, edges: &[(u32, u32)]) -> Vec<Vec<u32>> {
        let mut succ = vec![Vec::new(); vertices];
        for (from, to) in edges {
            succ[*from as usize].push(*to);
        }
        succ
    }

    /// Reference: `a` dominates `b` iff `b` is reachable and removing `a` disconnects it.
    fn brute(succ: &[Vec<u32>], start: u32, a: u32, b: u32) -> bool {
        let reach = |removed: Option<u32>| {
            let mut seen = vec![false; succ.len()];
            let mut stack = vec![start];
            while let Some(vertex) = stack.pop() {
                if Some(vertex) == removed || seen[vertex as usize] {
                    continue;
                }
                seen[vertex as usize] = true;
                stack.extend(succ[vertex as usize].iter().copied());
            }
            seen
        };
        let all = reach(None);
        all[a as usize] && all[b as usize] && (a == b || !reach(Some(a))[b as usize])
    }

    #[test]
    fn diamond_loop_and_irreducible_shapes() {
        let diamond = DomTree::new(&graph(4, &[(0, 1), (0, 2), (1, 3), (2, 3)]), 0);
        assert!(diamond.dominates(0, 3));
        assert!(!diamond.dominates(1, 3) && !diamond.dominates(2, 3));
        let looped = DomTree::new(&graph(4, &[(0, 1), (1, 2), (2, 1), (2, 3)]), 0);
        assert!(looped.dominates(1, 3) && looped.dominates(2, 3) && !looped.dominates(3, 2));
        // Two entries into the 1↔2 cycle: only 1 guards 3.
        let irreducible = graph(4, &[(0, 1), (0, 2), (1, 2), (2, 1), (1, 3)]);
        let tree = DomTree::new(&irreducible, 0);
        assert!(tree.dominates(1, 3) && !tree.dominates(2, 3) && !tree.dominates(2, 1));
    }

    #[test]
    fn unreachable_vertices_neither_dominate_nor_are_dominated() {
        let tree = DomTree::new(&graph(3, &[(0, 1), (0, 0)]), 0);
        assert!(tree.reachable(1) && !tree.reachable(2));
        assert!(!tree.dominates(2, 1) && !tree.dominates(0, 2) && !tree.dominates(2, 2));
        assert!(tree.dominates(1, 1) && tree.dominates(0, 1));
    }

    proptest! {
        #[test]
        fn matches_vertex_removal_reachability(
            vertices in 1usize..9,
            raw in prop::collection::vec((0u32..9, 0u32..9), 0..24),
        ) {
            let edges: Vec<_> = raw
                .into_iter()
                .filter(|(a, b)| (*a as usize) < vertices && (*b as usize) < vertices)
                .collect();
            let succ = graph(vertices, &edges);
            let tree = DomTree::new(&succ, 0);
            for a in 0..vertices as u32 {
                for b in 0..vertices as u32 {
                    prop_assert_eq!(tree.dominates(a, b), brute(&succ, 0, a, b), "{} dom {}", a, b);
                }
            }
        }
    }
}
