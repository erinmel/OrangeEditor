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
use sha2::{Digest, Sha256};
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
    /// Local variables the body uses. Each call gets its own; nothing else is shared.
    variables: Vec<Json>,
    /// Signature snapshots and package locks of the package nodes in the body, which a
    /// document that links the body must hold too.
    signatures: Map<String, Json>,
    package_lock: Vec<Json>,
    /// The stand-in connection ID the body's nodes name, when the node uses a
    /// connection. Each use of the node replaces it with the connection it names.
    connection: Option<String>,
    /// Set for an entry node, which is a fragment of root nodes and has no function.
    trigger: Option<Trigger>,
}

/// The body of an entry node made as a graph: root nodes that start at some other
/// entry and end by emitting `event`, whose payload is the node's `payload` output.
#[derive(Debug)]
struct Trigger {
    nodes: Vec<Json>,
    edges: Vec<Json>,
    /// Stand-in ID of the event the fragment emits. Each use gets its own event.
    event: String,
    /// Type of the node's `payload` output.
    payload: Json,
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
    /// kind, the same data ports and the same use of a connection. Whether the body is a
    /// valid function is decided by validating it, which the host does before publishing
    /// the package.
    pub fn decode(text: &str, definition: &str) -> Result<Self, String> {
        let file: Json = serde_json::from_str(text).map_err(|error| error.to_string())?;
        let definition: Json =
            serde_json::from_str(definition).map_err(|error| error.to_string())?;
        if file["format"] != GRAPH_NODE_FORMAT || file["format_version"] != 1 {
            return Err(format!("expected format `{GRAPH_NODE_FORMAT}` version 1"));
        }
        let list = |value: &Json| value.as_array().cloned().unwrap_or_default();
        let connection = match file.get("connection") {
            None => None,
            Some(id) => Some(id.as_str().ok_or("`connection` must be an ID")?.to_owned()),
        };
        if connection.is_some() != (definition["connection"] == true) {
            return Err("the graph and the node definition disagree on using a connection".into());
        }
        if definition["kind"] == "entry" {
            let trigger = file
                .get("trigger")
                .filter(|trigger| trigger.is_object())
                .ok_or("an entry node needs a `trigger`")?;
            let payload = list(&definition["ports"])
                .into_iter()
                .find(|port| port["port_id"] == "payload" && port["direction"] == "output")
                .map(|port| port["type"].clone())
                .filter(Json::is_string)
                .ok_or("an entry node made as a graph needs a `payload` output")?;
            return Ok(Self {
                function: Json::Null,
                variables: Vec::new(),
                signatures: file["signatures"].as_object().cloned().unwrap_or_default(),
                package_lock: list(&file["package_lock"]),
                connection,
                trigger: Some(Trigger {
                    nodes: list(&trigger["nodes"]),
                    edges: list(&trigger["edges"]),
                    event: trigger["event"]
                        .as_str()
                        .ok_or("the trigger names no `event`")?
                        .to_owned(),
                    payload,
                }),
            });
        }
        let function = file
            .get("function")
            .filter(|function| function.is_object())
            .ok_or("missing `function`")?
            .clone();
        if !function["id"].is_string() {
            return Err("the function has no `id`".into());
        }
        if function["kind"] != definition["kind"] {
            return Err("the function's kind differs from the node definition".into());
        }
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
            variables,
            signatures: file["signatures"].as_object().cloned().unwrap_or_default(),
            package_lock: list(&file["package_lock"]),
            connection,
            trigger: None,
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

/// A stable ID for `id` as used with `connection`: the same pair always gives the same
/// ID, and different connections never share one.
fn derived(id: &str, connection: &str) -> String {
    let hash = Sha256::digest(format!("orange.graph-node\n{id}\n{connection}").as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&hash[..16]);
    // A random-form (version 4) UUID, which is the only form document IDs take.
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

/// The function a use of `graph` calls. A graph that uses no connection has one
/// function for every use. One that does gets a copy per connection, with its own IDs
/// and with the stand-in connection replaced, because a function cannot take a
/// connection as an argument.
fn function_for(graph: &GraphNode, connection: Option<&str>) -> Json {
    let (Some(stand_in), Some(actual)) = (graph.connection.as_deref(), connection) else {
        return graph.function.clone();
    };
    let mut function = graph.function.clone();
    let rename = |value: &mut Json| {
        if let Some(id) = value.as_str() {
            *value = json!(derived(id, actual));
        }
    };
    rename(&mut function["id"]);
    for node in function["nodes"].as_array_mut().into_iter().flatten() {
        rename(&mut node["id"]);
        if node["config"]["connection"] == stand_in {
            node["config"]["connection"] = json!(actual);
        }
    }
    for edge in function["edges"].as_array_mut().into_iter().flatten() {
        rename(&mut edge["id"]);
        rename(&mut edge["from"]["node"]);
        rename(&mut edge["to"]["node"]);
    }
    function
}

/// Turns the node into a call of `function`. Its ID, literals, type arguments and
/// therefore every edge attached to it stay as they are.
fn retarget(node: &mut Map<String, Json>, function: &Json) {
    let _ = node.insert("definition_id".into(), json!("core/call"));
    let _ = node.insert("definition_version".into(), json!(1));
    let _ = node.insert("config_version".into(), json!(1));
    let _ = node.insert("config".into(), json!({"function": function["id"]}));
    let _ = node.remove("provider");
    let _ = node.remove("signature");
}

/// What linking one node adds to the document.
enum Linked {
    Function(Json),
    /// Root nodes and edges, and the event they emit.
    Fragment(Vec<Json>, Vec<Json>, Json),
}

/// The fragment of one use of an entry node: its own node, edge and event IDs, derived
/// from the using node, and the connection that node names.
fn fragment_for(
    graph: &GraphNode,
    trigger: &Trigger,
    user: &str,
    connection: Option<&str>,
) -> Linked {
    let rename = |value: &mut Json| {
        if let Some(id) = value.as_str() {
            *value = json!(derived(id, user));
        }
    };
    let event = derived(&trigger.event, user);
    let mut nodes = trigger.nodes.clone();
    for node in &mut nodes {
        rename(&mut node["id"]);
        if node["config"]["event"] == trigger.event.as_str() {
            node["config"]["event"] = json!(event);
        }
        if let (Some(stand_in), Some(actual)) = (graph.connection.as_deref(), connection)
            && node["config"]["connection"] == stand_in
        {
            node["config"]["connection"] = json!(actual);
        }
    }
    let mut edges = trigger.edges.clone();
    for edge in &mut edges {
        rename(&mut edge["id"]);
        rename(&mut edge["from"]["node"]);
        rename(&mut edge["to"]["node"]);
    }
    let declaration = json!({"id": event, "name": format!("trigger-{event}"),
        "payload": trigger.payload, "scope": "run"});
    Linked::Fragment(nodes, edges, declaration)
}

fn retarget_all(
    nodes: Option<&mut Json>,
    graphs: &Graphs,
    root: bool,
    used: &mut Vec<(Arc<GraphNode>, Linked)>,
) {
    for node in nodes.and_then(Json::as_array_mut).into_iter().flatten() {
        if let Some(graph) = graph_of(node, graphs) {
            let connection = node["config"]["connection"].as_str().map(str::to_owned);
            if let Some(trigger) = &graph.trigger {
                // An entry exists at the root only; elsewhere the node stays as it is
                // and validation refuses it.
                let user = node["id"].as_str().map(str::to_owned);
                let (true, Some(user), Some(fields)) = (root, user, node.as_object_mut()) else {
                    continue;
                };
                let linked = fragment_for(&graph, trigger, &user, connection.as_deref());
                if let Linked::Fragment(_, _, event) = &linked {
                    // The node becomes the handler of the fragment's event: the same
                    // `then` outcome and `payload` output, so its edges stay.
                    let _ = fields.insert("definition_id".into(), json!("core/on-event"));
                    let _ = fields.insert("definition_version".into(), json!(1));
                    let _ = fields.insert("config_version".into(), json!(1));
                    let _ = fields.insert("config".into(), json!({"event": event["id"]}));
                    let _ = fields.remove("provider");
                    let _ = fields.remove("signature");
                }
                used.push((graph, linked));
                continue;
            }
            let function = function_for(&graph, connection.as_deref());
            if let Some(fields) = node.as_object_mut() {
                retarget(fields, &function);
                used.push((graph, Linked::Function(function)));
            }
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

/// Adds what a linked body needs from its package beside the function itself.
fn add_support(document: &mut Map<String, Json>, graph: &GraphNode) {
    add_missing(document, "variables", &graph.variables);
    let entry = document.entry("signatures").or_insert_with(|| json!({}));
    if let Some(signatures) = entry.as_object_mut() {
        for (key, snapshot) in &graph.signatures {
            let _ = signatures
                .entry(key.clone())
                .or_insert_with(|| snapshot.clone());
        }
    }
    let entry = document.entry("package_lock").or_insert_with(|| json!([]));
    if let Some(locks) = entry.as_array_mut() {
        for lock in &graph.package_lock {
            let held = locks
                .iter()
                .any(|held| held["id"] == lock["id"] && held["version"] == lock["version"]);
            if !held {
                locks.push(lock.clone());
            }
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
        retarget_all(root.get_mut("nodes"), graphs, true, &mut used);
        let functions = root.get_mut("functions").and_then(Json::as_array_mut);
        for function in functions.into_iter().flatten() {
            retarget_all(function.get_mut("nodes"), graphs, false, &mut used);
        }
        if used.is_empty() {
            return;
        }
        for (graph, linked) in used {
            match linked {
                Linked::Function(function) => {
                    add_missing(root, "functions", std::slice::from_ref(&function));
                }
                Linked::Fragment(nodes, edges, event) => {
                    add_missing(root, "nodes", &nodes);
                    add_missing(root, "edges", &edges);
                    add_missing(root, "events", std::slice::from_ref(&event));
                }
            }
            add_support(root, &graph);
        }
    }
}

/// A document holding only the given graphs, for validating them before publication.
/// A graph that uses a connection is checked against a stand-in declaration.
pub(crate) fn scratch(empty_document: &str, graphs: &[Arc<GraphNode>]) -> Option<Json> {
    let mut document: Json = serde_json::from_str(empty_document).ok()?;
    let root = document.as_object_mut()?;
    for graph in graphs {
        if let Some(trigger) = &graph.trigger {
            add_missing(root, "nodes", &trigger.nodes);
            add_missing(root, "edges", &trigger.edges);
            let event = json!({"id": trigger.event, "name": "trigger",
                "payload": trigger.payload, "scope": "run"});
            add_missing(root, "events", std::slice::from_ref(&event));
        } else {
            add_missing(root, "functions", std::slice::from_ref(&graph.function));
        }
        add_support(root, graph);
        if let Some(stand_in) = &graph.connection {
            let declaration = json!({"id": stand_in, "name": "connection",
                "adapter": "host/parameter", "scope": "shared"});
            add_missing(root, "connections", std::slice::from_ref(&declaration));
        }
    }
    Some(document)
}
