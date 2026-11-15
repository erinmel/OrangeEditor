//! Editing a draft: adding and removing nodes, wiring ports, writing values on inputs
//! and placing nodes. Each operation is one model transaction on the [`Draft`], so it
//! is one step of the draft's undo history, and its validation follows it.
//!
//! None of them runs anything; a flow of the document is changed only when the front
//! end hands the draft's text to [`Host::update`].
use super::*;
use draft::refused;
use orange_model::binding::{NewNode, new_node_edit};
use orange_model::catalog::{NominalAccess, Ports, RegistrySnapshot};
use orange_model::document::ConfigValue;
use orange_model::document::{
    ActivationPolicy, BodyRef, Edge, NodeInstance, NodeKind, PortRef, ProviderBinding,
};
use orange_model::id::{PortId, PortKey};
use orange_model::{DocCommand, LayoutCommand};
use serde_json::{Value as Json, json};

/// One end of a wire: a node and the name of one of its ports.
pub type PortEnd<'a> = (NodeId, &'a str);

/// The text of a new, empty document with an ID of its own.
pub fn new_document() -> Result<String, HostError> {
    // The time and the process tell one new document from another.
    let moment = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    let mut hasher = Sha256::new();
    hasher.update(b"graph");
    hasher.update(moment.to_le_bytes());
    hasher.update(std::process::id().to_le_bytes());
    let graph = draft::uuid_text(&hasher.finalize())
        .parse()
        .map_err(|_| refused(&"the new ID is not valid"))?;
    to_json(&GraphDocument::new(graph)).map_err(|error| refused(&error))
}

fn port_ref(end: PortEnd<'_>) -> Result<PortRef, HostError> {
    Ok(PortRef {
        node: end.0,
        port: end
            .1
            .parse()
            .map_err(|_| HostError::Document(format!("`{}` is not a port name", end.1)))?,
    })
}

/// The activation a new entry node gets: a queue of sixteen that faults when full.
pub(crate) fn default_activation() -> Json {
    json!({"mode": "queue", "capacity": 16, "overflow": "fault", "on_fault": "continue",
        "order": 0})
}

/// The bound a new while loop starts with, so it is valid until a person changes it.
pub const NEW_WHILE_BOUND: u64 = 1000;

/// The settings a new node of `found` starts with: a while loop's bound, and the first
/// registered type (and member) a record or enum node can name. A record or enum
/// node with no type to name cannot be added, and the error says so.
fn new_config(
    found: &NodeDefinition,
    registry: &RegistrySnapshot,
) -> Result<ConfigValue, HostError> {
    let mut config = ConfigValue::new();
    if found.definition.id.as_str() == "core/while" {
        let _ = config.insert("max_iterations".to_owned(), json!(NEW_WHILE_BOUND));
    }
    if let Ports::Nominal(access) = found.ports {
        let record = matches!(
            access,
            NominalAccess::MakeRecord | NominalAccess::GetField | NominalAccess::SetField
        );
        let choices = view::nominal_choices(access, registry);
        let Some((ty, members)) = choices.first() else {
            return Err(HostError::Document(format!(
                "`{}` names {} type, and no active package declares one",
                found.definition.id,
                if record { "a record" } else { "an enum" }
            )));
        };
        let _ = config.insert("type".to_owned(), json!(ty));
        if let (Some(member), Some(first)) = (view::nominal_member(access), members.first()) {
            let _ = config.insert(member.to_owned(), json!(first));
        }
    }
    Ok(config)
}

impl Host {
    /// Adds an instance of a variadic port (such as another branch of a fork) after
    /// the node's last one. Returns its name, such as `branch#2`.
    pub fn add_port(
        &self,
        draft: &mut Draft,
        node: NodeId,
        port: &str,
    ) -> Result<String, HostError> {
        let view = self
            .node_view(draft.document(), node)
            .ok_or_else(|| HostError::Document("the document has no such node".into()))?;
        let found = view
            .variadic
            .iter()
            .find(|variadic| variadic.port == port)
            .ok_or_else(|| {
                HostError::Document(format!("the node has no variadic port `{port}`"))
            })?;
        if found.max.is_some_and(|max| found.count >= max) {
            return Err(HostError::Document(format!(
                "`{port}` has as many instances as it may"
            )));
        }
        let id: PortId = port
            .parse()
            .map_err(|_| HostError::Document(format!("`{port}` is not a port name")))?;
        let next = draft
            .document()
            .node(node)
            .and_then(|instance| instance.variadic.get(&id))
            .map_or(0, |state| state.next);
        let _ = draft.commit([DocCommand::InsertVariadic {
            node,
            port: id.clone(),
            at: found.count,
        }])?;
        Ok(PortKey::new(id, Some(next)).to_string())
    }

