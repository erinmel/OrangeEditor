//! A document as a canvas draws it: each node with its name, ports and place, and the
//! wires between them. A front end needs nothing else of the model to show a graph.
use super::*;
use orange_model::catalog::{NominalAccess, Ports};
use orange_model::document::{ConfigValue, NodeInstance, NodeKind, PortDirection, PortKind};
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
    /// What the node's definition calls this port, when it names it: "Queued" for
    /// the `then` of a send that only queues. A front end otherwise names it itself.
    pub label: Option<String>,
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
    /// What the node is set to, in a word, when its settings name something: the
    /// variable a variable node reads or writes, or a record node's type.
    pub detail: Option<String>,
    /// Whether it starts work: a trigger.
    pub entry: bool,
    /// Inputs and outputs in declared order. Empty when this host does not have the
    /// node's definition.
    pub ports: Vec<PortView>,
    /// Where the document's layout puts it, if it says.
    pub position: Option<[f32; 2]>,
    /// Its settings (`config`) as the document holds them.
    pub settings: Json,
    /// What its settings may hold, as the JSON schema its kind declares; `None` when
    /// the kind says nothing about them.
    pub schema: Option<Json>,
    /// Settings fields that name one of the document's declarations: a JSON pointer
    /// into the settings, such as `/variable`, and what it names.
    pub references: Vec<(String, ReferenceKind)>,
    /// The type parameters of a generic node, in declared order.
    pub types: Vec<TypeChoice>,
    /// The ports of which the node has as many as it is given ([`Host::add_port`]).
    pub variadic: Vec<VariadicView>,
    /// Text inputs that hold a path on this machine, from the node's definition file,
    /// so the editor can offer the system's file or folder chooser beside them.
    pub paths: Vec<PathInput>,
}

/// A variadic port of a node, such as the branches of a fork: its instances are
/// in [`NodeView::ports`] as `branch#0`, `branch#1`, ….
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariadicView {
    /// The port, such as `branch`.
    pub port: String,
    pub input: bool,
    /// How many instances the node has now, and how many it may have.
    pub count: usize,
    pub min: usize,
    pub max: Option<usize>,
}

/// One type parameter of a generic node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeChoice {
    /// Its name, such as `T`.
    pub name: String,
    /// The type a person chose for it, if any.
    pub chosen: Option<String>,
    /// The type it has now: the chosen one, or the one found from the wires.
    pub found: Option<String>,
}

/// The settings schema that the language's own checks of a core node follow, for a
/// front end to draw fields from: timers (11 §10.2, a length of time is written in
/// nanoseconds, as digits), the bound of a while loop and the mode of a for-each.
fn core_schema(definition: &str) -> Option<Json> {
    let time = json!({"type": "string", "format": "duration-ns", "pattern": "^(0|[1-9][0-9]*)$"});
    match definition {
        "core/while" => Some(json!({"type": "object", "required": ["max_iterations"],
            "properties": {"max_iterations": {"type": "integer", "minimum": 1}}})),
        "core/for-each" => Some(json!({"type": "object",
            "properties": {"mode": {"type": "string", "enum": ["list", "map"]}}})),
        "core/on-timer" => Some(json!({"type": "object", "required": ["interval"],
            "properties": {"interval": time,
                "first": {"type": "string", "enum": ["after-interval", "immediate"]},
                "missed": {"type": "string", "enum": ["coalesce", "skip"]}},
            "additionalProperties": false})),
        "core/on-timeout" => Some(json!({"type": "object", "required": ["delay"],
            "properties": {"delay": time}, "additionalProperties": false})),
        _ => None,
    }
}

/// The member a record or enum node names besides its type: `field` or `variant`.
pub(crate) fn nominal_member(access: NominalAccess) -> Option<&'static str> {
    match access {
        NominalAccess::MakeRecord => None,
        NominalAccess::GetField | NominalAccess::SetField => Some("field"),
        NominalAccess::MakeEnum | NominalAccess::EnumPayload => Some("variant"),
    }
}

