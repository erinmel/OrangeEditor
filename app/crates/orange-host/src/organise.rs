//! Organising a draft's canvas: groups of nodes drawn in a named, coloured frame, and
//! turning selected nodes into a graph function that a single call node uses.
//!
//! Groups are presentation (11 §14): they live in the document's layout under
//! `editor.groups`, which the compiler never reads, and each change is one layout edit
//! of the draft's undo history. Extracting a function is the editor refactoring of 11 §14
//! ("collapse to function"): one edit that moves the nodes into a [`FunctionDecl`], puts a
//! `core/call` where they were and rewires the boundary, kept only when it adds no error.
use super::*;
use draft::refused;
use orange_model::DocCommand;
use orange_model::LayoutCommand;
use orange_model::catalog::intrinsic_id as ids;
use orange_model::document::{
    BodyRef, Edge, FunctionDecl, GraphBody, NodeInstance, NodeKind, PortDescriptor, PortDirection,
    PortKind, PortRef,
};
use orange_model::id::{EdgeId, FunctionId, PortId, PortKey, TypeExpr};
use orange_model::validation::Severity;
use serde_json::{Value as Json, json};

/// Colours a new group takes in turn.
pub const GROUP_COLORS: [[u8; 3]; 6] = [
    [86, 156, 214],
    [106, 170, 100],
    [214, 157, 86],
    [180, 110, 200],
    [200, 90, 90],
    [90, 180, 180],
];

/// Nodes drawn together in a named, coloured frame that moves them as one and can be
/// collapsed to its title. A node is in at most one group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub id: String,
    pub name: String,
    pub color: [u8; 3],
    /// Top-level nodes of the draft, in the order they were grouped.
    pub nodes: Vec<NodeId>,
    pub collapsed: bool,
}

impl Group {
    fn layout(&self) -> Json {
        let [r, g, b] = self.color;
        json!({
            "name": self.name,
            "color": format!("#{r:02x}{g:02x}{b:02x}"),
            "nodes": self.nodes.iter().map(ToString::to_string).collect::<Vec<_>>(),
            "collapsed": self.collapsed,
        })
    }

    fn read(id: &str, saved: &Json) -> Self {
        let color = saved["color"]
            .as_str()
            .and_then(|text| text.strip_prefix('#'))
            .filter(|hex| hex.len() == 6)
            .and_then(|hex| {
                let channel = |at: usize| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok();
                Some([channel(0)?, channel(2)?, channel(4)?])
            })
            .unwrap_or(GROUP_COLORS[0]);
        Self {
            id: id.to_owned(),
            name: saved["name"].as_str().unwrap_or("Group").to_owned(),
            color,
            nodes: saved["nodes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|node| node.as_str()?.parse().ok())
                .collect(),
            collapsed: saved["collapsed"].as_bool().unwrap_or(false),
        }
    }
}

fn group_command(id: &str, group: Option<&Group>) -> DocCommand {
    DocCommand::Layout(LayoutCommand::Set {
        path: vec!["groups".to_owned(), id.to_owned()],
        value: group.map(Group::layout),
    })
}

impl Draft {
    /// The groups of the draft's top level, by name. A group lists only nodes that are
    /// still at the top level, and one with none of them left is not listed: a node
    /// taken into a function or deleted leaves its group, and comes back with an undo.
    pub fn groups(&self) -> Vec<Group> {
        let root = &self.document().root().nodes;
        let mut groups: Vec<Group> = self
            .document()
            .editor()
            .get("groups")
            .and_then(Json::as_object)
            .into_iter()
            .flatten()
            .map(|(id, saved)| {
                let mut group = Group::read(id, saved);
                group.nodes.retain(|node| root.contains_key(node));
                group
            })
            .filter(|group| !group.nodes.is_empty())
            .collect();
        groups.sort_by(|a, b| (&a.name, &a.id).cmp(&(&b.name, &b.id)));
        groups
    }

