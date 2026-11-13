//! Completing a document written by hand or by another program.
//!
//! A document records, for every package node, the exact provider it is bound to, the
//! package's version and digest, and a snapshot of the node's signature. An editor
//! writes those when a node is placed. A person writing JSON cannot: the values come
//! from the registry. [`Host::bind`] fills them in.
use super::*;
use orange_model::binding::{NewNode, new_node_edit};
use orange_model::document::NodeKind;
use serde_json::{Value as Json, json};

impl Host {
    /// Binds the package nodes of a document that are not bound yet: each root node
    /// without a `signature` whose definition is not part of the core language gets
    /// this host's provider for it, the package lock entry and the signature
    /// snapshot, exactly as if it had been placed in an editor. An entry node without
    /// an `activation` gets a queue of sixteen that faults when full. Nodes that are
    /// already bound, and everything else in the document, are left as they are.
    ///
    /// The result is the completed document text; nothing is opened or run. A node
    /// whose definition this host does not have is refused by name.
    pub fn bind(&self, json: &str) -> Result<String, HostError> {
        let refused = |reason: String| HostError::Document(reason);
        let mut saved: Json =
            serde_json::from_str(json).map_err(|error| refused(error.to_string()))?;
        let is_unbound = |node: &Json| {
            node.get("signature").is_none()
                && node["definition_id"]
                    .as_str()
                    .is_some_and(|id| !id.starts_with("core/"))
        };
        let nodes = saved["nodes"].as_array().cloned().unwrap_or_default();
        let (unbound, bound): (Vec<Json>, Vec<Json>) = nodes.into_iter().partition(is_unbound);
        if unbound.is_empty() {
            return Ok(json.to_owned());
        }
        // The unbound nodes and the edges that touch them are set aside, the rest is
        // read as a document, and the nodes are placed into it one by one.
        let ids: Vec<&str> = unbound
            .iter()
            .filter_map(|node| node["id"].as_str())
            .collect();
        let edges = saved["edges"].as_array().cloned().unwrap_or_default();
        let (held, kept): (Vec<Json>, Vec<Json>) = edges.into_iter().partition(|edge| {
            [&edge["from"]["node"], &edge["to"]["node"]]
                .iter()
                .any(|end| end.as_str().is_some_and(|id| ids.contains(&id)))
        });
        saved["nodes"] = Json::Array(bound);
        saved["edges"] = Json::Array(kept);
        let mut document =
            from_json(&saved.to_string()).map_err(|error| refused(error.to_string()))?;
        let registry = Arc::clone(self.store.current());
        let mut entries = Vec::new();
        for node in &unbound {
            let name = node["id"].as_str().unwrap_or("?").to_owned();
            let field = |what: &str| refused(format!("node {name}: `{what}` is not valid"));
            let part = |key: &str| node.get(key).cloned().unwrap_or_else(|| json!({}));
            let definition = DefinitionRef {
                id: node["definition_id"]
                    .as_str()
                    .and_then(|id| id.parse().ok())
                    .ok_or_else(|| field("definition_id"))?,
                version: node["definition_version"]
                    .as_u64()
                    .and_then(|version| u32::try_from(version).ok())
                    .unwrap_or(1),
            };
            if registry
                .definition(&definition)
                .is_some_and(|found| found.kind == NodeKind::Entry)
            {
                entries.push(name.clone());
            }
            let mut new = NewNode::new(name.parse().map_err(|_| field("id"))?, definition);
            new.config = serde_json::from_value(part("config")).map_err(|_| field("config"))?;
            new.inputs = serde_json::from_value(part("inputs")).map_err(|_| field("inputs"))?;
            new.type_pins =
                serde_json::from_value(part("type_pins")).map_err(|_| field("type_pins"))?;
            let edit = new_node_edit(&document, &registry, new)
                .map_err(|error| refused(format!("node {name}: {error}")))?;
            let _ = document
                .apply(edit.commands)
                .map_err(|error| refused(format!("node {name}: {error}")))?;
        }
        let text = to_json(&document).map_err(|error| refused(error.to_string()))?;
        let mut bound: Json =
            serde_json::from_str(&text).map_err(|error| refused(error.to_string()))?;
        // What the author wrote on a node beyond its binding (an activation, editor
        // data) is kept; the edges set aside go back.
        for placed in bound["nodes"].as_array_mut().into_iter().flatten() {
            let Some(original) = unbound.iter().find(|node| node["id"] == placed["id"]) else {
                continue;
            };
            for (key, value) in original.as_object().into_iter().flatten() {
                if placed.get(key).is_none() {
                    placed[key.as_str()] = value.clone();
                }
            }
            let entry = placed["id"]
                .as_str()
                .is_some_and(|id| entries.iter().any(|entry| entry == id));
            if entry && placed.get("activation").is_none() {
                placed["activation"] = json!({"mode": "queue", "capacity": 16,
                    "overflow": "fault", "on_fault": "continue", "order": 0});
            }
        }
        if let Some(edges) = bound["edges"].as_array_mut() {
            edges.extend(held);
        }
        let complete = from_json(&bound.to_string()).map_err(|error| refused(error.to_string()))?;
        to_json(&complete).map_err(|error| refused(error.to_string()))
    }
}
