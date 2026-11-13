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

/// A length of time written for people, such as `300ms`, `2s`, `1.5s`, `5m` or `1h`, as
/// the nanosecond count the document format uses. `None` for anything else, including
/// a count that is already in nanoseconds.
fn nanoseconds(text: &str) -> Option<String> {
    let (number, unit) = ["ms", "s", "m", "h"]
        .iter()
        .find_map(|unit| Some((text.strip_suffix(unit)?, *unit)))?;
    let scale = match unit {
        "ms" => 1e6,
        "s" => 1e9,
        "m" => 60e9,
        _ => 3600e9,
    };
    let amount: f64 = number
        .parse()
        .ok()
        .filter(|n: &f64| n.is_finite() && *n >= 0.0)?;
    let total = (amount * scale).round();
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "checked to be within range and not negative"
    )]
    (total <= 9.0e18).then(|| (total as u64).to_string())
}

/// Rewrites the plain forms a person writes into the exact forms the format stores:
/// a decimal number as a float literal's bits, and a duration with a unit as
/// nanoseconds, in literals everywhere and in the settings of timer entries. Returns
/// whether anything changed.
fn exact(value: &mut Json) -> bool {
    let mut changed = false;
    match value {
        Json::Object(fields) => {
            let ty = fields
                .get("type")
                .and_then(Json::as_str)
                .unwrap_or_default()
                .to_owned();
            let width = match ty.as_str() {
                "core/f64@1" => 64,
                "core/f32@1" => 32,
                _ => 0,
            };
            if width > 0
                && !fields.contains_key("bits")
                && let Some(number) = fields.get("value").and_then(Json::as_f64)
            {
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "a 32-bit float was asked for"
                )]
                let bits = if width == 64 {
                    format!("{:016x}", number.to_bits())
                } else {
                    format!("{:08x}", (number as f32).to_bits())
                };
                let _ = fields.remove("value");
                let _ = fields.insert("bits".to_owned(), Json::String(bits));
                changed = true;
            }
            if ty == "core/duration@1"
                && let Some(exact) = fields
                    .get("value")
                    .and_then(Json::as_str)
                    .and_then(nanoseconds)
            {
                let _ = fields.insert("value".to_owned(), Json::String(exact));
                changed = true;
            }
            let timer = fields
                .get("definition_id")
                .and_then(Json::as_str)
                .is_some_and(|id| matches!(id, "core/on-timer" | "core/on-timeout"));
            if timer && let Some(Json::Object(config)) = fields.get_mut("config") {
                for setting in config.values_mut() {
                    if let Some(exact) = setting.as_str().and_then(nanoseconds) {
                        *setting = Json::String(exact);
                        changed = true;
                    }
                }
            }
            for field in fields.values_mut() {
                changed |= exact(field);
            }
        }
        Json::Array(items) => {
            for item in items {
                changed |= exact(item);
            }
        }
        _ => {}
    }
    changed
}

impl Host {
    /// Binds the package nodes of a document that are not bound yet: each root node
    /// without a `signature` whose definition is not part of the core language gets
    /// this host's provider for it, the package lock entry and the signature
    /// snapshot, exactly as if it had been placed in an editor. An entry node without
    /// an `activation` gets a queue of sixteen that faults when full. Nodes that are
    /// already bound, and everything else in the document, are left as they are.
    ///
    /// It also accepts the plain forms a person writes: a float literal given as a
    /// number (`{"type": "core/f64@1", "value": 0.15}`), and a duration given with a
    /// unit (`"300ms"`, `"2s"`, `"5m"`, `"1h"`) in a duration literal or in a timer
    /// entry's settings. They are stored in the exact forms the format defines.
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
        let rewritten = exact(&mut saved);
        let nodes = saved["nodes"].as_array().cloned().unwrap_or_default();
        let (unbound, bound): (Vec<Json>, Vec<Json>) = nodes.into_iter().partition(is_unbound);
        if unbound.is_empty() {
            if !rewritten {
                return Ok(json.to_owned());
            }
            let document =
                from_json(&saved.to_string()).map_err(|error| refused(error.to_string()))?;
            return to_json(&document).map_err(|error| refused(error.to_string()));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_numbers_and_durations_become_the_exact_stored_forms() {
        assert_eq!(nanoseconds("300ms").as_deref(), Some("300000000"));
        assert_eq!(nanoseconds("1.5s").as_deref(), Some("1500000000"));
        assert_eq!(nanoseconds("5m").as_deref(), Some("300000000000"));
        assert_eq!(nanoseconds("1h").as_deref(), Some("3600000000000"));
        for other in ["300000000", "fast", "-1s", "s", "1e30h"] {
            assert_eq!(nanoseconds(other), None, "{other}");
        }
        let mut document = json!({"nodes": [
            {"definition_id": "core/on-timer", "config": {"interval": "300ms", "missed": "skip"}},
            {"definition_id": "org.example/x", "config": {"label": "5m"},
                "inputs": {"gain": {"type": "core/f64@1", "value": 0.15},
                    "kept": {"type": "core/f64@1", "bits": "3fc3333333333333"},
                    "wait": {"type": "core/duration@1", "value": "2s"},
                    "count": {"type": "core/i64@1", "value": "7"}}}]});
        assert!(exact(&mut document));
        assert_eq!(
            document["nodes"][0]["config"],
            json!({"interval": "300000000", "missed": "skip"})
        );
        // Only timer entries have their settings read as durations.
        assert_eq!(document["nodes"][1]["config"]["label"], "5m");
        let inputs = &document["nodes"][1]["inputs"];
        assert_eq!(
            inputs["gain"],
            json!({"type": "core/f64@1", "bits": "3fc3333333333333"})
        );
        assert_eq!(inputs["wait"]["value"], "2000000000");
        assert_eq!(inputs["count"]["value"], "7");
        // A second pass finds nothing left to rewrite.
        assert!(!exact(&mut document));
    }
}