    /// Groups `nodes` under `name`, taking them out of any group they were in, as one
    /// edit. Returns the new group's ID.
    pub fn group(&mut self, nodes: &[NodeId], name: &str) -> Result<String, HostError> {
        let root = &self.document().root().nodes;
        let mut members: Vec<NodeId> = Vec::new();
        for node in nodes {
            if !root.contains_key(node) {
                return Err(HostError::Document("the document has no such node".into()));
            }
            if !members.contains(node) {
                members.push(*node);
            }
        }
        if members.is_empty() {
            return Err(HostError::Document(
                "choose the nodes to group first".into(),
            ));
        }
        let saved = self.saved_groups();
        let mut commands: Vec<DocCommand> = saved
            .iter()
            .filter(|group| group.nodes.iter().any(|node| members.contains(node)))
            .map(|group| {
                let mut kept = group.clone();
                kept.nodes.retain(|node| !members.contains(node));
                group_command(&group.id, (!kept.nodes.is_empty()).then_some(&kept))
            })
            .collect();
        let id = self.fresh_id("group");
        let name = name.trim();
        let group = Group {
            id: id.clone(),
            name: if name.is_empty() { "Group" } else { name }.to_owned(),
            color: GROUP_COLORS[saved.len() % GROUP_COLORS.len()],
            nodes: members,
            collapsed: false,
        };
        commands.push(group_command(&id, Some(&group)));
        let _ = self.layout(commands)?;
        Ok(id)
    }

    /// Changes a group's name, colour, members or whether it is collapsed, as one edit.
    /// Returns whether anything changed.
    pub fn set_group(&mut self, group: &Group) -> Result<bool, HostError> {
        let Some(saved) = self
            .saved_groups()
            .into_iter()
            .find(|saved| saved.id == group.id)
        else {
            return Err(HostError::Document("the document has no such group".into()));
        };
        if saved == *group {
            return Ok(false);
        }
        self.layout(vec![group_command(&group.id, Some(group))])
    }

    /// Takes a group's frame away, leaving its nodes where they are.
    pub fn ungroup(&mut self, id: &str) -> Result<bool, HostError> {
        if !self.saved_groups().iter().any(|saved| saved.id == id) {
            return Ok(false);
        }
        self.layout(vec![group_command(id, None)])
    }

    /// Every group as saved, members that are gone included.
    fn saved_groups(&self) -> Vec<Group> {
        self.document()
            .editor()
            .get("groups")
            .and_then(Json::as_object)
            .into_iter()
            .flatten()
            .map(|(id, saved)| Group::read(id, saved))
            .collect()
    }
}

/// A value that crosses the edge of the nodes being extracted.
#[derive(Debug, Clone)]
struct Crossing {
    name: PortId,
    ty: TypeExpr,
    /// For an input, the outside output it comes from; for an output, the inside one.
    source: PortRef,
    /// The inputs it goes to: inside ones for an input, outside ones for an output.
    targets: Vec<PortRef>,
}

/// What extracting a selection would make, before anything is changed.
#[derive(Debug)]
struct Plan {
    nodes: Vec<NodeId>,
    kind: NodeKind,
    inputs: Vec<Crossing>,
    outputs: Vec<Crossing>,
    /// The inside input the steps come in at, and the outside outputs they come from.
    step_in: Option<(PortRef, Vec<PortRef>)>,
    /// The inside output the steps leave from, and the outside input they go to.
    step_out: Option<(PortRef, PortRef)>,
    /// Wires between the extracted nodes, which go with them.
    inner: Vec<EdgeId>,
}

/// What [`Host::extract`] would make of a selection: the function's kind and its typed
/// inputs and outputs, shown before anything is changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extraction {
    /// `pure`, `volatile`, `action` or `async`.
    pub kind: String,
    /// Each input and output by name, with its type such as `core/i64@1`.
    pub inputs: Vec<(String, String)>,
    pub outputs: Vec<(String, String)>,
}