/// The record or enum types a node of `access` can name, with the members it can
/// address in each, from this host's registered types.
pub(crate) fn nominal_choices(
    access: NominalAccess,
    registry: &RegistrySnapshot,
) -> Vec<(String, Vec<String>)> {
    let record = matches!(
        access,
        NominalAccess::MakeRecord | NominalAccess::GetField | NominalAccess::SetField
    );
    registry
        .types()
        .nominal_types()
        .into_iter()
        .filter(|ty| ty.record == record)
        .map(|ty| {
            let members = if access == NominalAccess::EnumPayload {
                ty.with_payload
            } else {
                ty.members
            };
            (ty.name, members)
        })
        // A projection needs a member to address.
        .filter(|(_, members)| nominal_member(access).is_none() || !members.is_empty())
        .collect()
}

/// The settings of a record or enum node: the type it names, chosen among the
/// registered ones, and the member of that type it addresses.
fn nominal_schema(
    access: NominalAccess,
    config: &ConfigValue,
    registry: &RegistrySnapshot,
) -> Json {
    let choices = nominal_choices(access, registry);
    let current = config.get("type").and_then(Json::as_str);
    let mut properties = serde_json::Map::new();
    let _ = properties.insert(
        "type".to_owned(),
        json!({"type": "string", "enum": choices.iter().map(|(name, _)| name).collect::<Vec<_>>()}),
    );
    if let Some(member) = nominal_member(access) {
        let members = choices
            .iter()
            .find(|(name, _)| Some(name.as_str()) == current)
            .map(|(_, members)| members.clone())
            .unwrap_or_default();
        let _ = properties.insert(
            member.to_owned(),
            json!({"type": "string", "enum": members}),
        );
    }
    json!({"type": "object", "required": ["type"], "properties": properties})
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

/// What every node view of one document is drawn with.
struct ViewContext<'a> {
    document: &'a GraphDocument,
    /// The document's registry, with its own types ([`Host::registry_for`]).
    registry: Arc<RegistrySnapshot>,
    names: Names,
    labels: BTreeMap<String, String>,
    places: Option<&'a Json>,
    /// The ports a wire ends or starts at, by node and side.
    connected: std::collections::HashMap<(NodeId, bool), std::collections::HashSet<String>>,
}

impl Host {
    /// What a canvas shows of a document: its top-level nodes with their ports and
    /// places, and its wires. The document does not have to be valid or bound; a node
    /// whose definition this host lacks is shown without ports.
    pub fn view(&self, json: &str) -> Result<GraphView, HostError> {
        Ok(self.view_of(&self.linked(json)?))
    }

    /// [`Host::view`] of a draft.
    pub fn draft_view(&self, draft: &Draft) -> Result<GraphView, HostError> {
        let document = self.linked_document(draft.document())?;
        Ok(self.view_of(&document))
    }

    fn view_context<'a>(&'a self, document: &'a GraphDocument) -> ViewContext<'a> {
        let labels: BTreeMap<String, String> = self
            .nodes()
            .map(|info| {
                (
                    info.definition.definition.id.as_str().to_owned(),
                    info.label().into_owned(),
                )
            })
            .collect();
        // Index endpoints once. A port lookup must not scan every edge, including
        // when the graph is incomplete and exposes only implicit control ports.
        let mut connected: std::collections::HashMap<_, std::collections::HashSet<String>> =
            std::collections::HashMap::new();
        for edge in document.root().edges.values() {
            for (end, input) in [(&edge.from, false), (&edge.to, true)] {
                let _ = connected
                    .entry((end.node, input))
                    .or_default()
                    .insert(end.port.to_string());
            }
        }
        ViewContext {
            document,
            registry: self.registry_for(document),
            names: Names::of_document(self, document),
            labels,
            places: document.editor().get("nodes"),
            connected,
        }
    }

    /// One top-level node as the canvas draws it; `None` when the document has no such node.
    pub(crate) fn node_view(&self, document: &GraphDocument, node: NodeId) -> Option<NodeView> {
        let context = self.view_context(document);
        document
            .root()
            .nodes
            .get(&node)
            .map(|node| self.node_view_in(&context, node))
    }