    /// Removes one instance of a variadic port (`branch#1`) with its wires and value.
    /// The fewest instances the port may have stay.
    pub fn remove_port(&self, draft: &mut Draft, at: PortEnd<'_>) -> Result<bool, HostError> {
        let key: PortKey =
            at.1.parse()
                .map_err(|_| HostError::Document(format!("`{}` is not a port name", at.1)))?;
        let view = self
            .node_view(draft.document(), at.0)
            .ok_or_else(|| HostError::Document("the document has no such node".into()))?;
        let found = view
            .variadic
            .iter()
            .find(|variadic| variadic.port == key.port().as_str() && key.instance().is_some())
            .ok_or_else(|| HostError::Document(format!("`{}` is not an added port", at.1)))?;
        if found.count <= found.min {
            return Err(HostError::Document(format!(
                "`{}` needs at least {} instance(s)",
                found.port, found.min
            )));
        }
        draft.commit([DocCommand::RemoveVariadic {
            node: at.0,
            port: key,
        }])
    }
}

/// The layout command that puts `node` at `at`, in whole units so a saved file stays
/// stable and readable, or takes its place off.
pub(crate) fn place_command(node: NodeId, at: Option<[f32; 2]>) -> DocCommand {
    DocCommand::Layout(LayoutCommand::Set {
        path: vec!["nodes".to_owned(), node.to_string()],
        value: at.map(|[x, y]| json!({"x": f64::from(x.round()), "y": f64::from(y.round())})),
    })
}

impl Host {
    /// Adds a node of kind `definition` (such as `org.orange.files/write-text`) at a
    /// place on the canvas, bound to this host's provider for it, with the default
    /// activation if it is a trigger. Returns the new node's ID. What the node still
    /// needs (a variable to name, a type to choose) shows in the draft's problems.
    pub fn add_node(
        &self,
        draft: &mut Draft,
        definition: &str,
        at: [f32; 2],
    ) -> Result<NodeId, HostError> {
        let version = self
            .nodes()
            .filter(|info| info.definition.definition.id.as_str() == definition)
            .map(|info| info.definition.definition.version)
            .max()
            .ok_or_else(|| HostError::Document(format!("no node kind `{definition}`")))?;
        let definition = DefinitionRef {
            id: definition
                .parse()
                .map_err(|_| HostError::Document(format!("no node kind `{definition}`")))?,
            version,
        };
        let node: NodeId = draft
            .fresh_id("node")
            .parse()
            .map_err(|_| refused(&"the new ID is not valid"))?;
        let registry = self.store.current();
        let found = registry
            .definition(&definition)
            .ok_or_else(|| HostError::Document(format!("no node kind `{}`", definition.id)))?;
        let entry = found.kind == NodeKind::Entry;
        let config = new_config(found, registry)?;
        let mut commands = if definition.id.is_core() {
            vec![DocCommand::AddNode {
                body: BodyRef::Root,
                node: NodeInstance {
                    id: node,
                    definition,
                    config_version: found.config_version,
                    config,
                    type_args: Default::default(),
                    type_pins: Default::default(),
                    provider: None::<ProviderBinding>,
                    signature: None,
                    inputs: Default::default(),
                    variadic: Default::default(),
                    activation: None,
                    extensions: Default::default(),
                },
            }]
        } else {
            let mut new = NewNode::new(node, definition.clone());
            new.config = config;
            let uses_connection = found
                .references
                .iter()
                .any(|field| field.kind == ReferenceKind::Connection);
            if uses_connection {
                let adapter = self
                    .presentations
                    .get(&definition)
                    .and_then(|shown| shown.connection_adapter.as_deref());
                let (connection, declared) = self.connection_for(draft, adapter)?;
                let _ = new
                    .config
                    .insert("connection".to_owned(), json!(connection));
                if let Some(decl) = declared {
                    // The node's contract needs its connection declared, so the node is
                    // built against the document with the declaration and both are one edit.
                    let mut with = draft.document().clone();
                    let _ = with
                        .apply([DocCommand::UpsertConnection(decl.clone())])
                        .map_err(|error| refused(&error))?;
                    let mut commands = vec![DocCommand::UpsertConnection(decl)];
                    commands.extend(
                        new_node_edit(&with, registry, new)
                            .map_err(|error| refused(&error))?
                            .commands,
                    );
                    commands
                } else {
                    new_node_edit(draft.document(), registry, new)
                        .map_err(|error| refused(&error))?
                        .commands
                }
            } else {
                new_node_edit(draft.document(), registry, new)
                    .map_err(|error| refused(&error))?
                    .commands
            }
        };
        // A variadic port starts with the fewest instances it may have, so a new fork
        // has its branch and a new sequence its two steps.
        if let Ports::Declared(ports) = &found.ports {
            for port in ports {
                let least = port.variadic.map_or(0, |bounds| bounds.min);
                for at in 0..least {
                    commands.push(DocCommand::InsertVariadic {
                        node,
                        port: port.port_id.clone(),
                        at: usize::from(at),
                    });
                }
            }
        }
        if entry {
            let policy: ActivationPolicy =
                serde_json::from_value(default_activation()).map_err(|error| refused(&error))?;
            commands.push(DocCommand::SetActivation {
                node,
                policy: Some(policy),
            });
        }
        commands.push(place_command(node, Some(at)));
        let _ = draft.commit(commands)?;
        Ok(node)
    }

