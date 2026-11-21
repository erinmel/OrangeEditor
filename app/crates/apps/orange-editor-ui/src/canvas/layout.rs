//! Layout is a topology operation, independent of painting and input handling.
use super::{LAYOUT_GAP, NODE_WIDTH, node_size};
use egui::Pos2;
use orange_host::{GraphView, NodeId};
use std::collections::{BTreeMap, HashMap, VecDeque};

/// Places unsaved nodes in columns along the wires, keeping every saved position.
/// Cycles and their blocked descendants use the columns reached from acyclic roots;
/// nodes sharing a column are stacked in document order. Dangling wires are ignored.
pub fn auto_layout(graph: &GraphView) -> BTreeMap<NodeId, Pos2> {
    if graph.nodes.iter().all(|node| node.position.is_some()) {
        return graph
            .nodes
            .iter()
            .filter_map(|node| node.position.map(|[x, y]| (node.id, Pos2::new(x, y))))
            .collect();
    }
    let indices: HashMap<_, _> = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id, index))
        .collect();
    let mut incoming = vec![0usize; graph.nodes.len()];
    let mut outgoing = vec![Vec::new(); graph.nodes.len()];
    for wire in &graph.wires {
        if let (Some(&from), Some(&to)) = (indices.get(&wire.from.0), indices.get(&wire.to.0)) {
            outgoing[from].push(to);
            incoming[to] += 1;
        }
    }
    let mut ready: VecDeque<_> = incoming
        .iter()
        .enumerate()
        .filter_map(|(index, &count)| (count == 0).then_some(index))
        .collect();
    let mut column = vec![0usize; graph.nodes.len()];
    while let Some(from) = ready.pop_front() {
        for &to in &outgoing[from] {
            column[to] = column[to].max(column[from] + 1);
            incoming[to] -= 1;
            if incoming[to] == 0 {
                ready.push_back(to);
            }
        }
    }
    let mut widths = vec![NODE_WIDTH; graph.nodes.len()];
    for (index, node) in graph.nodes.iter().enumerate() {
        widths[column[index]] = widths[column[index]].max(node_size(node).x);
    }
    let mut starts = vec![0.0; graph.nodes.len()];
    for index in 1..starts.len() {
        starts[index] = starts[index - 1] + widths[index - 1] + LAYOUT_GAP.x;
    }
    let mut next_y = vec![0.0; graph.nodes.len()];
    let mut places = BTreeMap::new();
    for (index, node) in graph.nodes.iter().enumerate() {
        let at = column[index];
        let placed = Pos2::new(starts[at], next_y[at]);
        next_y[at] += node_size(node).y + LAYOUT_GAP.y;
        let place = node.position.map_or(placed, |[x, y]| Pos2::new(x, y));
        let _ = places.insert(node.id, place);
    }
    places
}
