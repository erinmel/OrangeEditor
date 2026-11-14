//! Editing a document: adding and removing nodes, wiring ports and writing values on
//! inputs. Each operation takes the document's text and returns the edited text, so a
//! front end keeps nothing but the text and can undo by keeping the previous one.
//!
//! Every change goes through the model's own transactions. None of them runs
//! anything; a flow of the document is changed only when the front end hands the new
//! text to [`Host::update`].
use super::*;
use orange_model::DocCommand;
use orange_model::document::PortRef;
use serde_json::{Value as Json, json};

/// One end of a wire: a node and the name of one of its ports.
pub type PortEnd<'a> = (NodeId, &'a str);

fn refused(error: &dyn std::fmt::Display) -> HostError {
    HostError::Document(error.to_string())
}

/// An ID no node or edge of `saved` has yet, as UUID version 4 text. It is derived
/// from the document and a counter, so the same edit of the same text gives the same
/// ID: edits can be repeated in tests and undone and redone to identical documents.
pub(crate) fn fresh_id(saved: &Json, what: &str) -> String {
    let text = saved.to_string();
    let mut attempt = 0u32;
    loop {
        let mut hasher = Sha256::new();
        hasher.update(what.as_bytes());
        hasher.update(attempt.to_le_bytes());
        hasher.update(text.as_bytes());
        let digest = hasher.finalize();
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(digest.get(..16).unwrap_or(&[0; 16]));
        // Version 4, variant 1: the form every ID of a document has.
        bytes[6] = (bytes[6] & 0x0f) | 0x40;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        let hex = hex(&bytes);
        let id = format!(
            "{}-{}-{}-{}-{}",
            &hex[..8],
            &hex[8..12],
            &hex[12..16],
            &hex[16..20],
            &hex[20..]
        );
        if !text.contains(&id) {
            return id;
        }
        attempt += 1;
    }
}