    /// The connection a new node that uses one of kind `adapter` names: the first one
    /// of that kind the document declares, in name order, or a new graph connection with
    /// default settings when it declares none, returned to be declared with the node.
    /// A node whose definition does not say its kind takes the first connection.
    fn connection_for(
        &self,
        draft: &Draft,
        adapter: Option<&str>,
    ) -> Result<(String, Option<orange_model::document::ConnectionDecl>), HostError> {
        let existing = draft
            .document()
            .connections()
            .values()
            .filter(|decl| adapter.is_none_or(|kind| decl.adapter == kind))
            .min_by(|a, b| (&a.name, &a.id).cmp(&(&b.name, &b.id)));
        if let Some(decl) = existing {
            return Ok((decl.id.to_string(), None));
        }
        let Some(adapter) = adapter else {
            return Err(refused(
                &"the node needs a connection: declare one in the Document tab first",
            ));
        };
        if !self.adapters.contains_key(adapter) {
            return Err(refused(&format!(
                "the node needs a `{adapter}` connection, which this application cannot open"
            )));
        }
        let name = adapter.rsplit('/').next().unwrap_or(adapter).to_owned();
        let (id, decl) = self.connection_decl(
            draft,
            &Connection {
                id: None,
                name,
                adapter: adapter.to_owned(),
                scope: ConnectionScope::Graph,
                settings: json!({}),
            },
        )?;
        Ok((id, Some(decl)))
    }

    /// Removes nodes with the wires that touch them and their places.
    pub fn remove_nodes(&self, draft: &mut Draft, nodes: &[NodeId]) -> Result<bool, HostError> {
        let placed = draft.document().editor().get("nodes");
        let places: Vec<DocCommand> = nodes
            .iter()
            .filter(|node| placed.is_some_and(|places| places.get(node.to_string()).is_some()))
            .map(|node| place_command(*node, None))
            .collect();
        draft.commit(
            std::iter::once(DocCommand::RemoveNodes {
                ids: nodes.to_vec(),
            })
            .chain(places),
        )
    }

