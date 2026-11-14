//! A document as a canvas draws it: each node with its name, ports and place, and the
//! wires between them. A front end needs nothing else of the model to show a graph.
use super::*;
use orange_model::document::{NodeKind, PortDirection, PortKind};
use serde_json::{Value as Json, json};

/// One port of a node.
#[derive(Debug, Clone, PartialEq)]
pub struct PortView {
    pub id: String,
    pub input: bool,
    /// An execution port carries the order of steps; any other carries a value.
    pub exec: bool,
    /// The type of a value port, such as `core/string@1`.
    pub ty: Option<String>,
    /// The value written on an input, as a person reads it.
    pub value: Option<String>,
    /// Whether a wire ends or starts here.
    pub connected: bool,
}

/// One node of a document's top level.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeView {
    pub id: NodeId,
    /// What the node is called: its `name`, or what its kind is called.
    pub name: String,
    /// What its kind is called, such as "Write text file".
    pub kind: String,
    /// The definition, such as `org.orange.files/write-text`.
    pub definition: String,
    /// Whether it starts work: a trigger.
    pub entry: bool,
    /// Inputs and outputs in declared order. Empty when this host does not have the
    /// node's definition.
    pub ports: Vec<PortView>,
    /// Where the document's layout puts it, if it says.
    pub position: Option<[f32; 2]>,
}

/// One wire from an output to an input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireView {
    pub from: (NodeId, String),
    pub to: (NodeId, String),
    pub exec: bool,
}

/// The top level of a document, for drawing.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GraphView {
    pub nodes: Vec<NodeView>,
    pub wires: Vec<WireView>,
}

impl Host {
    /// What a canvas shows of a document: its top-level nodes with their ports and
    /// places, and its wires. The document does not have to be valid or bound; a node
    /// whose definition this host lacks is shown without ports.
    pub fn view(&self, json: &str) -> Result<GraphView, HostError> {
        let document = self.linked(json)?;
        let registry = self.store.current();
        let names = Names::of(self, json);
        let labels: BTreeMap<String, String> = self
            .nodes()
            .map(|info| {
                (
                    info.definition.definition.id.as_str().to_owned(),
                    info.label().into_owned(),
                )
            })
            .collect();
        let places = document.editor().get("nodes");
        let body = document.root();
        let wired = |node: NodeId, port: &str, input: bool| {
            body.edges.values().any(|edge| {
                let end = if input { &edge.to } else { &edge.from };
                end.node == node && end.port.to_string() == port
            })
        };
        let mut nodes = Vec::with_capacity(body.nodes.len());
        for node in body.nodes.values() {
            let contract = registry.contract(&document, node).ok().flatten();
            let declared: Vec<PortView> = contract
                .iter()
                .flat_map(|contract| contract.ports.iter())
                .map(|port| {
                    let id = port.port_id.to_string();
                    let input = port.direction == PortDirection::Input;
                    let value = node
                        .inputs
                        .iter()
                        .find(|(key, _)| key.to_string() == id)
                        .and_then(|(_, literal)| self.value_of(literal).ok())
                        .map(|value| plain(&value));
                    PortView {
                        connected: wired(node.id, &id, input),
                        input,
                        exec: port.kind != PortKind::Data,
                        ty: port.ty.as_ref().map(ToString::to_string),
                        value: value.filter(|_| input),
                        id,
                    }
                })
                .collect();
            // The ports a kind has without declaring them (11 §4.1): the way in and
            // the way on come first; the routes for failure and cancelling are shown
            // only once something is wired to them, so a card stays short.
            let mut ports = Vec::with_capacity(declared.len() + 2);
            let mut after = Vec::new();
            // A node that is not set up yet (a variable node that names no variable)
            // has no contract; its kind still says which of these it has.
            let kind = contract.as_ref().map(|contract| contract.kind).or_else(|| {
                registry
                    .definition(&node.definition)
                    .map(|found| found.kind)
            });
            if let Some(kind) = kind {
                let outcomes = declared.iter().any(|port| port.exec && !port.input);
                for name in orange_model::catalog::implicit_port_names(
                    node.definition.id.as_str(),
                    kind,
                    outcomes,
                ) {
                    let input = name == "in";
                    let port = PortView {
                        id: name.to_owned(),
                        input,
                        exec: name != "error",
                        ty: (name == "error").then(|| "core/error@1".to_owned()),
                        value: None,
                        connected: wired(node.id, name, input),
                    };
                    match name {
                        "in" | "then" => ports.push(port),
                        "body" | "completed" => after.push(port),
                        _ if port.connected => after.push(port),
                        _ => {}
                    }
                }
            }
            ports.extend(declared);
            ports.extend(after);
            let definition = node.definition.id.as_str().to_owned();
            let position = places
                .and_then(|places| places.get(node.id.to_string()))
                .and_then(|place| Some([place["x"].as_f64()?, place["y"].as_f64()?]))
                .map(|[x, y]| {
                    #[expect(clippy::cast_possible_truncation, reason = "screen coordinates")]
                    [x as f32, y as f32]
                });
            nodes.push(NodeView {
                id: node.id,
                name: names.node(&node.id),
                kind: labels
                    .get(&definition)
                    .cloned()
                    .unwrap_or_else(|| definition.clone()),
                entry: kind == Some(NodeKind::Entry),
                definition,
                ports,
                position,
            });
        }
        let exec = |from: &(NodeId, String)| {
            nodes
                .iter()
                .find(|node| node.id == from.0)
                .and_then(|node| {
                    node.ports
                        .iter()
                        .find(|port| !port.input && port.id == from.1)
                })
                .is_some_and(|port| port.exec)
        };
        let wires = body
            .edges
            .values()
            .map(|edge| {
                let from = (edge.from.node, edge.from.port.to_string());
                WireView {
                    exec: exec(&from),
                    from,
                    to: (edge.to.node, edge.to.port.to_string()),
                }
            })
            .collect();
        Ok(GraphView { nodes, wires })
    }
}

/// Writes where nodes are placed into a document's layout (`editor.nodes`), which the
/// compiler never reads. Places of other nodes are kept. Returns the document text.
pub fn place(json: &str, places: &[(NodeId, [f32; 2])]) -> Result<String, HostError> {
    let refused = |error: &dyn std::fmt::Display| HostError::Document(error.to_string());
    let mut saved: Json = serde_json::from_str(json).map_err(|error| refused(&error))?;
    if !saved["editor"].is_object() {
        saved["editor"] = json!({});
    }
    if !saved["editor"]["nodes"].is_object() {
        saved["editor"]["nodes"] = json!({});
    }
    for (node, [x, y]) in places {
        // Whole numbers keep the saved file stable and readable.
        saved["editor"]["nodes"][node.to_string()] =
            json!({"x": f64::from(x.round()), "y": f64::from(y.round())});
    }
    let document = from_json(&saved.to_string()).map_err(|error| refused(&error))?;
    to_json(&document).map_err(|error| refused(&error))
}
