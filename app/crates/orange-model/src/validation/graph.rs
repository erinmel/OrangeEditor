//! Private ordered, iterative graph traversal; callers retain language rules and witnesses.
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn strongly_connected_components<T: Ord + Clone>(
    graph: &BTreeMap<T, BTreeSet<T>>,
) -> Vec<BTreeSet<T>> {
    let mut visited = BTreeSet::new();
    let mut finish = Vec::with_capacity(graph.len());
    for start in graph.keys() {
        if visited.contains(start) {
            continue;
        }
        visited.insert(start.clone());
        let neighbors: Vec<_> = graph
            .get(start)
            .into_iter()
            .flat_map(|items| items.iter().cloned())
            .collect();
        let mut stack = vec![(start.clone(), neighbors, 0_usize)];
        while let Some((vertex, neighbors, cursor)) = stack.last_mut() {
            if *cursor >= neighbors.len() {
                finish.push(vertex.clone());
                stack.pop();
                continue;
            }
            let next = neighbors[*cursor].clone();
            *cursor += 1;
            if visited.insert(next.clone()) {
                let next_neighbors: Vec<_> = graph
                    .get(&next)
                    .into_iter()
                    .flat_map(|items| items.iter().cloned())
                    .collect();
                stack.push((next, next_neighbors, 0));
            }
        }
    }

    let mut reverse = BTreeMap::<T, BTreeSet<T>>::new();
    for (from, neighbors) in graph {
        reverse.entry(from.clone()).or_default();
        for to in neighbors {
            reverse.entry(to.clone()).or_default().insert(from.clone());
        }
    }

    visited.clear();
    let mut components = Vec::new();
    for start in finish.into_iter().rev() {
        if !visited.insert(start.clone()) {
            continue;
        }
        let mut component = BTreeSet::new();
        let mut stack = vec![start];
        while let Some(vertex) = stack.pop() {
            component.insert(vertex.clone());
            if let Some(neighbors) = reverse.get(&vertex) {
                for neighbor in neighbors.iter().rev() {
                    if visited.insert(neighbor.clone()) {
                        stack.push(neighbor.clone());
                    }
                }
            }
        }
        components.push(component);
    }
    components
}

/// [`strongly_connected_components`] over dense vertices `0..adjacency.len()` whose neighbour
/// lists ascend: the same components in the same order, each ascending, without cloning or
/// ordered-set lookups per vertex.
pub(super) fn dense_strongly_connected_components(adjacency: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let count = adjacency.len();
    let mut visited = vec![false; count];
    let mut finish = Vec::with_capacity(count);
    let mut stack: Vec<(usize, usize)> = Vec::new();
    for start in 0..count {
        if visited[start] {
            continue;
        }
        visited[start] = true;
        stack.push((start, 0));
        while let Some((vertex, cursor)) = stack.last_mut() {
            let Some(&next) = adjacency[*vertex].get(*cursor) else {
                finish.push(*vertex);
                stack.pop();
                continue;
            };
            *cursor += 1;
            if !visited[next] {
                visited[next] = true;
                stack.push((next, 0));
            }
        }
    }

    // Reverse lists ascend because sources are visited in ascending order.
    let mut reverse = vec![Vec::new(); count];
    for (from, neighbors) in adjacency.iter().enumerate() {
        for &to in neighbors {
            reverse[to].push(from);
        }
    }

    visited.fill(false);
    let mut components = Vec::new();
    let mut pending = Vec::new();
    for start in finish.into_iter().rev() {
        if visited[start] {
            continue;
        }
        visited[start] = true;
        let mut component = Vec::new();
        pending.push(start);
        while let Some(vertex) = pending.pop() {
            component.push(vertex);
            for &neighbor in reverse[vertex].iter().rev() {
                if !visited[neighbor] {
                    visited[neighbor] = true;
                    pending.push(neighbor);
                }
            }
        }
        component.sort_unstable();
        components.push(component);
    }
    components
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]
        #[test]
        fn partitions_match_independent_mutual_reachability(
            count in 1u8..9, edges in prop::collection::vec((0u8..8,0u8..8),0..48)
        ) {
            let mut graph: BTreeMap<_, BTreeSet<_>> = (0..count).map(|v|(v,BTreeSet::new())).collect();
            for (from,to) in edges { graph.get_mut(&(from%count)).unwrap().insert(to%count); }
            let components = strongly_connected_components(&graph);
            let adjacency: Vec<Vec<usize>> = graph
                .values()
                .map(|items| items.iter().map(|&v| usize::from(v)).collect())
                .collect();
            let dense: Vec<BTreeSet<u8>> = dense_strongly_connected_components(&adjacency)
                .into_iter()
                .map(|c| c.into_iter().map(|v| v as u8).collect())
                .collect();
            prop_assert_eq!(&dense, &components);
            let reaches = |from,to| {
                let mut pending=vec![from]; let mut seen=BTreeSet::new();
                while let Some(v)=pending.pop() {
                    if v==to { return true; }
                    if seen.insert(v) { pending.extend(graph[&v].iter().copied()); }
                }
                false
            };
            for from in 0..count {
                prop_assert_eq!(components.iter().filter(|c|c.contains(&from)).count(),1);
                for to in 0..count {
                    let together = components.iter().any(|c|c.contains(&from)&&c.contains(&to));
                    prop_assert_eq!(together,reaches(from,to)&&reaches(to,from));
                }
            }
        }
    }
}