/// The text of a new, empty document with an ID of its own.
pub fn new_document() -> Result<String, HostError> {
    // The time and the process tell one new document from another.
    let moment = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    let seed = json!({"moment": moment.to_string(), "process": std::process::id()});
    let id = fresh_id(&seed, "graph");
    let graph = id
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

impl Host {
    /// Adds a node of kind `definition` (such as `org.orange.files/write-text`) at a
    /// place on the canvas, bound to this host's provider for it, with the default
    /// activation if it is a trigger. Returns the document and the new node's ID. What
    /// the node still needs (a variable to name, a type to choose) shows as problems
    /// when the document is checked.
    pub fn add_node(
        &self,
        json: &str,
        definition: &str,
        at: [f32; 2],
    ) -> Result<(String, NodeId), HostError> {
        let version = self
            .nodes()
            .filter(|info| info.definition.definition.id.as_str() == definition)
            .map(|info| info.definition.definition.version)
            .max()
            .ok_or_else(|| HostError::Document(format!("no node kind `{definition}`")))?;
        let mut saved: Json = serde_json::from_str(json).map_err(|error| refused(&error))?;
        let id = fresh_id(&saved, "node");
        let node = json!({"id": id, "definition_id": definition,
            "definition_version": version, "config_version": 1, "config": {}});
        saved["nodes"]
            .as_array_mut()
            .ok_or_else(|| HostError::Document("the document has no node list".into()))?
            .push(node);
        let bound = self.bind(&saved.to_string())?;
        let node: NodeId = id
            .parse()
            .map_err(|_| refused(&"the new ID is not valid"))?;
        Ok((place(&bound, &[(node, at)])?, node))
    }

    /// Removes nodes with the wires that touch them.
    pub fn remove_nodes(&self, json: &str, nodes: &[NodeId]) -> Result<String, HostError> {
        let mut document = from_json(json).map_err(|error| refused(&error))?;
        let _ = document
            .apply([DocCommand::RemoveNodes {
                ids: nodes.to_vec(),
            }])
            .map_err(|error| refused(&error))?;
        // A removed node's place is of no use to anyone.
        let mut saved: Json =
            serde_json::from_str(&to_json(&document).map_err(|error| refused(&error))?)
                .map_err(|error| refused(&error))?;
        if let Some(places) = saved["editor"]["nodes"].as_object_mut() {
            for node in nodes {
                let _ = places.remove(&node.to_string());
            }
        }
        let document = from_json(&saved.to_string()).map_err(|error| refused(&error))?;
        to_json(&document).map_err(|error| refused(&error))
    }

    /// Wires an output to an input. A value input takes one wire and a step output
    /// leads one way, so a wire already there is replaced. A wire the language does
    /// not allow (a value into a step, types that do not fit) is refused with the
    /// reasons as [`HostError::Invalid`], and the document is unchanged.
    pub fn connect(
        &self,
        json: &str,
        from: PortEnd<'_>,
        to: PortEnd<'_>,
    ) -> Result<String, HostError> {
        let view = self.view(json)?;
        let port = |end: PortEnd<'_>, input: bool| {
            view.nodes
                .iter()
                .find(|node| node.id == end.0)
                .and_then(|node| {
                    node.ports
                        .iter()
                        .find(|port| port.input == input && port.id == end.1)
                })
                .ok_or_else(|| {
                    let side = if input { "input" } else { "output" };
                    HostError::Document(format!("the node has no {side} `{}`", end.1))
                })
        };
        let (source, target) = (port(from, false)?, port(to, true)?);
        if source.exec != target.exec {
            return Err(HostError::Document(
                "a step connects to a step and a value to a value".into(),
            ));
        }
        let mut document = from_json(json).map_err(|error| refused(&error))?;
        let (from_ref, to_ref) = (port_ref(from)?, port_ref(to)?);
        let mut commands: Vec<DocCommand> = document
            .root()
            .edges
            .values()
            .filter(|edge| {
                if source.exec {
                    edge.from == from_ref
                } else {
                    edge.to == to_ref
                }
            })
            .map(|edge| DocCommand::Disconnect { edge: edge.id })
            .collect();
        let saved: Json = serde_json::from_str(json).map_err(|error| refused(&error))?;
        let id = fresh_id(&saved, "edge");
        let edge = serde_json::from_value(json!({"id": id,
            "from": {"node": from.0.to_string(), "port": from.1},
            "to": {"node": to.0.to_string(), "port": to.1}}))
        .map_err(|error| refused(&error))?;
        commands.push(DocCommand::Connect { edge });
        let _ = document.apply(commands).map_err(|error| refused(&error))?;
        let text = to_json(&document).map_err(|error| refused(&error))?;
        // Problems that name the new wire are the reason it cannot be made.
        let against: Vec<Diagnostic> = self
            .check(&text)?
            .into_iter()
            .filter(|diagnostic| {
                diagnostic
                    .subject
                    .edge
                    .is_some_and(|edge| edge.to_string() == id)
            })
            .collect();
        if against.is_empty() {
            Ok(text)
        } else {
            Err(HostError::Invalid(against))
        }
    }

    /// Removes the wires at one port: those into an input, or those out of an output.
    pub fn disconnect(
        &self,
        json: &str,
        at: PortEnd<'_>,
        input: bool,
    ) -> Result<String, HostError> {
        let mut document = from_json(json).map_err(|error| refused(&error))?;
        let end = port_ref(at)?;
        let commands: Vec<DocCommand> = document
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
        if commands.is_empty() {
            return Ok(json.to_owned());
        }
        let _ = document.apply(commands).map_err(|error| refused(&error))?;
        to_json(&document).map_err(|error| refused(&error))
    }

    /// Replaces a node's settings (its `config`), such as the variable a variable
    /// node names or the interval of a timer.
    pub fn set_config(&self, json: &str, node: NodeId, config: &Json) -> Result<String, HostError> {
        let mut document = from_json(json).map_err(|error| refused(&error))?;
        let config_version = document
            .node(node)
            .map(|found| found.config_version)
            .ok_or_else(|| HostError::Document("the document has no such node".into()))?;
        let config = serde_json::from_value(config.clone()).map_err(|error| refused(&error))?;
        match document.apply([DocCommand::SetConfig {
            node,
            config_version,
            config,
        }]) {
            Ok(_) => to_json(&document).map_err(|error| refused(&error)),
            Err(orange_model::TxError::Empty) => Ok(json.to_owned()),
            Err(error) => Err(refused(&error)),
        }
    }

    /// Writes a value on an input, typed plainly (`7`, `0.5`, `true`, `some text`) for
    /// text, boolean, 64-bit integer and `f64` inputs, or as a full literal for any
    /// other type. `None` takes the value off the input.
    pub fn set_input(
        &self,
        json: &str,
        at: PortEnd<'_>,
        typed: Option<&str>,
    ) -> Result<String, HostError> {
        let literal_of = |typed: &str| -> Result<Literal, HostError> {
            let view = self.view(json)?;
            let ty = view
                .nodes
                .iter()
                .find(|node| node.id == at.0)
                .and_then(|node| {
                    node.ports
                        .iter()
                        .find(|port| port.input && !port.exec && port.id == at.1)
                })
                .and_then(|port| port.ty.clone())
                .ok_or_else(|| {
                    HostError::Document(format!("the node has no value input `{}`", at.1))
                })?;
            let text = literal(&ty, typed).map_err(HostError::Variable)?;
            let parsed: Literal = serde_json::from_str(&text)
                .map_err(|error| HostError::Variable(error.to_string()))?;
            // A literal that does not decode is not a value of the input's type.
            let _ = self.value_of(&parsed)?;
            Ok(parsed)
        };
        let literal = typed.map(literal_of).transpose()?;
        let mut document = from_json(json).map_err(|error| refused(&error))?;
        let port = port_ref(at)?;
        // Taking off a value that is not there changes nothing, the revision included.
        let written = document
            .node(at.0)
            .is_some_and(|node| node.inputs.contains_key(&port.port));
        if literal.is_none() && !written {
            return Ok(json.to_owned());
        }
        match document.apply([DocCommand::SetLiteral { port, literal }]) {
            Ok(_) => to_json(&document).map_err(|error| refused(&error)),
            // Taking off a value that is not there changes nothing.
            Err(orange_model::TxError::Empty) => Ok(json.to_owned()),
            Err(error) => Err(refused(&error)),
        }
    }
}
