//! Host-input dependency sets of checked data nodes and demand roots.
use super::*;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Merge storage reused across every union of one control plan.
#[derive(Default)]
pub(super) struct UnionScratch {
    heads: BinaryHeap<Reverse<(usize, usize, usize)>>,
    merged: Vec<usize>,
}

/// For each data node, the sorted host inputs its value transitively depends on.
pub(super) fn host_dependencies(
    data: &DataPlan,
    options: CompileOptions<'_>,
    items: &mut usize,
    scratch: &mut UnionScratch,
) -> Result<Vec<Arc<[usize]>>, CompileError> {
    let host_indices: BTreeMap<_, _> = data
        .host_inputs()
        .iter()
        .enumerate()
        .map(|(i, n)| (n.index(), i))
        .collect();
    let mut dependencies: Vec<Arc<[usize]>> = Vec::with_capacity(data.nodes().len());
    for node in data.nodes() {
        options.checkpoint()?;
        let inputs = if let Some(index) = host_indices.get(&node.index().index()) {
            charge(items, 1, options.limits.max_graph_items)?;
            Arc::from([*index])
        } else {
            let producers = node
                .inputs()
                .iter()
                .filter_map(|i| i.producer)
                .map(|i| {
                    dependencies
                        .get(i.index())
                        .ok_or(CompileError::ValidationIncomplete)
                })
                .collect::<Result<Vec<_>, _>>()?;
            union_inputs(producers, options, items, scratch)?
        };
        dependencies.push(inputs);
    }
    Ok(dependencies)
}

/// Union of strictly increasing host-input sets, sharing the input set when all are
/// identical. Otherwise a k-way merge visits each input element once, in ascending
/// order: one checkpoint per visited element and one item charge per distinct result.
pub(super) fn union_inputs<'s>(
    sets: impl IntoIterator<Item = &'s Arc<[usize]>>,
    options: CompileOptions<'_>,
    items: &mut usize,
    scratch: &mut UnionScratch,
) -> Result<Arc<[usize]>, CompileError> {
    let sets: Vec<_> = sets.into_iter().collect();
    let Some(first) = sets.first() else {
        return Ok(Arc::from([]));
    };
    if sets
        .iter()
        .all(|s| Arc::ptr_eq(s, first) || s.as_ref() == first.as_ref())
    {
        return Ok(Arc::clone(first));
    }
    debug_assert!(sets.iter().all(|s| s.windows(2).all(|w| w[0] < w[1])));
    let UnionScratch { heads, merged } = scratch;
    heads.clear();
    merged.clear();
    for (set, values) in sets.iter().enumerate() {
        if let Some(&value) = values.first() {
            heads.push(Reverse((value, set, 0)));
        }
    }
    while let Some(Reverse((value, set, at))) = heads.pop() {
        options.checkpoint()?;
        if merged.last() != Some(&value) {
            charge(items, 1, options.limits.max_graph_items)?;
            merged.push(value);
        }
        if let Some(&next) = sets[set].get(at + 1) {
            heads.push(Reverse((next, set, at + 1)));
        }
    }
    Ok(Arc::from(merged.as_slice()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The previous set-insertion union, kept as the semantic reference.
    fn reference(
        sets: &[Arc<[usize]>],
        limit: usize,
        items: &mut usize,
    ) -> Result<Arc<[usize]>, CompileError> {
        let Some(first) = sets.first() else {
            return Ok(Arc::from([]));
        };
        if sets.iter().all(|s| s.as_ref() == first.as_ref()) {
            return Ok(Arc::clone(first));
        }
        let mut union = BTreeSet::new();
        for set in sets {
            for &index in set.iter() {
                if union.insert(index) {
                    charge(items, 1, limit)?;
                }
            }
        }
        Ok(union.into_iter().collect::<Vec<_>>().into())
    }

    fn options(limit: usize) -> CompileOptions<'static> {
        let mut options = CompileOptions::default();
        options.limits.max_graph_items = limit;
        options
    }

    fn check(sets: &[Arc<[usize]>], limit: usize, scratch: &mut UnionScratch) {
        let (mut expected_items, mut actual_items) = (3, 3);
        let expected = reference(sets, limit, &mut expected_items);
        let actual = union_inputs(sets, options(limit), &mut actual_items, scratch);
        match (expected, actual) {
            (Ok(expected), Ok(actual)) => {
                assert_eq!(expected, actual, "{sets:?}");
                assert_eq!(expected_items, actual_items, "{sets:?}");
                if sets.iter().all(|s| s == &sets[0]) && !sets.is_empty() {
                    assert!(Arc::ptr_eq(&actual, &sets[0]));
                }
            }
            (Err(expected), Err(actual)) => assert_eq!(expected, actual),
            (expected, actual) => panic!("{sets:?}: {expected:?} != {actual:?}"),
        }
    }

    #[test]
    fn merge_matches_set_union_semantics() {
        let set = |values: &[usize]| Arc::<[usize]>::from(values);
        let shared = set(&[1, 4, 9]);
        let mut scratch = UnionScratch::default();
        let fixed: [Vec<Arc<[usize]>>; 7] = [
            vec![],
            vec![set(&[])],
            vec![set(&[]), set(&[])],
            vec![shared.clone(), shared.clone(), set(&[1, 4, 9])],
            vec![set(&[]), set(&[2, 3])],
            vec![set(&[1, 3, 5]), set(&[2, 3, 6]), set(&[0, 5, 7])],
            vec![set(&[1, 2]), set(&[1, 2, 3])],
        ];
        for sets in &fixed {
            for limit in [3, 4, 5, 100] {
                check(sets, limit, &mut scratch);
            }
        }
        // Deterministic overlapping sets, reusing one scratch across every case.
        let mut state = 0x2545_f491_4f6c_dd1d_u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            usize::try_from(state % 64).unwrap()
        };
        for case in 0..500 {
            let sets: Vec<Arc<[usize]>> = (0..1 + case % 5)
                .map(|_| {
                    let values: BTreeSet<_> = (0..next() % 12).map(|_| next()).collect();
                    values.into_iter().collect::<Vec<_>>().into()
                })
                .collect();
            check(&sets, 3 + next() % 40, &mut scratch);
        }
    }
}
