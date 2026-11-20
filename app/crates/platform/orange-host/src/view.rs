//! A document as a canvas draws it: each node with its name, ports and place, and the
//! wires between them. A front end needs nothing else of the model to show a graph.
use super::*;
use orange_model::catalog::{NominalAccess, Ports};
use orange_model::document::{
    ConfigValue, ConnectionDecl, EventDecl, FunctionDecl, NodeInstance, NodeKind, PortDirection,
    PortKind, SignatureSnapshot, VariableDecl,
};
use orange_model::id::{ConnectionId, EdgeId, EventDeclId, FunctionId, SignatureKey};
use serde_json::{Value as Json, json};

fn spectrum_band_label(node: &NodeInstance, key: &PortKey) -> Option<String> {
    if node.definition.id.as_str() != "org.orange.audio/multiband-spectrum" {
        return None;
    }
    let id = key.instance()?;
    let bands = node.config.get("bands")?.as_object()?;
    if key.port().as_str() != "band_stream" {
        return None;
    }
    let band = bands.get(&id.to_string())?;
    let low = band["low_hz"].as_f64()?;
    let high = band["high_hz"].as_f64()?;
    let name = band["name"].as_str()?;
    Some(if name == format!("Band {}", u32::from(id) + 1) {
        format!("{low}–{high} Hz")
    } else {
        format!("{name} · {low}–{high} Hz")
    })
}

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
    /// The value an input takes when nothing is written or wired to it, from its
    /// definition, as a person reads it; a card shows it dimmed in the empty field.
    pub default: Option<String>,
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

/// One choice of an enum type, as an editor offers it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumChoice {
    /// What values store, such as `happy`.
    pub id: String,
    /// What a person reads: the document's name for it, or its ID.
    pub name: String,
    /// The choice carries a value, so it cannot be written by name alone.
    pub payload: bool,
    /// The literal that writes this choice on an input, for [`Host::set_input`].
    pub literal: String,
}

impl Host {
    /// The choices of enum type `ty` in a draft, in declared order, named as the
    /// document declares them; `None` when `ty` is not an enum it knows.
    pub fn enum_choices(&self, draft: &Draft, ty: &str) -> Option<Vec<EnumChoice>> {
        let document = draft.document();
        enum_choices_in(document, &self.registry_for(document), ty)
    }
}

pub(crate) fn enum_choices_in(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    ty: &str,
) -> Option<Vec<EnumChoice>> {
    let found = registry
        .types()
        .nominal_types()
        .into_iter()
        .find(|nominal| !nominal.record && nominal.name == ty)?;
    let declared = document
        .types()
        .values()
        .find(|local| local.ty().to_string() == ty)
        .and_then(|local| match &local.shape {
            TypeShape::Enum { variants } => Some(variants),
            TypeShape::Record { .. } => None,
        });
    Some(
        found
            .members
            .iter()
            .map(|id| EnumChoice {
                name: declared
                    .and_then(|variants| variants.iter().find(|variant| &variant.id == id))
                    .map_or_else(|| id.clone(), |variant| variant.name.clone()),
                payload: found.with_payload.contains(id),
                literal: json!({"type": ty, "value": {"variant": id}}).to_string(),
                id: id.clone(),
            })
            .collect(),
    )
}

/// A value as a person reads it: an enum's choice by its name.
pub(crate) fn value_text(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    ty: Option<&str>,
    value: &Value,
) -> String {
    if let (Value::Enum { tag, payload: None }, Some(ty)) = (value, ty)
        && let Some(choice) = enum_choices_in(document, registry, ty)
            .and_then(|choices| choices.into_iter().nth(usize::try_from(*tag).ok()?))
    {
        return choice.name;
    }
    plain(value)
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
    /// Shared, so a view built again after an edit keeps the cards the edit left alone
    /// without copying them.
    pub nodes: Vec<Arc<NodeView>>,
    pub wires: Vec<WireView>,
}

/// The cards of a draft's last view and the nodes edited since, so the next view
/// builds only those again (docs/10 "Performance").
#[derive(Default)]
pub(crate) struct ViewCache {
    /// What every card was drawn with; a change to any of it draws every card again.
    shared: Option<SharedKey>,
    nodes: std::collections::HashMap<NodeId, Arc<NodeView>>,
    /// Each top-level wire's ends, to find the cards a wire added or removed changes.
    edges: std::collections::HashMap<EdgeId, (NodeId, NodeId)>,
    /// Nodes edited since the last view.
    edited: std::collections::HashSet<NodeId>,
}

