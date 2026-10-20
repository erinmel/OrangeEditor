//! Semantic signature snapshots (12 §3.1): projection, canonical key, implicit ports and
//! drift comparison.
//!
//! A snapshot contains semantics only. Presentation (labels, descriptions, groups, editor
//! hints) is removed before hashing, so a locale or catalogue change never changes a key
//! and can never cause `E-SIGNATURE-DRIFT` (G-39).

use std::collections::BTreeMap;

use serde_json::{Map, Value as Json};
use sha2::{Digest, Sha256};

use crate::document::{
    Extensions, NodeKind, PortDirection, PortKind, ResolvedPort, SignatureSnapshot, TypeParamName,
};
use crate::id::{PortId, SignatureKey, TypeExpr};

/// Presentation-only fields of a port or snapshot (11 §4.1, 12 §3.1).
const PRESENTATION_KEYS: [&str; 4] = ["label", "description", "group", "editor"];

/// Builds the semantic snapshot of resolved definition ports: presentation removed and
/// type parameters replaced by the node's `type_args`.
pub fn project(
    kind: NodeKind,
    ports: impl IntoIterator<Item = ResolvedPort>,
    type_args: &BTreeMap<TypeParamName, TypeExpr>,
) -> SignatureSnapshot {
    let ports = ports
        .into_iter()
        .map(|mut port| {
            strip_presentation(&mut port.extensions);
            port.ty = port.ty.map(|ty| substitute_params(&ty, type_args));
            port
        })
        .collect();
    SignatureSnapshot {
        kind,
        ports,
        extensions: Extensions::new(),
    }
}

fn strip_presentation(extensions: &mut Extensions) {
    for key in PRESENTATION_KEYS {
        extensions.shift_remove(key);
    }
}

/// Replaces each `$name` parameter (11 §5.1) that `args` resolves. Unresolved parameters
/// stay for the validator to report. The 00c type parser will replace this token scan.
pub fn substitute_params(ty: &TypeExpr, args: &BTreeMap<TypeParamName, TypeExpr>) -> TypeExpr {
    let text = ty.as_str();
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('$') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let end = after
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(after.len());
        let name = &after[..end];
        match args.get(name) {
            Some(arg) => out.push_str(arg.as_str()),
            None => {
                out.push('$');
                out.push_str(name);
            }
        }
        rest = &after[end..];
    }
    out.push_str(rest);
    out.parse()
        .expect("substituting non-blank type expressions yields a non-blank one")
}

/// Canonical JSON of a snapshot: object keys sorted by byte order, compact, serde_json
/// string escaping. For the ASCII keys and integer numbers of snapshots this matches
/// RFC 8785; cross-language digest vectors are published with the WU-03 contracts.
pub fn canonical_json(snapshot: &SignatureSnapshot) -> String {
    let value = serde_json::to_value(snapshot).expect("snapshots always serialize");
    sorted(value).to_string()
}

fn sorted(value: Json) -> Json {
    match value {
        Json::Object(map) => {
            let mut entries: Vec<(String, Json)> = map.into_iter().collect();
            entries.sort_by(|(a, _), (b, _)| a.cmp(b));
            Json::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key, sorted(value)))
                    .collect::<Map<_, _>>(),
            )
        }
        Json::Array(items) => Json::Array(items.into_iter().map(sorted).collect()),
        other => other,
    }
}

/// Content-addressed key: identical snapshots share one document entry.
pub fn semantic_key(snapshot: &SignatureSnapshot) -> SignatureKey {
    let digest = Sha256::digest(canonical_json(snapshot).as_bytes());
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("sha256:{hex}")
        .parse()
        .expect("a sha256 key is never blank")
}

impl SignatureSnapshot {
    /// True if `port` is declared or is an implicit port of the snapshot's kind (11 §4.1).
    pub fn declares_port(&self, port: &PortId) -> bool {
        self.ports.iter().any(|declared| &declared.port_id == port)
            || self.implicit_ports().contains(&port.as_str())
    }

