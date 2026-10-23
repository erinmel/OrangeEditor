//! Shared, read-only resolution of a graph port reference.

use thiserror::Error;

use crate::document::{
    GraphDocument, NodeKind, PortDescriptor, PortDirection, PortKind, PortRef, SignatureSnapshot,
    Suspends,
};
use crate::id::{PortId, TypeExpr};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(crate) enum PortLookupError {
    #[error("port refers to a missing node")]
    MissingNode,
    #[error("node metadata is unavailable")]
    MissingMetadata,
    #[error("port is not declared by the node")]
    UnknownPort,
}

/// Look up a concrete port using the node's stored semantic snapshot or known intrinsic
/// metadata. Intrinsic metadata is host-owned and deliberately limited to port layouts
/// settled in 11 §8.4.
pub(crate) fn lookup_port(
    document: &GraphDocument,
    port: &PortRef,
) -> Result<PortDescriptor, PortLookupError> {
    let node = document
        .node(port.node)
        .ok_or(PortLookupError::MissingNode)?;
    let definition = node.definition.id.as_str();
    let snapshot = if is_intrinsic_definition(definition) {
        if node.definition.version != 1 {
            return Err(PortLookupError::MissingMetadata);
        }
        intrinsic_snapshot(definition, node.config_version)
            .ok_or(PortLookupError::MissingMetadata)?
    } else {
        let key = node
            .signature
            .as_ref()
            .ok_or(PortLookupError::MissingMetadata)?;
        document
            .signatures()
            .get(key)
            .cloned()
            .ok_or(PortLookupError::MissingMetadata)?
    };

    // Break and Continue terminate the current region; the generic Action default
    // outcome is intentionally absent for these control intrinsics.
    if matches!(definition, "core/break" | "core/continue") && port.port.port().as_str() == "then" {
        return Err(PortLookupError::UnknownPort);
    }

    if let Some(index) = port.port.instance() {
        let state = node
            .variadic
            .get(port.port.port())
            .ok_or(PortLookupError::UnknownPort)?;
        if !state.order.contains(&index) {
            return Err(PortLookupError::UnknownPort);
        }
        let declared = snapshot
            .ports
            .iter()
            .find(|candidate| candidate.port_id == *port.port.port())
            .ok_or(PortLookupError::UnknownPort)?;
        if !declared
            .extensions
            .get("variadic")
            .is_some_and(serde_json::Value::is_object)
        {
            return Err(PortLookupError::UnknownPort);
        }
        return Ok(declared.clone());
    }

    if node.variadic.contains_key(port.port.port())
        || snapshot.ports.iter().any(|p| {
            p.port_id == *port.port.port()
                && p.extensions
                    .get("variadic")
                    .is_some_and(serde_json::Value::is_object)
        })
    {
        return Err(PortLookupError::UnknownPort);
    }
    if let Some(declared) = snapshot
        .ports
        .iter()
        .find(|p| p.port_id == *port.port.port())
    {
        return Ok(declared.clone());
    }
    implicit_port(&snapshot, port.port.port()).ok_or(PortLookupError::UnknownPort)
}

/// Whether an ID is a language intrinsic, independent of whether this slice has a
/// complete descriptor for it yet. Later validator passes use this to avoid treating
/// arbitrary `core/` IDs as trusted intrinsics.
pub(crate) use crate::document::is_intrinsic_definition;

type IntrinsicPortSpec<'a> = (&'a str, &'a str, &'a str, Option<&'a str>, Option<&'a str>);