fn rank(kind: NodeKind) -> u8 {
    match kind {
        NodeKind::Pure => 0,
        NodeKind::Volatile => 1,
        NodeKind::Action => 2,
        _ => 3,
    }
}

fn kind_word(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Pure => "pure",
        NodeKind::Volatile => "volatile",
        NodeKind::Action => "action",
        NodeKind::Async => "async",
        NodeKind::Entry => "entry",
        NodeKind::Iterator => "iterator",
    }
}

/// Port names a function may not declare: those kinds supply (11 §4.1).
const RESERVED: [&str; 7] = [
    "in",
    "then",
    "failed",
    "error",
    "cancelled",
    "body",
    "completed",
];

/// A port name not used yet, from the name of the port it stands for.
fn unique_name(base: &str, used: &mut BTreeSet<String>) -> Result<PortId, HostError> {
    let base = if RESERVED.contains(&base) {
        format!("{base}_value")
    } else {
        base.to_owned()
    };
    let mut name = base.clone();
    let mut n = 2;
    while used.contains(&name) {
        name = format!("{base}_{n}");
        n += 1;
    }
    let _ = used.insert(name.clone());
    name.parse()
        .map_err(|_| refused(&format!("`{name}` cannot name a port")))
}

fn marker(id: NodeId, definition: &str) -> Result<NodeInstance, HostError> {
    Ok(NodeInstance {
        id,
        definition: DefinitionRef {
            id: definition
                .parse()
                .map_err(|_| refused(&"no such function marker"))?,
            version: 1,
        },
        config_version: 1,
        config: Default::default(),
        type_args: Default::default(),
        type_pins: Default::default(),
        provider: None,
        signature: None,
        inputs: Default::default(),
        variadic: Default::default(),
        activation: None,
        extensions: Default::default(),
    })
}

fn port_on(node: NodeId, port: &PortId) -> PortRef {
    PortRef {
        node,
        port: PortKey::new(port.clone(), None),
    }
}

impl Host {
    /// The kind and typed ports of a top-level node: its definition's, or for a call
    /// the called function's. `None` when this host does not know the node.
    fn contract_of(
        &self,
        document: &GraphDocument,
        node: &NodeInstance,
    ) -> Option<orange_model::catalog::NodeContract> {
        let registry = self.registry_for(document);
        let registry = &registry;
        if let Some(contract) = view::call_contract(document, node, registry) {
            return Some(contract);
        }
        match registry.contract(document, node) {
            Ok(Some(contract)) => Some(contract.into_owned()),
            _ => None,
        }
    }

    /// The type of a value port of a top-level node, with the node's settled type
    /// arguments put in; `None` when it is not known or still open.
    fn port_type(&self, document: &GraphDocument, end: &PortRef) -> Option<TypeExpr> {
        let node = document.root().nodes.get(&end.node)?;
        let contract = self.contract_of(document, node)?;
        let port = contract
            .ports
            .iter()
            .find(|port| &port.port_id == end.port.port() && port.kind == PortKind::Data)?;
        let ty = port.ty.as_ref()?;
        let mut args = node.type_args.clone();
        args.extend(node.type_pins.clone());
        let ty = orange_model::signature::substitute_params(ty, &args);
        (!ty.to_string().contains('$')).then_some(ty)
    }

    /// Whether `end` is a step port of its node, as the view draws it.
    fn is_step(&self, document: &GraphDocument, end: &PortRef, input: bool) -> bool {
        self.node_view(document, end.node).is_some_and(|node| {
            node.ports
                .iter()
                .any(|port| port.input == input && port.id == end.port.to_string() && port.exec)
        })
    }