    /// Ports a kind gets without declaring them (11 §4.1). Declared exec outputs (outcome
    /// outs) replace `then`.
    pub fn implicit_ports(&self) -> Vec<&'static str> {
        use NodeKind::*;
        let has_outcomes = self
            .ports
            .iter()
            .any(|port| port.direction == PortDirection::Output && port.kind == PortKind::Exec);
        let mut ports = Vec::new();
        if matches!(self.kind, Action | Async | Iterator) {
            ports.extend(["in", "failed", "error"]);
        }
        if matches!(self.kind, Action | Async | Entry) && !has_outcomes {
            ports.push("then");
        }
        if matches!(self.kind, Async | Iterator) {
            ports.push("cancelled");
        }
        if self.kind == Iterator {
            ports.extend(["body", "completed"]);
        }
        ports
    }
}

/// How a stored snapshot differs from the one resolved from the current registry (G-29).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SignatureDrift {
    pub kind_changed: bool,
    /// Declared in the stored snapshot only: edges using them become orphan edges.
    pub removed: Vec<PortId>,
    pub added: Vec<PortId>,
    /// Declared in both with different semantics (type, direction, source, ...).
    pub changed: Vec<PortId>,
}

/// `None` when the snapshots are semantically identical.
pub fn compare(stored: &SignatureSnapshot, resolved: &SignatureSnapshot) -> Option<SignatureDrift> {
    if semantic_key(stored) == semantic_key(resolved) {
        return None;
    }
    let find = |snapshot: &SignatureSnapshot, id: &PortId| {
        snapshot
            .ports
            .iter()
            .find(|port| &port.port_id == id)
            .cloned()
    };
    let mut drift = SignatureDrift {
        kind_changed: stored.kind != resolved.kind,
        ..Default::default()
    };
    for port in &stored.ports {
        match find(resolved, &port.port_id) {
            None => drift.removed.push(port.port_id.clone()),
            Some(other) if other != *port => drift.changed.push(port.port_id.clone()),
            Some(_) => {}
        }
    }
    drift.added = resolved
        .ports
        .iter()
        .filter(|port| find(stored, &port.port_id).is_none())
        .map(|port| port.port_id.clone())
        .collect();
    Some(drift)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn args(pairs: &[(&str, &str)]) -> BTreeMap<TypeParamName, TypeExpr> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.parse().unwrap()))
            .collect()
    }

    fn ty(text: &str) -> TypeExpr {
        text.parse().unwrap()
    }

    #[test]
    fn substitutes_nested_parameters_and_keeps_unknown_ones() {
        let args = args(&[("T", "core/i64@1"), ("K", "core/string@1")]);
        assert_eq!(substitute_params(&ty("$T"), &args), ty("core/i64@1"));
        assert_eq!(
            substitute_params(&ty("core/map@1<$K,core/list@1<$T>>"), &args),
            ty("core/map@1<core/string@1,core/list@1<core/i64@1>>")
        );
        assert_eq!(
            substitute_params(&ty("core/list@1<$U>"), &args),
            ty("core/list@1<$U>")
        );
        assert_eq!(
            substitute_params(&ty("core/bool@1"), &args),
            ty("core/bool@1")
        );
    }

    fn snapshot(value: Json) -> SignatureSnapshot {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn canonical_json_sorts_keys_at_every_level() {
        let snapshot = snapshot(json!({"kind": "pure", "ports": [
            {"port_id": "a", "direction": "input", "kind": "data", "type": "core/i64@1", "z_ext": {"b": 1, "a": 2}}
        ]}));
        assert_eq!(
            canonical_json(&snapshot),
            r#"{"kind":"pure","ports":[{"direction":"input","kind":"data","port_id":"a","type":"core/i64@1","z_ext":{"a":2,"b":1}}]}"#
        );
    }

    #[test]
    fn key_is_sha256_of_canonical_json() {
        // Vector: sha256 of the canonical JSON of the 12 §6 `s-add-i64` snapshot, computed
        // independently with `printf '%s' '<canonical json>' | sha256sum`.
        let add = snapshot(json!({"kind": "pure", "ports": [
            {"port_id": "a", "direction": "input", "kind": "data", "type": "core/i64@1"},
            {"port_id": "b", "direction": "input", "kind": "data", "type": "core/i64@1"},
            {"port_id": "sum", "direction": "output", "kind": "data", "type": "core/i64@1", "source": "computed"}
        ]}));
        assert_eq!(
            semantic_key(&add).as_str(),
            include_str!("../tests/fixtures/s-add-i64.sha256").trim()
        );
    }
}