pub(crate) fn intrinsic_snapshot(id: &str, config_version: u32) -> Option<SignatureSnapshot> {
    if config_version != 1 {
        return None;
    }
    let (kind, specs): (NodeKind, &[IntrinsicPortSpec<'_>]) = match id {
        "core/branch" => (
            NodeKind::Action,
            &[
                ("in", "input", "exec", None, None),
                ("condition", "input", "data", Some("core/bool@1"), None),
                ("true", "output", "exec", None, None),
                ("false", "output", "exec", None, None),
            ],
        ),
        "core/sequence" => (NodeKind::Action, &[("in", "input", "exec", None, None)]),
        "core/fork" => (
            NodeKind::Action,
            &[
                ("in", "input", "exec", None, None),
                ("completed", "output", "exec", None, None),
            ],
        ),
        "core/count" => (
            NodeKind::Iterator,
            &[
                ("in", "input", "exec", None, None),
                ("count", "input", "data", Some("core/i64@1"), None),
                (
                    "index",
                    "output",
                    "data",
                    Some("core/i64@1"),
                    Some("iteration"),
                ),
            ],
        ),
        "core/while" => (
            NodeKind::Iterator,
            &[
                ("in", "input", "exec", None, None),
                ("condition", "input", "data", Some("core/bool@1"), None),
            ],
        ),
        "core/yield" => (NodeKind::Async, &[("in", "input", "exec", None, None)]),
        "core/wait" => (
            NodeKind::Async,
            &[
                ("in", "input", "exec", None, None),
                ("duration", "input", "data", Some("core/duration@1"), None),
            ],
        ),
        "core/break" | "core/continue" => {
            (NodeKind::Action, &[("in", "input", "exec", None, None)])
        }
        _ => return None,
    };
    let mut ports: Vec<PortDescriptor> = specs
        .iter()
        .map(|(name, direction, port_kind, ty, source)| {
            let mut value =
                serde_json::json!({"port_id":name,"direction":direction,"kind":port_kind});
            if let Some(ty) = ty {
                value["type"] = serde_json::json!(ty);
            }
            if let Some(source) = source {
                value["source"] = serde_json::json!(source);
            }
            serde_json::from_value(value).expect("intrinsic descriptor is valid")
        })
        .collect();
    if matches!(id, "core/sequence" | "core/fork") {
        let name = if id == "core/sequence" {
            "then"
        } else {
            "branch"
        };
        let mut value = serde_json::json!({
            "port_id":name,
            "direction":"output",
            "kind":"exec",
            "variadic":{"min": if id == "core/sequence" { 2 } else { 1 }, "max": null}
        });
        let descriptor: PortDescriptor =
            serde_json::from_value(value.take()).expect("intrinsic descriptor is valid");
        ports.push(descriptor);
    }
    let suspends = match id {
        "core/yield" | "core/wait" => Some(Suspends::Always),
        "core/count" | "core/while" => Some(Suspends::Maybe),
        _ => None,
    };
    Some(SignatureSnapshot {
        kind,
        suspends,
        ports,
        extensions: Default::default(),
    })
}

pub(crate) fn implicit_port(snapshot: &SignatureSnapshot, id: &PortId) -> Option<PortDescriptor> {
    if !snapshot.declares_port(id) {
        return None;
    }
    let name = id.as_str();
    let (direction, kind, ty, source) = match name {
        "in" => (PortDirection::Input, PortKind::Exec, None, None),
        "failed" | "cancelled" => (PortDirection::Output, PortKind::Exec, None, None),
        "error" => (
            PortDirection::Output,
            PortKind::Data,
            Some("core/error@1"),
            Some("execution"),
        ),
        "then" | "body" | "completed" => (PortDirection::Output, PortKind::Exec, None, None),
        _ => return None,
    };
    Some(PortDescriptor {
        port_id: id.clone(),
        direction,
        kind,
        ty: ty.map(|value| {
            value
                .parse::<TypeExpr>()
                .expect("intrinsic type expression is valid")
        }),
        source: source.map(|value| {
            serde_json::from_value(serde_json::json!(value)).expect("source is valid")
        }),
        available_on: (name == "error").then(|| vec!["failed".parse().expect("port id is valid")]),
        default: None,
        sensitivity: None,
        extensions: Default::default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn document(
        definition: &str,
        signature: Option<&str>,
        variadic: serde_json::Value,
    ) -> GraphDocument {
        document_with_versions(definition, signature, variadic, 1, 1)
    }

    fn document_with_versions(
        definition: &str,
        signature: Option<&str>,
        variadic: serde_json::Value,
        definition_version: u32,
        config_version: u32,
    ) -> GraphDocument {
        let mut value = json!({
            "format_version": 1,
            "graph_id": "10000000-0000-4000-8000-000000000001",
            "revision": 0,
            "nodes": [{
                "id": "10000000-0000-4000-8000-000000000002",
                "definition_id": definition,
                "definition_version": definition_version,
                "config_version": config_version,
                "config": {},
                "variadic": variadic
            }],
            "edges": [],
            "signatures": {}
        });
        if let Some(signature) = signature {
            value["nodes"][0]["signature"] = json!(signature);
            value["signatures"][signature] = json!({"kind":"action","ports":[
                {"port_id":"value","direction":"output","kind":"event","type":"core/i64@1","source":"execution","available_on":["failed"]},
            {"port_id":"args","direction":"input","kind":"data","type":"core/i64@1","variadic":{"min":0,"max":null}}
            ]});
        }
        serde_json::from_value(value).unwrap()
    }

    fn reference(key: &str) -> PortRef {
        serde_json::from_value(json!({"node":"10000000-0000-4000-8000-000000000002","port":key}))
            .unwrap()
    }

    #[test]
    fn distinguishes_missing_metadata_from_missing_node_and_unknown_port() {
        let doc = document("vendor/task", None, json!({}));
        assert_eq!(
            lookup_port(&doc, &reference("in")),
            Err(PortLookupError::MissingMetadata)
        );
        let absent: PortRef = serde_json::from_value(
            json!({"node":"10000000-0000-4000-8000-000000000099","port":"in"}),
        )
        .unwrap();
        assert_eq!(
            lookup_port(&doc, &absent),
            Err(PortLookupError::MissingNode)
        );
        let doc = document("core/branch", None, json!({}));
        assert_eq!(
            lookup_port(&doc, &reference("anything")),
            Err(PortLookupError::UnknownPort)
        );
        assert_eq!(
            lookup_port(&doc, &reference("condition"))
                .unwrap()
                .ty
                .as_ref()
                .unwrap()
                .as_str(),
            "core/bool@1"
        );
        assert_eq!(
            lookup_port(
                &document("core/not-known", None, json!({})),
                &reference("in")
            ),
            Err(PortLookupError::MissingMetadata)
        );
    }

    #[test]
    fn intrinsic_identity_and_versions_gate_host_metadata() {
        let stored = document(
            "core/convert",
            Some("sha256:0000000000000000000000000000000000000000000000000000000000000000"),
            json!({}),
        );
        assert!(lookup_port(&stored, &reference("value")).is_ok());
        assert_eq!(
            lookup_port(&document("core/foo", None, json!({})), &reference("in")),
            Err(PortLookupError::MissingMetadata)
        );
        assert_eq!(
            lookup_port(
                &document_with_versions("core/branch", None, json!({}), 2, 1),
                &reference("in")
            ),
            Err(PortLookupError::MissingMetadata)
        );
        assert_eq!(
            lookup_port(
                &document_with_versions("core/branch", None, json!({}), 1, 2),
                &reference("in")
            ),
            Err(PortLookupError::MissingMetadata)
        );
        for definition in ["core/break", "core/continue"] {
            let doc = document(definition, None, json!({}));
            assert_eq!(
                lookup_port(&doc, &reference("in")).unwrap().kind,
                PortKind::Exec
            );
            assert_eq!(
                lookup_port(&doc, &reference("error"))
                    .unwrap()
                    .ty
                    .as_ref()
                    .unwrap()
                    .as_str(),
                "core/error@1"
            );
            assert_eq!(
                lookup_port(&doc, &reference("then")),
                Err(PortLookupError::UnknownPort)
            );
        }
    }

    #[test]
    fn snapshots_include_implicit_semantics_and_preserve_event_descriptors() {
        let doc = document(
            "vendor/task",
            Some("sha256:0000000000000000000000000000000000000000000000000000000000000000"),
            json!({"args":{"order":[2,5],"next":6}}),
        );
        assert_eq!(
            lookup_port(&doc, &reference("in")).unwrap().kind,
            PortKind::Exec
        );
        let event = lookup_port(&doc, &reference("value")).unwrap();
        assert_eq!(event.kind, PortKind::Event);
        assert_eq!(event.available_on.as_ref().unwrap()[0].as_str(), "failed");
        assert_eq!(
            lookup_port(&doc, &reference("error"))
                .unwrap()
                .ty
                .as_ref()
                .unwrap()
                .as_str(),
            "core/error@1"
        );
        assert_eq!(
            lookup_port(&doc, &reference("args#5"))
                .unwrap()
                .port_id
                .as_str(),
            "args"
        );
        assert_eq!(
            lookup_port(&doc, &reference("args#3")),
            Err(PortLookupError::UnknownPort)
        );
        assert_eq!(
            lookup_port(&doc, &reference("args")),
            Err(PortLookupError::UnknownPort)
        );
        assert_eq!(
            lookup_port(&doc, &reference("value#2")),
            Err(PortLookupError::UnknownPort)
        );
    }

    #[test]
    fn supported_intrinsic_variadics_use_stable_instance_order() {
        let doc = document(
            "core/sequence",
            None,
            json!({"then":{"order":[0,2],"next":3}}),
        );
        assert_eq!(
            lookup_port(&doc, &reference("then#2")).unwrap().kind,
            PortKind::Exec
        );
        assert_eq!(
            lookup_port(&doc, &reference("then#1")),
            Err(PortLookupError::UnknownPort)
        );
        assert_eq!(
            lookup_port(&doc, &reference("then")),
            Err(PortLookupError::UnknownPort)
        );
    }
}