impl std::fmt::Debug for ViewCache {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ViewCache")
            .field("cards", &self.nodes.len())
            .finish_non_exhaustive()
    }
}

impl ViewCache {
    /// Notes the nodes one edit of the draft changed.
    pub(crate) fn follow(&mut self, change: &orange_model::ChangeSet) {
        if self.nodes.is_empty() {
            return;
        }
        if change.global_dirty() {
            self.nodes.clear();
            return;
        }
        let nodes = change.nodes();
        self.edited
            .extend(nodes.added.iter().chain(&nodes.changed).copied());
        for removed in &nodes.removed {
            let _ = self.nodes.remove(removed);
        }
    }

    /// Forgets every card when what all of them were drawn with changed, and the cards
    /// of nodes edited, or whose wires changed, since the last view.
    fn forget_changed(&mut self, context: &ViewContext<'_>) {
        if !self
            .shared
            .as_ref()
            .is_some_and(|shared| shared.is(context))
        {
            self.nodes.clear();
            self.shared = Some(SharedKey::of(context));
        }
        for node in self.edited.drain() {
            let _ = self.nodes.remove(&node);
        }
        let mut edges = std::collections::HashMap::with_capacity(self.edges.len());
        for (id, edge) in &context.document.root().edges {
            let ends = (edge.from.node, edge.to.node);
            if self.edges.remove(id) != Some(ends) {
                let _ = self.nodes.remove(&ends.0);
                let _ = self.nodes.remove(&ends.1);
            }
            let _ = edges.insert(*id, ends);
        }
        // What is left was taken away.
        for (from, to) in std::mem::replace(&mut self.edges, edges).into_values() {
            let _ = self.nodes.remove(&from);
            let _ = self.nodes.remove(&to);
        }
    }
}

/// The parts of a document and host that any card may show.
struct SharedKey {
    registry: Arc<RegistrySnapshot>,
    labels: BTreeMap<String, String>,
    variables: Vec<(VariableId, VariableDecl)>,
    events: Vec<(EventDeclId, EventDecl)>,
    connections: Vec<(ConnectionId, ConnectionDecl)>,
    functions: Vec<(FunctionId, FunctionDecl)>,
    signatures: Vec<(SignatureKey, SignatureSnapshot)>,
}

/// A copy of a map's entries, to compare with it later.
fn entries<'a, K: Clone + 'a, V: Clone + 'a>(
    map: impl IntoIterator<Item = (&'a K, &'a V)>,
) -> Vec<(K, V)> {
    map.into_iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

/// Whether `kept` holds the same entries as `map`, in the same order.
fn same<'a, K: PartialEq + 'a, V: PartialEq + 'a>(
    kept: &[(K, V)],
    map: impl ExactSizeIterator<Item = (&'a K, &'a V)>,
) -> bool {
    kept.len() == map.len()
        && kept
            .iter()
            .zip(map)
            .all(|((key, value), (other, its))| key == other && value == its)
}

impl SharedKey {
    fn of(context: &ViewContext<'_>) -> Self {
        let document = context.document;
        Self {
            registry: Arc::clone(&context.registry),
            labels: context.labels.clone(),
            variables: entries(document.variables()),
            events: entries(document.events()),
            connections: entries(document.connections()),
            functions: entries(document.functions()),
            signatures: entries(document.signatures()),
        }
    }