    fn view_of(&self, document: &GraphDocument) -> GraphView {
        let context = self.view_context(document);
        let body = document.root();
        let nodes: Vec<NodeView> = body
            .nodes
            .values()
            .map(|node| self.node_view_in(&context, node))
            .collect();
        let outputs: std::collections::HashMap<_, _> = nodes
            .iter()
            .flat_map(|node| {
                node.ports
                    .iter()
                    .filter(|port| !port.input)
                    .map(move |port| ((node.id, port.id.as_str()), port.exec))
            })
            .collect();
        let wires = body
            .edges
            .values()
            .map(|edge| {
                let from = (edge.from.node, edge.from.port.to_string());
                WireView {
                    exec: outputs
                        .get(&(from.0, from.1.as_str()))
                        .copied()
                        .unwrap_or(false),
                    from,
                    to: (edge.to.node, edge.to.port.to_string()),
                }
            })
            .collect();
        GraphView { nodes, wires }
    }

    fn node_view_in(&self, context: &ViewContext<'_>, node: &NodeInstance) -> NodeView {
        let ViewContext {
            document,
            registry,
            names,
            labels,
            places,
            connected,
        } = context;
        let wired = |node: NodeId, port: &str, input: bool| {
            connected
                .get(&(node, input))
                .is_some_and(|ports| ports.contains(port))
        };
        let contract = call_contract(document, node, registry)
            .map(Cow::Owned)
            .or_else(|| registry.contract(document, node).ok().flatten());
        // What the node's definition file says about showing it.
        let shown = self.presentations.get(&node.definition);
        // A generic port shows the type its parameter has now: the chosen one, else the
        // one found from the wires, so a value can be typed on it.
        let settled = (!node.type_args.is_empty() || !node.type_pins.is_empty()).then(|| {
            let mut args = node.type_args.clone();
            args.extend(node.type_pins.clone());
            args
        });
        let mut variadic = Vec::new();
        let mut declared: Vec<PortView> = Vec::new();
        for port in contract.iter().flat_map(|contract| contract.ports.iter()) {
            let input = port.direction == PortDirection::Input;
            // A variadic port is shown as its instances, `branch#0`, `branch#1`, … in
            // the node's order (11 §4.5).
            let instances: Vec<Option<u16>> = match port.variadic {
                Some(bounds) => {
                    let order = node
                        .variadic
                        .get(&port.port_id)
                        .map_or(&[][..], |state| state.order.as_slice());
                    variadic.push(VariadicView {
                        port: port.port_id.to_string(),
                        input,
                        count: order.len(),
                        min: usize::from(bounds.min),
                        max: bounds.max.map(usize::from),
                    });
                    order.iter().copied().map(Some).collect()
                }
                None => vec![None],
            };
            for instance in instances {
                let key = PortKey::new(port.port_id.clone(), instance);
                let id = key.to_string();
                let value = node
                    .inputs
                    .get(&key)
                    .and_then(|literal| self.value_in(document, literal).ok())
                    .map(|value| plain(&value));
                declared.push(PortView {
                    connected: wired(node.id, &id, input),
                    input,
                    exec: port.kind != PortKind::Data,
                    ty: port.ty.as_ref().map(|ty| match &settled {
                        Some(args) => {
                            orange_model::signature::substitute_params(ty, args).to_string()
                        }
                        None => ty.to_string(),
                    }),
                    value: value.filter(|_| input),
                    id,
                    label: None,
                });
            }
        }
        // Names the contract declares itself, so an implicit port is not shown twice.
        let declares = |name: &str, input: bool| {
            contract
                .iter()
                .flat_map(|contract| contract.ports.iter())
                .any(|port| {
                    port.port_id.as_str() == name
                        && (port.direction == PortDirection::Input) == input
                })
        };
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
                if declares(name, input) {
                    continue;
                }
                let port = PortView {
                    id: name.to_owned(),
                    input,
                    exec: name != "error",
                    ty: (name == "error").then(|| "core/error@1".to_owned()),
                    value: None,
                    connected: wired(node.id, name, input),
                    label: (name == "then")
                        .then(|| shown?.then_label.as_ref())
                        .flatten()
                        .map(|label| label.fallback.clone()),
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
        let known = registry.definition(&node.definition);
        let types = known
            .iter()
            .flat_map(|found| found.type_params.iter())
            .map(|param| TypeChoice {
                name: param.name.to_string(),
                chosen: node.type_pins.get(&param.name).map(ToString::to_string),
                found: node.type_args.get(&param.name).map(ToString::to_string),
            })
            .collect();
        let position = places
            .and_then(|places| places.get(node.id.to_string()))
            .and_then(|place| Some([place["x"].as_f64()?, place["y"].as_f64()?]))
            .map(|[x, y]| {
                #[expect(clippy::cast_possible_truncation, reason = "screen coordinates")]
                [x as f32, y as f32]
            });
        // What the node is set to, in a word: the variable it names, or the record or
        // enum type it makes or reads.
        let config = |key: &str| node.config.get(key).and_then(Json::as_str);
        let detail = known
            .iter()
            .flat_map(|found| found.references.iter())
            .filter(|field| field.kind == ReferenceKind::Variable)
            .find_map(|field| {
                let id: VariableId = node
                    .config
                    .get(field.pointer.trim_start_matches('/'))?
                    .as_str()?
                    .parse()
                    .ok()?;
                let name = names.variable(&id);
                (name != id.to_string()).then_some(name)
            })
            .or_else(|| {
                let ty = config("type").filter(|_| {
                    known.is_some_and(|found| matches!(found.ports, Ports::Nominal(_)))
                })?;
                let short = ty.rsplit('/').next().unwrap_or(ty);
                let short = short.split('@').next().unwrap_or(short);
                Some(match config("field").or_else(|| config("variant")) {
                    Some(member) => format!("{short}.{member}"),
                    None => short.to_owned(),
                })
            })
            // A trigger shows what fires it.
            .or_else(|| trigger_detail(node.definition.id.as_str(), &node.config));
        NodeView {
            id: node.id,
            detail,
            name: names.called(&node.id),
            kind: labels
                .get(&definition)
                .cloned()
                .or_else(|| (definition == "core/call").then(|| "Function".to_owned()))
                .unwrap_or_else(|| definition.clone()),
            entry: kind == Some(NodeKind::Entry),
            definition,
            ports,
            position,
            settings: serde_json::to_value(&node.config).unwrap_or_default(),
            // A record or enum node's choices come from the registered types, which
            // its definition's own schema cannot list.
            schema: known
                .and_then(|found| match found.ports {
                    Ports::Nominal(access) => Some(nominal_schema(access, &node.config, registry)),
                    _ => None,
                })
                .or_else(|| known.and_then(|found| found.config_schema.clone()))
                .or_else(|| core_schema(node.definition.id.as_str())),
            variadic,
            references: known
                .iter()
                .flat_map(|found| found.references.iter())
                .map(|field| (field.pointer.clone(), field.kind))
                .collect(),
            types,
            paths: shown.map(|shown| shown.paths.clone()).unwrap_or_default(),
        }
    }
}

impl Host {
    /// The ends of the top-level wire a problem is about, output first: where a canvas
    /// marks it. `None` when the problem names no wire of the top level.
    pub fn wire_of(
        &self,
        draft: &Draft,
        diagnostic: &Diagnostic,
    ) -> Option<((NodeId, String), (NodeId, String))> {
        if diagnostic.subject.function.is_some() {
            return None;
        }
        let edge = draft
            .document()
            .root()
            .edges
            .get(&diagnostic.subject.edge?)?;
        Some((
            (edge.from.node, edge.from.port.to_string()),
            (edge.to.node, edge.to.port.to_string()),
        ))
    }
}

/// The contract of a call of one of the document's functions: the function's declared
/// kind and interface, so the call is drawn and wired like any node. `None` for any
/// other node, or a call whose function is missing or malformed.
pub(crate) fn call_contract(
    document: &GraphDocument,
    node: &NodeInstance,
    registry: &RegistrySnapshot,
) -> Option<orange_model::catalog::NodeContract> {
    let interface = orange_model::catalog::call_interface(document, node, registry)
        .ok()
        .flatten()?;
    let function = document.functions().get(&interface.function)?;
    Some(orange_model::catalog::NodeContract {
        kind: function.kind,
        suspends: None,
        type_params: interface.type_params,
        ports: interface.ports,
    })
}

/// What an operator computes, written with its inputs' names, such as `a > b` for
/// Greater: a card shows it under its title, with the inputs' values in place of their
/// names when they are known. `None` for a node that is not an operator.
pub fn formula(definition: &str) -> Option<&'static str> {
    let name = definition.strip_prefix("org.orange.math/")?;
    let name = name.strip_prefix("float-").unwrap_or(name);
    Some(match name {
        "greater" => "a > b",
        "greater-or-equal" => "a >= b",
        "less" => "a < b",
        "less-or-equal" => "a <= b",
        "equal" => "a == b",
        "not-equal" => "a != b",
        "add" => "a + b",
        "subtract" => "a - b",
        "multiply" => "a × b",
        "divide" => "a / b",
        "remainder" | "modulo" => "a % b",
        "power" => "base ^ exponent",
        "minimum" => "min(a, b)",
        "maximum" => "max(a, b)",
        _ => return None,
    })
}