    fn plan(&self, document: &GraphDocument, nodes: &[NodeId]) -> Result<Plan, HostError> {
        let root = document.root();
        let mut chosen: Vec<NodeId> = Vec::new();
        for node in nodes {
            if !root.nodes.contains_key(node) {
                return Err(HostError::Document("the document has no such node".into()));
            }
            if !chosen.contains(node) {
                chosen.push(*node);
            }
        }
        if chosen.is_empty() {
            return Err(HostError::Document(
                "choose the nodes to make a function of first".into(),
            ));
        }
        let names = Names::of_document(self, document);
        let mut kind = NodeKind::Pure;
        for id in &chosen {
            let node = &root.nodes[id];
            let contract = self.contract_of(document, node).ok_or_else(|| {
                refused(&format!(
                    "`{}` is not set up yet, or this application does not have its kind",
                    names.called(id)
                ))
            })?;
            if matches!(contract.kind, NodeKind::Entry | NodeKind::Iterator) {
                return Err(refused(&format!(
                    "`{}` is {}: a function cannot hold a trigger or a stream loop (11 §14)",
                    names.called(id),
                    if contract.kind == NodeKind::Entry {
                        "a trigger"
                    } else {
                        "a stream loop"
                    }
                )));
            }
            if rank(contract.kind) > rank(kind) {
                kind = contract.kind;
            }
        }
        let inside = |node: &NodeId| chosen.contains(node);
        let mut used = BTreeSet::new();
        let mut inputs: Vec<Crossing> = Vec::new();
        let mut outputs: Vec<Crossing> = Vec::new();
        let mut inner = Vec::new();
        let mut step_in: Option<(PortRef, Vec<PortRef>)> = None;
        let mut step_out: Option<(PortRef, PortRef)> = None;
        let type_of = |end: &PortRef| self.port_type(document, end);
        for edge in root.edges.values() {
            match (inside(&edge.from.node), inside(&edge.to.node)) {
                (true, true) => inner.push(edge.id),
                (false, false) => {}
                (false, true) if self.is_step(document, &edge.to, true) => match &mut step_in {
                    None => step_in = Some((edge.to.clone(), vec![edge.from.clone()])),
                    Some((target, sources)) if *target == edge.to => {
                        sources.push(edge.from.clone());
                    }
                    Some(_) => {
                        return Err(refused(
                            &"the steps come into the chosen nodes at more than one place; \
                              a function has one way in",
                        ));
                    }
                },
                (true, false) if self.is_step(document, &edge.from, false) => {
                    if step_out.is_some() {
                        return Err(refused(
                            &"the steps leave the chosen nodes at more than one place; \
                              a function has one way on",
                        ));
                    }
                    step_out = Some((edge.from.clone(), edge.to.clone()));
                }
                (false, true) => {
                    if let Some(crossing) = inputs.iter_mut().find(|c| c.source == edge.from) {
                        crossing.targets.push(edge.to.clone());
                        continue;
                    }
                    let ty = type_of(&edge.to)
                        .or_else(|| type_of(&edge.from))
                        .ok_or_else(|| {
                            refused(&format!(
                                "the type going into `{}` is not settled yet; choose it first",
                                names.called(&edge.to.node)
                            ))
                        })?;
                    inputs.push(Crossing {
                        name: unique_name(edge.to.port.port().as_str(), &mut used)?,
                        ty,
                        source: edge.from.clone(),
                        targets: vec![edge.to.clone()],
                    });
                }
                (true, false) => {
                    if let Some(crossing) = outputs.iter_mut().find(|c| c.source == edge.from) {
                        crossing.targets.push(edge.to.clone());
                        continue;
                    }
                    let ty = type_of(&edge.from)
                        .or_else(|| type_of(&edge.to))
                        .ok_or_else(|| {
                            refused(&format!(
                                "the type coming out of `{}` is not settled yet; choose it first",
                                names.called(&edge.from.node)
                            ))
                        })?;
                    outputs.push(Crossing {
                        name: unique_name(edge.from.port.port().as_str(), &mut used)?,
                        ty,
                        source: edge.from.clone(),
                        targets: vec![edge.to.clone()],
                    });
                }
            }
        }
        Ok(Plan {
            nodes: chosen,
            kind,
            inputs,
            outputs,
            step_in,
            step_out,
            inner,
        })
    }

