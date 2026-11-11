//! Package nodes implemented as graphs.
//!
//! A node made in the editor is published as a function: an interface and a body of
//! other nodes. A document uses it like any package node, locked to its exact provider.
//! When the host opens the document it *links* each such node: the node becomes a call
//! to the package's function, which is added to the in-memory copy of the document. The
//! saved document is never changed, and from there on validation, compilation and
//! execution are those of ordinary graph functions, with no separate interpreter and no
//! sandbox to cross.
use serde_json::{Map, Value as Json, json};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// Provider ID a manifest gives a node it implements as a graph.
pub const GRAPH_PROVIDER: &str = "graph";
/// `format` of an archive's `graphs/<name>.json`.
pub const GRAPH_NODE_FORMAT: &str = "orange.graph-node";

/// Exact identity of one graph-implemented node: package, package version, definition
/// and definition version.
pub(crate) type GraphKey = (String, String, String, u64);
pub(crate) type Graphs = BTreeMap<GraphKey, Arc<GraphNode>>;

/// One decoded `graphs/<name>.json`.
#[derive(Debug)]
pub(crate) struct GraphNode {
    function: Json,
    function_id: String,
    /// Local variables the body uses. Each call gets its own; nothing else is shared.
    variables: Vec<Json>,
}

fn data_ports<'a>(ports: impl Iterator<Item = &'a Json>) -> BTreeSet<(String, String, String)> {
    let text = |port: &Json, field: &str| port[field].as_str().unwrap_or_default().to_owned();
    ports
        .filter(|port| port["kind"] == "data")
        .map(|port| {
            (
                text(port, "port_id"),
                text(port, "direction"),
                text(port, "type"),
            )
        })
        .collect()
}

impl GraphNode {
    /// Decodes a graph node file and checks it against the node's definition file: same
    /// kind and the same data ports. Whether the body is a valid function is decided by
    /// validating it, which the host does before publishing the package.
    pub fn decode(text: &str, definition: &str) -> Result<Self, String> {
        let file: Json = serde_json::from_str(text).map_err(|error| error.to_string())?;
        let definition: Json =
            serde_json::from_str(definition).map_err(|error| error.to_string())?;
        if file["format"] != GRAPH_NODE_FORMAT || file["format_version"] != 1 {
            return Err(format!("expected format `{GRAPH_NODE_FORMAT}` version 1"));
        }
        let function = file
            .get("function")
            .filter(|function| function.is_object())
            .ok_or("missing `function`")?
            .clone();
        let function_id = function["id"]
            .as_str()
            .ok_or("the function has no `id`")?
            .to_owned();
        if function["kind"] != definition["kind"] {
            return Err("the function's kind differs from the node definition".into());
        }
        let list = |value: &Json| value.as_array().cloned().unwrap_or_default();
        let (inputs, outputs) = (list(&function["inputs"]), list(&function["outputs"]));
        let declared = list(&definition["ports"]);
        if data_ports(inputs.iter().chain(&outputs)) != data_ports(declared.iter()) {
            return Err("the function's data ports differ from the node definition".into());
        }
        let variables = list(&file["variables"]);
        if variables
            .iter()
            .any(|variable| variable["scope"] != "local")
        {
            return Err("a graph node may declare local variables only".into());
        }
        Ok(Self {
            function,
            function_id,
            variables,
        })
    }
}

fn graph_of(node: &Json, graphs: &Graphs) -> Option<Arc<GraphNode>> {
    let provider = node.get("provider")?;
    if provider["id"] != GRAPH_PROVIDER {
        return None;
    }
    let key = (
        provider["package"].as_str()?.to_owned(),
        provider["version"].as_str()?.to_owned(),
        node["definition_id"].as_str()?.to_owned(),
        node["definition_version"].as_u64()?,
    );
    graphs.get(&key).cloned()
}

/// Turns the node into a call of `graph`'s function. Its ID, literals, type arguments
/// and therefore every edge attached to it stay as they are.
fn retarget(node: &mut Map<String, Json>, graph: &GraphNode) {
    let _ = node.insert("definition_id".into(), json!("core/call"));
    let _ = node.insert("definition_version".into(), json!(1));
    let _ = node.insert("config_version".into(), json!(1));
    let _ = node.insert("config".into(), json!({"function": graph.function_id}));
    let _ = node.remove("provider");
    let _ = node.remove("signature");
}

fn retarget_all(nodes: Option<&mut Json>, graphs: &Graphs, used: &mut Vec<Arc<GraphNode>>) {
    for node in nodes.and_then(Json::as_array_mut).into_iter().flatten() {
        if let Some(graph) = graph_of(node, graphs)
            && let Some(fields) = node.as_object_mut()
        {
            retarget(fields, &graph);
            used.push(graph);
        }
    }
}

fn add_missing(document: &mut Map<String, Json>, list: &str, items: &[Json]) {
    let entry = document.entry(list).or_insert_with(|| json!([]));
    let Some(existing) = entry.as_array_mut() else {
        return;
    };
    for item in items {
        if existing.iter().all(|present| present["id"] != item["id"]) {
            existing.push(item.clone());
        }
    }
}

/// Links every graph-implemented node of a saved document, including those inside the
/// functions it pulls in. A node whose graph is not installed is left as it is, and
/// validation then reports its provider as unavailable.
pub(crate) fn link(document: &mut Json, graphs: &Graphs) {
    let Some(root) = document.as_object_mut() else {
        return;
    };
    // Each pass rewrites at least one node and a function is added once, so this ends.
    loop {
        let mut used = Vec::new();
        retarget_all(root.get_mut("nodes"), graphs, &mut used);
        let functions = root.get_mut("functions").and_then(Json::as_array_mut);
        for function in functions.into_iter().flatten() {
            retarget_all(function.get_mut("nodes"), graphs, &mut used);
        }
        if used.is_empty() {
            return;
        }
        for graph in used {
            add_missing(root, "functions", std::slice::from_ref(&graph.function));
            add_missing(root, "variables", &graph.variables);
        }
    }
}

/// A document holding only the given graphs, for validating them before publication.
pub(crate) fn scratch(empty_document: &str, graphs: &[Arc<GraphNode>]) -> Option<Json> {
    let mut document: Json = serde_json::from_str(empty_document).ok()?;
    let root = document.as_object_mut()?;
    for graph in graphs {
        add_missing(root, "functions", std::slice::from_ref(&graph.function));
        add_missing(root, "variables", &graph.variables);
    }
    Some(document)
}