/// What a trigger waits for, from its settings: `every 1 s`, `after 300 ms`, or what a
/// manual entry fires with.
fn trigger_detail(definition: &str, config: &serde_json::Map<String, Json>) -> Option<String> {
    let text = |key: &str| config.get(key).and_then(Json::as_str);
    match definition {
        "core/on-timer" => Some(format!("every {}", human_duration(text("interval")?)?)),
        "core/on-timeout" => Some(format!("after {}", human_duration(text("delay")?)?)),
        "core/on-manual" => text("payload").map(|ty| {
            let short = ty.rsplit('/').next().unwrap_or(ty);
            format!("with {}", short.split('@').next().unwrap_or(short))
        }),
        _ => None,
    }
}

/// Nanoseconds, as the format stores a length of time, read as a person would say it.
fn human_duration(nanoseconds: &str) -> Option<String> {
    let ns: u128 = nanoseconds.parse().ok()?;
    let units = [
        (3_600_000_000_000, "h"),
        (60_000_000_000, "min"),
        (1_000_000_000, "s"),
        (1_000_000, "ms"),
        (1_000, "µs"),
    ];
    for (size, unit) in units {
        if ns >= size && ns.is_multiple_of((size / 10).max(1)) {
            let whole = ns / size;
            let tenths = (ns % size) * 10 / size;
            return Some(if tenths == 0 {
                format!("{whole} {unit}")
            } else {
                format!("{whole}.{tenths} {unit}")
            });
        }
    }
    Some(format!("{ns} ns"))
}

#[cfg(test)]
mod trigger_detail_tests {
    use super::*;

    #[test]
    fn a_trigger_says_what_fires_it() {
        let config = |json: Json| json.as_object().cloned().unwrap();
        assert_eq!(
            trigger_detail(
                "core/on-timer",
                &config(serde_json::json!({"interval": "1000000000"}))
            )
            .as_deref(),
            Some("every 1 s")
        );
        assert_eq!(
            trigger_detail(
                "core/on-timeout",
                &config(serde_json::json!({"delay": "300000000"}))
            )
            .as_deref(),
            Some("after 300 ms")
        );
        assert_eq!(human_duration("1500000000").as_deref(), Some("1.5 s"));
        assert_eq!(human_duration("120000000000").as_deref(), Some("2 min"));
        assert_eq!(
            trigger_detail(
                "core/on-manual",
                &config(serde_json::json!({"payload": "core/i64@1"}))
            )
            .as_deref(),
            Some("with i64")
        );
        assert_eq!(
            trigger_detail("core/on-start", &config(serde_json::json!({}))),
            None
        );
    }
}