    /// What [`Host::extract`] would make of `nodes`: the function's kind and its typed
    /// inputs and outputs. Nothing is changed. Refused, with the reason, for a selection
    /// that cannot be a function: one with a trigger, or entered or left by steps at more
    /// than one place, or whose crossing types are not settled.
    pub fn extraction(&self, draft: &Draft, nodes: &[NodeId]) -> Result<Extraction, HostError> {
        let plan = self.plan(draft.document(), nodes)?;
        let side = |crossings: &[Crossing]| {
            crossings
                .iter()
                .map(|c| (c.name.to_string(), c.ty.to_string()))
                .collect()
        };
        Ok(Extraction {
            kind: kind_word(plan.kind).to_owned(),
            inputs: side(&plan.inputs),
            outputs: side(&plan.outputs),
        })
    }

    /// Makes a graph function named `name` of top-level `nodes` and puts a call to it
    /// at `at`, as one edit that can be undone (11 §14). Wires between the nodes go with
    /// them; a value wired in from outside becomes an input of the function, a value
    /// used outside becomes an output, and steps coming in and going on pass through the
    /// call. If the result would have an error the draft did not have, nothing is
    /// changed and the error says why. Returns the call node.
    pub fn extract(
        &self,
        draft: &mut Draft,
        nodes: &[NodeId],
        name: &str,
        at: [f32; 2],
    ) -> Result<NodeId, HostError> {
        let plan = self.plan(draft.document(), nodes)?;
        let name = name.trim();
        if name.is_empty() {
            return Err(refused(&"give the function a name"));
        }
        let before = errors(&self.problems(draft)?);
        let document = draft.document();
        let new_id = |what: &str| draft.fresh_id(&format!("extract-{what}"));
        let node_id = |what: &str| -> Result<NodeId, HostError> {
            new_id(what)
                .parse()
                .map_err(|_| refused(&"the new ID is not valid"))
        };
        let mut edge_count = 0usize;
        let mut new_edge = |from: PortRef, to: PortRef| -> Result<Edge, HostError> {
            edge_count += 1;
            Ok(Edge {
                id: new_id(&format!("edge-{edge_count}"))
                    .parse()
                    .map_err(|_| refused(&"the new ID is not valid"))?,
                from,
                to,
                extensions: Default::default(),
            })
        };
        let function: FunctionId = new_id("function")
            .parse()
            .map_err(|_| refused(&"the new ID is not valid"))?;
        let call = node_id("call")?;
        let acts = rank(plan.kind) >= rank(NodeKind::Action);

        let mut body = GraphBody::default();
        for id in &plan.nodes {
            let node = document.root().nodes[id].clone();
            let _ = body.nodes.insert(node.id, node);
        }
        for id in &plan.inner {
            let edge = document.root().edges[id].clone();
            let _ = body.edges.insert(edge.id, edge);
        }
        let needs_entry = acts || !plan.inputs.is_empty();
        let entry = node_id("entry")?;
        if needs_entry {
            let _ = body
                .nodes
                .insert(entry, marker(entry, ids::FUNCTION_ENTRY)?);
        }
        for input in &plan.inputs {
            for target in &input.targets {
                let edge = new_edge(port_on(entry, &input.name), target.clone())?;
                let _ = body.edges.insert(edge.id, edge);
            }
        }
        let then: PortId = "then".parse().map_err(|_| refused(&"port"))?;
        let step: PortId = "in".parse().map_err(|_| refused(&"port"))?;
        if let Some((target, _)) = &plan.step_in {
            let edge = new_edge(port_on(entry, &then), target.clone())?;
            let _ = body.edges.insert(edge.id, edge);
        }
        // Results leave through a return on an action's way on, otherwise through the
        // pure function's outputs.
        let finish = node_id("finish")?;
        let returns = acts && (plan.step_out.is_some() || !plan.outputs.is_empty());
        if returns || (!acts && !plan.outputs.is_empty()) {
            let definition = if acts {
                ids::RETURN
            } else {
                ids::FUNCTION_OUTPUTS
            };
            let _ = body.nodes.insert(finish, marker(finish, definition)?);
        }
        if returns && let Some((source, _)) = &plan.step_out {
            let edge = new_edge(source.clone(), port_on(finish, &step))?;
            let _ = body.edges.insert(edge.id, edge);
        }
        for output in &plan.outputs {
            let edge = new_edge(output.source.clone(), port_on(finish, &output.name))?;
            let _ = body.edges.insert(edge.id, edge);
        }
        let descriptor = |crossing: &Crossing, direction: PortDirection| PortDescriptor {
            port_id: crossing.name.clone(),
            direction,
            kind: PortKind::Data,
            ty: Some(crossing.ty.clone()),
            source: None,
            available_on: None,
            default: None,
            sensitivity: None,
            optional: None,
            variadic: None,
            lazy: None,
            extensions: Default::default(),
        };
        let declaration = FunctionDecl {
            id: function,
            name: name.to_owned(),
            kind: plan.kind,
            type_params: Vec::new(),
            inputs: plan
                .inputs
                .iter()
                .map(|c| descriptor(c, PortDirection::Input))
                .collect(),
            outputs: plan
                .outputs
                .iter()
                .map(|c| descriptor(c, PortDirection::Output))
                .collect(),
            body,
            extensions: Default::default(),
        };
        let mut caller = marker(call, ids::CALL)?;
        let _ = caller
            .config
            .insert("function".to_owned(), json!(function.to_string()));
        let _ = caller.extensions.insert("name".to_owned(), json!(name));

        let mut commands = vec![
            DocCommand::RemoveNodes {
                ids: plan.nodes.clone(),
            },
            DocCommand::UpsertFunction(declaration),
            DocCommand::AddNode {
                body: BodyRef::Root,
                node: caller,
            },
        ];
        let mut connect = |from: PortRef, to: PortRef| -> Result<(), HostError> {
            commands.push(DocCommand::Connect {
                edge: new_edge(from, to)?,
            });
            Ok(())
        };
        for input in &plan.inputs {
            connect(input.source.clone(), port_on(call, &input.name))?;
        }
        for output in &plan.outputs {
            for target in &output.targets {
                connect(port_on(call, &output.name), target.clone())?;
            }
        }
        if let Some((_, sources)) = &plan.step_in {
            for source in sources {
                connect(source.clone(), port_on(call, &step))?;
            }
        }
        if let Some((_, target)) = &plan.step_out {
            connect(port_on(call, &then), target.clone())?;
        }
        commands.push(edit::place_command(call, Some(at)));

        let Some(redo) = draft.try_commit(&self.registry_for(draft.document()), commands)? else {
            return Err(refused(&"nothing to extract"));
        };
        let after = self.problems(draft)?;
        if let Some(new) = after
            .iter()
            .find(|found| found.severity == Severity::Error && !before.contains(&key(found)))
        {
            let names = Names::of_document(self, draft.document());
            let why = names::describe(new, &names);
            draft.withdraw(redo);
            return Err(refused(&format!(
                "these nodes cannot be a function as they are: {why}"
            )));
        }
        Ok(call)
    }
}

/// A problem by what it says and where, without the function or wire it is in: the
/// same problem of a node moved into a function is not a new one.
type ProblemKey = (String, Option<NodeId>, Option<String>, Option<VariableId>);

fn key(found: &Diagnostic) -> ProblemKey {
    (
        format!("{:?}", found.code),
        found.subject.node,
        found.subject.port.as_ref().map(ToString::to_string),
        found.subject.variable,
    )
}

fn errors(found: &[Diagnostic]) -> BTreeSet<ProblemKey> {
    found
        .iter()
        .filter(|found| found.severity == Severity::Error)
        .map(key)
        .collect()
}