    fn is(&self, context: &ViewContext<'_>) -> bool {
        let document = context.document;
        Arc::ptr_eq(&self.registry, &context.registry)
            && self.labels == context.labels
            && same(&self.variables, document.variables().iter())
            && same(&self.events, document.events().iter())
            && same(&self.connections, document.connections().iter())
            && same(&self.functions, document.functions().iter())
            && same(&self.signatures, document.signatures().iter())
    }
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
        let mut document =
            from_json(json).map_err(|error| HostError::Document(error.to_string()))?;
        let _ = self.replace_definitions(&mut document)?;
        Ok(self.view_of(&document))
    }

    /// [`Host::view`] of a draft. The cards of nodes the last edits left alone, with
    /// the same wired ports, place and name in an unchanged document context, are
    /// reused from the draft's previous view instead of being built again.
    pub fn draft_view(&self, draft: &Draft) -> Result<GraphView, HostError> {
        // Keep package definitions and saved settings in the editor. Runtime linking
        // replaces them with internal calls and can expand hidden implementation nodes.
        let mut cache = draft.view.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(self.view_with(draft.document(), Some(&mut cache)))
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
        self.view_with(document, None)
    }

    fn view_with(&self, document: &GraphDocument, cache: Option<&mut ViewCache>) -> GraphView {
        let context = self.view_context(document);
        let body = document.root();
        let nodes: Vec<Arc<NodeView>> = match cache {
            None => body
                .nodes
                .values()
                .map(|node| Arc::new(self.node_view_in(&context, node)))
                .collect(),
            Some(cache) => {
                cache.forget_changed(&context);
                body.nodes
                    .values()
                    .map(|node| {
                        let kept = cache
                            .nodes
                            .get(&node.id)
                            .filter(|kept| kept.name == context.names.called(&node.id));
                        let view = match kept {
                            // Moving a node is a layout edit, which names no node.
                            Some(kept) => {
                                let position = position_in(context.places, node.id);
                                if kept.position == position {
                                    Arc::clone(kept)
                                } else {
                                    Arc::new(NodeView {
                                        position,
                                        ..NodeView::clone(kept)
                                    })
                                }
                            }
                            None => Arc::new(self.node_view_in(&context, node)),
                        };
                        let _ = cache.nodes.insert(node.id, Arc::clone(&view));
                        view
                    })
                    .collect()
            }
        };
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
                if node.definition.id.as_str() == "org.orange.audio/multiband-spectrum"
                    && id == "spectrum"
                    && !wired(node.id, &id, input)
                {
                    continue;
                }
                let ty = port.ty.as_ref().map(|ty| match &settled {
                    Some(args) => orange_model::signature::substitute_params(ty, args).to_string(),
                    None => ty.to_string(),
                });
                let value = node
                    .inputs
                    .get(&key)
                    .and_then(|literal| self.value_in(document, literal).ok())
                    .map(|value| value_text(document, registry, ty.as_deref(), &value));
                // Read as a value is: an enum's default by its choice's name.
                let default = port
                    .default
                    .as_ref()
                    .filter(|_| input)
                    .and_then(|literal| self.value_in(document, literal).ok())
                    .map(|value| value_text(document, registry, ty.as_deref(), &value));
                declared.push(PortView {
                    connected: wired(node.id, &id, input),
                    input,
                    exec: port.kind != PortKind::Data,
                    ty,
                    value: value.filter(|_| input),
                    default,
                    id,
                    label: spectrum_band_label(node, &key),
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
            let outcomes = contract.as_ref().is_some_and(|contract| {
                contract.ports.iter().any(|port| {
                    port.kind == PortKind::Exec && port.direction == PortDirection::Output
                })
            });
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
                    default: None,
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
        let position = position_in(*places, node.id);
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
            entry: kind == Some(NodeKind::Entry) && ports.iter().any(|port| port.exec),
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

/// Where the document's layout (`editor.nodes`) places `node`.
fn position_in(places: Option<&Json>, node: NodeId) -> Option<[f32; 2]> {
    places
        .and_then(|places| places.get(node.to_string()))
        .and_then(|place| Some([place["x"].as_f64()?, place["y"].as_f64()?]))
        .map(|[x, y]| {
            #[expect(clippy::cast_possible_truncation, reason = "screen coordinates")]
            [x as f32, y as f32]
        })
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
            trigger_detail("core/on-timer", &config(json!({"interval": "1000000000"}))).as_deref(),
            Some("every 1 s")
        );
        assert_eq!(
            trigger_detail("core/on-timeout", &config(json!({"delay": "300000000"}))).as_deref(),
            Some("after 300 ms")
        );
        assert_eq!(human_duration("1500000000").as_deref(), Some("1.5 s"));
        assert_eq!(human_duration("120000000000").as_deref(), Some("2 min"));
        assert_eq!(
            trigger_detail("core/on-manual", &config(json!({"payload": "core/i64@1"}))).as_deref(),
            Some("with i64")
        );
        assert_eq!(trigger_detail("core/on-start", &config(json!({}))), None);
    }
}
