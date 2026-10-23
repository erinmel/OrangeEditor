//! Shared, read-only resolution of a graph port reference.

use std::collections::BTreeMap;

use thiserror::Error;

use crate::document::{GraphDocument, OutputSource, PortDescriptor, PortRef, SignatureSnapshot};
use crate::id::{NodeId, PortId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(crate) enum PortLookupError {
    #[error("port refers to a missing node")]
    MissingNode,
    #[error("node metadata is unavailable")]
    MissingMetadata,
    #[error("port is not declared by the node")]
    UnknownPort,
}

/// Look up a concrete port in the node's fresh resolution (`resolved`, from
/// [`validate_nodes`](super::validate_nodes)). A package node that did not resolve falls
/// back to its stored snapshot, so a missing package still shows its ports (11 §4.5); an
/// unresolved intrinsic has no stored snapshot and stays unavailable.
pub(crate) fn lookup_port(
    document: &GraphDocument,
    resolved: &BTreeMap<NodeId, SignatureSnapshot>,
    port: &PortRef,
) -> Result<PortDescriptor, PortLookupError> {
    let node = document
        .node(port.node)
        .ok_or(PortLookupError::MissingNode)?;
    let definition = node.definition.id.as_str();
    let snapshot = match resolved.get(&port.node) {
        Some(snapshot) => snapshot,
        None if is_intrinsic_definition(definition) => {
            return Err(PortLookupError::MissingMetadata);
        }
        None => node
            .signature
            .as_ref()
            .and_then(|key| document.signatures().get(key))
            .ok_or(PortLookupError::MissingMetadata)?,
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
        if declared.variadic.is_none() {
            return Err(PortLookupError::UnknownPort);
        }
        return Ok(declared.clone());
    }

    if node.variadic.contains_key(port.port.port())
        || snapshot
            .ports
            .iter()
            .any(|p| p.port_id == *port.port.port() && p.variadic.is_some())
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
    implicit_port(snapshot, port.port.port()).ok_or(PortLookupError::UnknownPort)
}

/// Whether an ID is a language intrinsic, independent of whether this slice has a
/// complete descriptor for it yet. Later validator passes use this to avoid treating
/// arbitrary `core/` IDs as trusted intrinsics.
pub(crate) use crate::document::is_intrinsic_definition;

pub(crate) fn implicit_port(snapshot: &SignatureSnapshot, id: &PortId) -> Option<PortDescriptor> {
    if !snapshot.declares_port(id) {
        return None;
    }
    let name = id.as_str();
    let port = match name {
        "in" => PortDescriptor::exec_in(name),
        "failed" | "cancelled" | "then" | "body" | "completed" => PortDescriptor::exec_out(name),
        "error" => PortDescriptor {
            available_on: Some(vec!["failed".parse().expect("port id is valid")]),
            ..PortDescriptor::data_out(
                name,
                "core/error@1"
                    .parse()
                    .expect("intrinsic type expression is valid"),
                OutputSource::Execution,
            )
        },
        _ => return None,
    };
    Some(port)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::RegistrySnapshot;
    use crate::document::PortKind;
    use crate::validation::{HostOrigin, validate_nodes};
    use serde_json::json;

    /// Looks a port up against the core registry's fresh resolution of `doc`.
    fn lookup(doc: &GraphDocument, port: &PortRef) -> Result<PortDescriptor, PortLookupError> {
        let origin = HostOrigin::new("0.1.0", format!("sha256:{}", "c".repeat(64))).unwrap();
        let resolved = validate_nodes(doc, &RegistrySnapshot::core(origin)).resolved;
        lookup_port(doc, &resolved, port)
    }

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
            lookup(&doc, &reference("in")),
            Err(PortLookupError::MissingMetadata)
        );
        let absent: PortRef = serde_json::from_value(
            json!({"node":"10000000-0000-4000-8000-000000000099","port":"in"}),
        )
        .unwrap();
        assert_eq!(lookup(&doc, &absent), Err(PortLookupError::MissingNode));
        let doc = document("core/branch", None, json!({}));
        assert_eq!(
            lookup(&doc, &reference("anything")),
            Err(PortLookupError::UnknownPort)
        );
        assert_eq!(
            lookup(&doc, &reference("condition"))
                .unwrap()
                .ty
                .as_ref()
                .unwrap()
                .as_str(),
            "core/bool@1"
        );
        assert_eq!(
            lookup(
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
        assert!(lookup(&stored, &reference("value")).is_ok());
        assert_eq!(
            lookup(&document("core/foo", None, json!({})), &reference("in")),
            Err(PortLookupError::MissingMetadata)
        );
        assert_eq!(
            lookup(
                &document_with_versions("core/branch", None, json!({}), 2, 1),
                &reference("in")
            ),
            Err(PortLookupError::MissingMetadata)
        );
        assert_eq!(
            lookup(
                &document_with_versions("core/branch", None, json!({}), 1, 2),
                &reference("in")
            ),
            Err(PortLookupError::MissingMetadata)
        );
        for definition in ["core/break", "core/continue"] {
            let doc = document(definition, None, json!({}));
            assert_eq!(lookup(&doc, &reference("in")).unwrap().kind, PortKind::Exec);
            assert_eq!(
                lookup(&doc, &reference("error"))
                    .unwrap()
                    .ty
                    .as_ref()
                    .unwrap()
                    .as_str(),
                "core/error@1"
            );
            assert_eq!(
                lookup(&doc, &reference("then")),
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
        assert_eq!(lookup(&doc, &reference("in")).unwrap().kind, PortKind::Exec);
        let event = lookup(&doc, &reference("value")).unwrap();
        assert_eq!(event.kind, PortKind::Event);
        assert_eq!(event.available_on.as_ref().unwrap()[0].as_str(), "failed");
        assert_eq!(
            lookup(&doc, &reference("error"))
                .unwrap()
                .ty
                .as_ref()
                .unwrap()
                .as_str(),
            "core/error@1"
        );
        assert_eq!(
            lookup(&doc, &reference("args#5")).unwrap().port_id.as_str(),
            "args"
        );
        assert_eq!(
            lookup(&doc, &reference("args#3")),
            Err(PortLookupError::UnknownPort)
        );
        assert_eq!(
            lookup(&doc, &reference("args")),
            Err(PortLookupError::UnknownPort)
        );
        assert_eq!(
            lookup(&doc, &reference("value#2")),
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
            lookup(&doc, &reference("then#2")).unwrap().kind,
            PortKind::Exec
        );
        assert_eq!(
            lookup(&doc, &reference("then#1")),
            Err(PortLookupError::UnknownPort)
        );
        assert_eq!(
            lookup(&doc, &reference("then")),
            Err(PortLookupError::UnknownPort)
        );
    }
}