    /// Wires an output to an input. A value input takes one wire and a step output
    /// leads one way, so a wire already there is replaced. A wire the language does
    /// not allow (a value into a step, types that do not fit) is refused with the
    /// reasons as [`HostError::Invalid`], and the draft is unchanged.
    pub fn connect(
        &self,
        draft: &mut Draft,
        from: PortEnd<'_>,
        to: PortEnd<'_>,
    ) -> Result<bool, HostError> {
        let exec = |end: PortEnd<'_>, input: bool| {
            self.node_view(draft.document(), end.0)
                .and_then(|node| {
                    node.ports
                        .into_iter()
                        .find(|port| port.input == input && port.id == end.1)
                })
                .map(|port| port.exec)
                .ok_or_else(|| {
                    let side = if input { "input" } else { "output" };
                    HostError::Document(format!("the node has no {side} `{}`", end.1))
                })
        };
        let (source, target) = (exec(from, false)?, exec(to, true)?);
        if source != target {
            return Err(HostError::Document(
                "a step connects to a step and a value to a value".into(),
            ));
        }
        let (from_ref, to_ref) = (port_ref(from)?, port_ref(to)?);
        let mut commands: Vec<DocCommand> = draft
            .document()
            .root()
            .edges
            .values()
            .filter(|edge| {
                if source {
                    edge.from == from_ref
                } else {
                    edge.to == to_ref
                }
            })
            .map(|edge| DocCommand::Disconnect { edge: edge.id })
            .collect();
        let id = draft.fresh_id("edge");
        let edge: Edge = serde_json::from_value(json!({"id": id,
            "from": {"node": from.0.to_string(), "port": from.1},
            "to": {"node": to.0.to_string(), "port": to.1}}))
        .map_err(|error| refused(&error))?;
        let edge_id = edge.id;
        commands.push(DocCommand::Connect { edge });
        let Some(redo) = draft.try_commit(commands)? else {
            return Ok(false);
        };
        // Problems that name the new wire are the reason it cannot be made.
        let against: Vec<Diagnostic> = self
            .problems(draft)?
            .into_iter()
            .filter(|diagnostic| diagnostic.subject.edge == Some(edge_id))
            .collect();
        if against.is_empty() {
            Ok(true)
        } else {
            draft.withdraw(redo);
            Err(HostError::Invalid(against))
        }
    }

    /// Removes the wires at one port: those into an input, or those out of an output.
    pub fn disconnect(
        &self,
        draft: &mut Draft,
        at: PortEnd<'_>,
        input: bool,
    ) -> Result<bool, HostError> {
        let end = port_ref(at)?;
        let commands: Vec<DocCommand> = draft
            .document()
            .root()
            .edges
            .values()
            .filter(|edge| {
                if input {
                    edge.to == end
                } else {
                    edge.from == end
                }
            })
            .map(|edge| DocCommand::Disconnect { edge: edge.id })
            .collect();
        draft.commit(commands)
    }

    /// Replaces a node's settings (its `config`), such as the variable a variable
    /// node names or the interval of a timer.
    pub fn set_config(
        &self,
        draft: &mut Draft,
        node: NodeId,
        config: &Json,
    ) -> Result<bool, HostError> {
        let config_version = draft
            .document()
            .node(node)
            .map(|found| found.config_version)
            .ok_or_else(|| HostError::Document("the document has no such node".into()))?;
        let config = serde_json::from_value(config.clone()).map_err(|error| refused(&error))?;
        draft.commit([DocCommand::SetConfig {
            node,
            config_version,
            config,
        }])
    }

    /// Writes a value on an input, typed plainly (`7`, `0.5`, `true`, `some text`) for
    /// text, boolean, 64-bit integer and `f64` inputs, or as a full literal for any
    /// other type. `None` takes the value off the input.
    pub fn set_input(
        &self,
        draft: &mut Draft,
        at: PortEnd<'_>,
        typed: Option<&str>,
    ) -> Result<bool, HostError> {
        let port = port_ref(at)?;
        let literal = match typed {
            None => {
                // Taking off a value that is not there changes nothing.
                let written = draft
                    .document()
                    .node(at.0)
                    .is_some_and(|node| node.inputs.contains_key(&port.port));
                if !written {
                    return Ok(false);
                }
                None
            }
            Some(typed) => {
                let ty = self
                    .node_view(draft.document(), at.0)
                    .and_then(|node| {
                        node.ports
                            .into_iter()
                            .find(|port| port.input && !port.exec && port.id == at.1)
                    })
                    .and_then(|port| port.ty)
                    .ok_or_else(|| {
                        HostError::Document(format!("the node has no value input `{}`", at.1))
                    })?;
                let text = literal(&ty, typed).map_err(HostError::Variable)?;
                let parsed: Literal = serde_json::from_str(&text)
                    .map_err(|error| HostError::Variable(error.to_string()))?;
                // A literal that does not decode is not a value of the input's type.
                let _ = self.value_of(&parsed)?;
                Some(parsed)
            }
        };
        draft.commit([DocCommand::SetLiteral { port, literal }])
    }
}
