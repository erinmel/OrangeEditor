//! Copy and paste of nodes. What is copied is plain text, so it can go through the
//! system clipboard and be pasted into another document. It holds the source document
//! and the copied node IDs; pasting copies them with the model's remapping (12 §2), so
//! pasted nodes, wires and declarations get IDs of their own and a pasted variable
//! node in the same document keeps naming the same variable.
//!
//! The text holds what the document holds: settings and references, never a secret.
use super::*;
use orange_model::document::BodyRef;
use orange_model::paste::Fragment;
use serde_json::{Value as Json, json};

/// What marks clipboard text as copied nodes, and its version.
const CLIP_FORMAT: &str = "orange-nodes";

use draft::refused;

impl Host {
    /// Copies nodes of a document's top level as text for the clipboard. Wires between
    /// them, and the declarations they name, go with them.
    pub fn copy(&self, draft: &Draft, nodes: &[NodeId]) -> Result<String, HostError> {
        let saved: Json = serde_json::from_str(&draft.text()?).map_err(|error| refused(&error))?;
        let ids: Vec<String> = nodes.iter().map(ToString::to_string).collect();
        Ok(
            json!({"format": CLIP_FORMAT, "version": 1, "document": saved, "nodes": ids})
                .to_string(),
        )
    }

    /// Pastes text from [`Host::copy`] into a document, the copied nodes keeping their
    /// places relative to each other with the top-left one at `at`, as one edit.
    /// Returns the new nodes' IDs. Text that is not copied nodes is refused.
    pub fn paste(
        &self,
        draft: &mut Draft,
        clip: &str,
        at: [f32; 2],
    ) -> Result<Vec<NodeId>, HostError> {
        let not_nodes = || refused(&"the clipboard holds no copied nodes");
        let clip: Json = serde_json::from_str(clip).map_err(|_| not_nodes())?;
        if clip["format"] != CLIP_FORMAT || clip["version"] != 1 {
            return Err(not_nodes());
        }
        let source = self.linked(&clip["document"].to_string())?;
        let copied: Vec<NodeId> = clip["nodes"]
            .as_array()
            .ok_or_else(not_nodes)?
            .iter()
            .map(|id| {
                id.as_str()
                    .and_then(|id| id.parse().ok())
                    .ok_or_else(not_nodes)
            })
            .collect::<Result<_, _>>()?;
        let registry = self.store.current();
        let fragment =
            Fragment::copy(&source, registry, &copied).map_err(|error| refused(&error))?;
        let mut count = 0u32;
        let mut paste = {
            let target: &Draft = draft;
            fragment
                .paste(target.document(), BodyRef::Root, || {
                    count += 1;
                    let id = target.fresh_id(&format!("paste {count}"));
                    id.parse::<uuid::Uuid>().unwrap_or_default()
                })
                .map_err(|error| refused(&error))?
        };
        if paste.commands.is_empty() {
            return Ok(Vec::new());
        }
        // Places: the copied layout, moved so its top-left corner is at `at`.
        let layout = &clip["document"]["editor"]["nodes"];
        let place_of = |node: &NodeId| -> Option<[f64; 2]> {
            let place = &layout[node.to_string()];
            Some([place["x"].as_f64()?, place["y"].as_f64()?])
        };
        let corner = paste
            .nodes
            .keys()
            .filter_map(place_of)
            .fold([f64::INFINITY, f64::INFINITY], |corner, [x, y]| {
                [corner[0].min(x), corner[1].min(y)]
            });
        let mut placed = Vec::with_capacity(paste.nodes.len());
        let mut fresh = Vec::with_capacity(paste.nodes.len());
        for (index, (old, new)) in paste.nodes.iter().enumerate() {
            #[expect(clippy::cast_possible_truncation, reason = "screen coordinates")]
            let spot = match place_of(old) {
                Some([x, y]) if corner[0].is_finite() => [
                    at[0] + (x - corner[0]) as f32,
                    at[1] + (y - corner[1]) as f32,
                ],
                // Nodes without a place go down from `at`, one under another.
                _ => [at[0], at[1] + 120.0 * index as f32],
            };
            placed.push((*new, spot));
            fresh.push(*new);
        }
        // Keep the source's order, which a front end shows them in.
        fresh.sort_by_key(|new| {
            copied
                .iter()
                .position(|old| paste.nodes.get(old) == Some(new))
                .unwrap_or(usize::MAX)
        });
        paste.commands.extend(
            placed
                .into_iter()
                .map(|(node, at)| edit::place_command(node, Some(at))),
        );
        let _ = draft.commit(self.store.current(), std::mem::take(&mut paste.commands))?;
        Ok(fresh)
    }
}
