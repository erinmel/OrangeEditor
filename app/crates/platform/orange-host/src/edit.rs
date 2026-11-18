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
    if found.definition.id.as_str() == "core/on-watch" {
        let _ = config.insert("mode".to_owned(), json!("changed"));
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
    /// Add a frequency range and its stable output ports as one edit.
    pub fn add_audio_band(
        &self,
        draft: &mut Draft,
        node: NodeId,
        range: [u32; 2],
    ) -> Result<bool, HostError> {
        self.edit_audio_band(
            draft,
            node,
            None,
            Some(json!({
                "low_hz": range[0], "high_hz": range[1],
            })),
        )
    }

    /// Create a filtered stream on this card, upgrading an older spectrum in the
    /// same undo step. Legacy numeric and execution outputs retain their keys.
    pub fn add_filtered_audio_band(
        &self,
        draft: &mut Draft,
        node: NodeId,
        range: [u32; 2],
    ) -> Result<bool, HostError> {
        let instance = draft
            .document()
            .root()
            .nodes
            .get(&node)
            .filter(|node| {
                matches!(
                    node.definition.id.as_str(),
                    "org.orange.audio/spectrum" | "org.orange.audio/multiband-spectrum"
                )
            })
            .ok_or_else(|| refused(&"select a Frequency Splitter"))?;
        if instance.definition.version >= 2 {
            return self.add_audio_band(draft, node, range);
        }

        let registry = self.registry_for(draft.document());
        let definition = DefinitionRef {
            id: "org.orange.audio/multiband-spectrum"
                .parse()
                .map_err(|error| refused(&error))?,
            version: 3,
        };
        let mut new = NewNode::new(node, definition);
        new.inputs.clone_from(&instance.inputs);
        new.config.clone_from(&instance.config);
        if let Some(bands) = new.config.get_mut("bands").and_then(Json::as_object_mut) {
            for band in bands.values_mut() {
                let band = band
                    .as_object_mut()
                    .ok_or_else(|| refused(&"invalid spectrum band"))?;
                band.insert("legacy_level".to_owned(), true.into());
            }
        }
        let mut replacement = new_node_edit(draft.document(), &registry, new)
            .map_err(|error| refused(&error))?
            .commands;
        for command in &mut replacement {
            if let DocCommand::AddNode { node: next, .. } = command {
                next.extensions.clone_from(&instance.extensions);
                next.variadic.clone_from(&instance.variadic);
                if let Some(levels) = next
                    .variadic
                    .get(&orange_model::port_id!("band_level"))
                    .cloned()
                {
                    next.variadic
                        .insert(orange_model::port_id!("band_stream"), levels);
                }
                next.activation = match &instance.activation {
                    Some(policy) => Some(policy.clone()),
                    None => Some(
                        serde_json::from_value(default_activation())
                            .map_err(|error| refused(&error))?,
                    ),
                };
            }
        }
        let mut commands = vec![DocCommand::RemoveNodes { ids: vec![node] }];
        commands.extend(replacement);
        commands.extend(
            draft
                .document()
                .root()
                .edges
                .values()
                .filter(|edge| edge.from.node == node || edge.to.node == node)
                .cloned()
                .map(|edge| DocCommand::Connect { edge }),
        );
        let staged = self.staged_node(draft, &commands)?;
        commands.extend(self.audio_band_commands(
            &staged,
            node,
            None,
            Some(json!({
                "low_hz": range[0], "high_hz": range[1],
            })),
        )?);
        draft.commit(&registry, commands)
    }

    /// Replace a band's settings, or remove the band and its wires when settings are absent.
    pub fn change_audio_band(
        &self,
        draft: &mut Draft,
        node: NodeId,
        band: u16,
        settings: Option<Json>,
    ) -> Result<bool, HostError> {
        self.edit_audio_band(draft, node, Some(band), settings)
    }

    /// Moves a band and its enabled outputs in display order without changing any wire.
    pub fn move_audio_band(
        &self,
        draft: &mut Draft,
        node: NodeId,
        band: u16,
        at: usize,
    ) -> Result<bool, HostError> {
        let instance = draft
            .document()
            .node(node)
            .filter(|node| node.definition.id.as_str() == "org.orange.audio/multiband-spectrum")
            .ok_or_else(|| refused(&"select a Frequency Splitter"))?;
        let level = spectrum_band_port(instance);
        let state = instance
            .variadic
            .get(&level)
            .ok_or_else(|| refused(&"no bands"))?;
        let before = state
            .order
            .iter()
            .position(|&id| id == band)
            .ok_or_else(|| refused(&"this spectrum has no such band"))?;
        if at >= state.order.len() {
            return Err(refused(&"band position is out of range"));
        }
        let mut order = state.order.clone();
        order.remove(before);
        order.insert(at, band);
        let mut commands = vec![DocCommand::MoveVariadic {
            node,
            port: PortKey::new(level, Some(band)),
            at,
        }];
        let bands = instance
            .config
            .get("bands")
            .ok_or_else(|| refused(&"no band settings"))?;
        let legacy = |band: u16| bands[band.to_string()]["legacy_level"].as_bool() == Some(true);
        if legacy(band) {
            commands.push(DocCommand::MoveVariadic {
                node,
                port: PortKey::new(orange_model::port_id!("band_level"), Some(band)),
                at: order[..at].iter().filter(|&&band| legacy(band)).count(),
            });
        }
        for (field, port) in [
            ("above_port", orange_model::port_id!("band_above")),
            ("below_port", orange_model::port_id!("band_below")),
        ] {
            let route = |band: u16| {
                bands[band.to_string()][field]
                    .as_u64()
                    .and_then(|n| u16::try_from(n).ok())
            };
            if let Some(id) = route(band) {
                commands.push(DocCommand::MoveVariadic {
                    node,
                    port: PortKey::new(port, Some(id)),
                    at: order[..at]
                        .iter()
                        .filter(|&&band| route(band).is_some())
                        .count(),
                });
            }
        }
        draft.commit(&self.registry_for(draft.document()), commands)
    }

    fn edit_audio_band(
        &self,
        draft: &mut Draft,
        node: NodeId,
        band: Option<u16>,
        settings: Option<Json>,
    ) -> Result<bool, HostError> {
        let commands = self.audio_band_commands(draft, node, band, settings)?;
        draft.commit(&self.registry_for(draft.document()), commands)
    }

    fn audio_band_commands(
        &self,
        draft: &Draft,
        node: NodeId,
        band: Option<u16>,
        settings: Option<Json>,
    ) -> Result<Vec<DocCommand>, HostError> {
        let instance = draft
            .document()
            .node(node)
            .filter(|node| node.definition.id.as_str() == "org.orange.audio/multiband-spectrum")
            .ok_or_else(|| refused(&"select a Frequency Splitter"))?;
        let mut config = instance.config.clone();
        let bands = config
            .entry("bands".to_owned())
            .or_insert_with(|| json!({}));
        let bands = bands
            .as_object_mut()
            .ok_or_else(|| refused(&"invalid spectrum bands"))?;
        let level = spectrum_band_port(instance);
        let ids = instance.variadic.get(&level);
        let id = band.unwrap_or_else(|| ids.map_or(0, |ids| ids.next));
        let name = id.to_string();
        let saved = bands.get(&name);
        let legacy = saved.is_some_and(|band| band["legacy_level"].as_bool() == Some(true));
        let has_triggers = level.as_str() == "band_level" || legacy;
        if band.is_some() && saved.is_none_or(|_| !ids.is_some_and(|ids| ids.order.contains(&id))) {
            return Err(refused(&"this spectrum has no such band"));
        }
        let mut commands = Vec::new();
        if band.is_none() {
            let limit = self
                .node_view(draft.document(), node)
                .and_then(|view| {
                    view.variadic
                        .iter()
                        .find(|port| port.port == level.as_str())?
                        .max
                })
                .ok_or_else(|| refused(&"this spectrum has no band limit"))?;
            if bands.len() >= limit {
                return Err(refused(&format!("a spectrum supports up to {limit} bands")));
            }
            commands.push(DocCommand::InsertVariadic {
                node,
                port: level,
                at: ids.map_or(0, |ids| ids.order.len()),
            });
        } else if settings.is_none() {
            commands.push(DocCommand::RemoveVariadic {
                node,
                port: PortKey::new(level, Some(id)),
            });
            if legacy {
                commands.push(DocCommand::RemoveVariadic {
                    node,
                    port: PortKey::new(orange_model::port_id!("band_level"), Some(id)),
                });
            }
        }

        let mut settings = settings;
        if let Some(band) = &mut settings {
            let [low, high] = ["low_hz", "high_hz"].map(|field| band[field].as_f64());
            if !matches!((low, high), (Some(low), Some(high)) if low >= 20.0 && high <= 24_000.0 && low < high)
            {
                return Err(refused(&"use two ordered frequencies from 20 to 24000 Hz"));
            }
            if !band["name"]
                .as_str()
                .is_some_and(|name| !name.trim().is_empty())
            {
                band["name"] = format!("Band {}", u32::from(id) + 1).into();
            }
            if !matches!(
                band["mode"].as_str(),
                None | Some("above" | "below" | "both")
            ) {
                return Err(refused(&"choose Above, Below, or Both"));
            }
            if legacy {
                band["legacy_level"] = true.into();
            } else if let Some(fields) = band.as_object_mut() {
                fields.remove("legacy_level");
            }
        }
        for (direction, field, port) in [
            ("above", "above_port", orange_model::port_id!("band_above")),
            ("below", "below_port", orange_model::port_id!("band_below")),
        ] {
            if !has_triggers {
                break;
            }
            let old = saved
                .and_then(|band| band[field].as_u64())
                .and_then(|n| u16::try_from(n).ok());
            let enabled = settings.as_ref().is_some_and(|band| {
                matches!(band["mode"].as_str().unwrap_or("above"), "both")
                    || band["mode"].as_str().unwrap_or("above") == direction
            });
            let next = match (old, enabled) {
                (Some(port_id), false) => {
                    commands.push(DocCommand::RemoveVariadic {
                        node,
                        port: PortKey::new(port, Some(port_id)),
                    });
                    None
                }
                (None, true) => {
                    let state = instance.variadic.get(&port);
                    let id = state.map_or(0, |state| state.next);
                    commands.push(DocCommand::InsertVariadic {
                        node,
                        port,
                        at: state.map_or(0, |state| state.order.len()),
                    });
                    Some(id)
                }
                (old, _) => old,
            };
            if let Some(band) = settings.as_mut().and_then(Json::as_object_mut) {
                band.remove(field);
                if let Some(next) = next {
                    band.insert(field.to_owned(), next.into());
                }
            }
        }
        if let Some(settings) = settings {
            bands.insert(name, settings);
        } else {
            bands.remove(&name);
        }
        commands.push(DocCommand::SetConfig {
            node,
            config_version: instance.config_version,
            config,
        });
        Ok(commands)
    }

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
        let _ = draft.commit(
            &self.registry_for(draft.document()),
            [DocCommand::InsertVariadic {
                node,
                port: id.clone(),
                at: found.count,
            }],
        )?;
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
        draft.commit(
            &self.registry_for(draft.document()),
            [DocCommand::RemoveVariadic {
                node: at.0,
                port: key,
            }],
        )
    }
}

fn spectrum_band_port(node: &NodeInstance) -> PortId {
    if node.definition.version >= 2 {
        orange_model::port_id!("band_stream")
    } else {
        orange_model::port_id!("band_level")
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
        let (node, commands) = self.new_node_commands(draft, definition, at)?;
        let _ = draft.commit(&self.registry_for(draft.document()), commands)?;
        Ok(node)
    }

    /// Adds a Band level read wired into When, as one undoable edit.
    pub fn add_band_trigger(&self, draft: &mut Draft, at: [f32; 2]) -> Result<NodeId, HostError> {
        let (level, mut commands) =
            self.new_node_commands(draft, "org.orange.audio/live-band-level", at)?;
        let (when, entry) = self.node_commands(
            draft,
            "core/on-watch",
            [at[0] + 360.0, at[1]],
            "node:band-trigger",
        )?;
        commands.extend(entry);
        commands.push(DocCommand::SetConfig {
            node: when,
            config_version: 2,
            config: serde_json::from_value(json!({
                "mode": "above", "threshold": -20, "release": -26, "hold": 50
            }))
            .map_err(|error| refused(&error))?,
        });
        let wire = serde_json::from_value(json!({
            "id": draft.fresh_id("edge"),
            "from": {"node": level, "port": "level"},
            "to": {"node": when, "port": "watch"}
        }))
        .map_err(|error| refused(&error))?;
        commands.push(DocCommand::Connect { edge: wire });
        let _ = draft.commit(&self.registry_for(draft.document()), commands)?;
        Ok(when)
    }

    /// Adds a meter or threshold entry at a spectrum selection, sharing its source.
    /// The new card, its frequency limits and its wire form one undoable edit.
    pub fn add_spectrum_band(
        &self,
        draft: &mut Draft,
        spectrum: NodeId,
        range: [u32; 2],
        trigger: bool,
        at: [f32; 2],
    ) -> Result<NodeId, HostError> {
        let [low, high] = range;
        if !(20..high).contains(&low) || high > 24_000 {
            return Err(refused(&"choose a valid frequency range"));
        }
        if !draft
            .document()
            .node(spectrum)
            .is_some_and(|node| node.definition.id.as_str() == "org.orange.audio/spectrum")
        {
            return Err(refused(&"select a range on an Audio spectrum node"));
        }
        let destination = port_ref((spectrum, "stream"))?;
        let from = draft
            .document()
            .root()
            .edges
            .values()
            .find(|edge| edge.to == destination)
            .map(|edge| edge.from.clone())
            .ok_or_else(|| refused(&"connect an audio source to the spectrum first"))?;
        let definition = if trigger {
            "org.orange.audio/band-trigger"
        } else {
            "org.orange.audio/live-band-level"
        };
        let (node, mut commands) = self.new_node_commands(draft, definition, at)?;
        for (port, hz) in [("low_hz", low), ("high_hz", high)] {
            let text = literal("core/f64@1", &hz.to_string()).map_err(HostError::Variable)?;
            commands.push(DocCommand::SetLiteral {
                port: port_ref((node, port))?,
                literal: Some(serde_json::from_str(&text).map_err(|error| refused(&error))?),
            });
        }
        let edge = serde_json::from_value(json!({
            "id": draft.fresh_id("edge"), "from": from, "to": {"node": node, "port": "stream"}
        }))
        .map_err(|error| refused(&error))?;
        commands.push(DocCommand::Connect { edge });
        let _ = draft.commit(&self.registry_for(draft.document()), commands)?;
        Ok(node)
    }

    fn new_node_commands(
        &self,
        draft: &Draft,
        definition: &str,
        at: [f32; 2],
    ) -> Result<(NodeId, Vec<DocCommand>), HostError> {
        self.node_commands(draft, definition, at, "node")
    }

    fn node_commands(
        &self,
        draft: &Draft,
        definition: &str,
        at: [f32; 2],
        identity: &str,
    ) -> Result<(NodeId, Vec<DocCommand>), HostError> {
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
            .fresh_id(identity)
            .parse()
            .map_err(|_| refused(&"the new ID is not valid"))?;
        let registry = &self.registry_for(draft.document());
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
        Ok((node, commands))
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
            &self.registry_for(draft.document()),
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
        let Some(redo) = draft.try_commit(&self.registry_for(draft.document()), commands)? else {
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
        draft.commit(&self.registry_for(draft.document()), commands)
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
            .map(|found| {
                if found.definition.id.as_str() == "core/on-watch" && found.config_version == 1 {
                    2
                } else {
                    found.config_version
                }
            })
            .ok_or_else(|| HostError::Document("the document has no such node".into()))?;
        let config = serde_json::from_value(config.clone()).map_err(|error| refused(&error))?;
        draft.commit(
            &self.registry_for(draft.document()),
            [DocCommand::SetConfig {
                node,
                config_version,
                config,
            }],
        )
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
                let _ = self.value_in(draft.document(), &parsed)?;
                Some(parsed)
            }
        };
        draft.commit(
            &self.registry_for(draft.document()),
            [DocCommand::SetLiteral { port, literal }],
        )
    }
}

impl Host {
    /// Removes the one wire from `from` to `to`. Returns whether there was one.
    pub fn disconnect_wire(
        &self,
        draft: &mut Draft,
        from: PortEnd<'_>,
        to: PortEnd<'_>,
    ) -> Result<bool, HostError> {
        let (from, to) = (port_ref(from)?, port_ref(to)?);
        let edges: Vec<DocCommand> = draft
            .document()
            .root()
            .edges
            .values()
            .filter(|edge| edge.from == from && edge.to == to)
            .map(|edge| DocCommand::Disconnect { edge: edge.id })
            .collect();
        draft.commit(&self.registry_for(draft.document()), edges)
    }

    /// Puts `node` into the wire from `from` to `to`, as one edit: the wire is replaced
    /// by one into the node's first input of the same kind and type, and one out of its
    /// first such output. Refused, with nothing changed, when the node has no fitting
    /// ports or the language does not allow the new wires.
    pub fn insert_into_wire(
        &self,
        draft: &mut Draft,
        node: NodeId,
        from: PortEnd<'_>,
        to: PortEnd<'_>,
    ) -> Result<bool, HostError> {
        let (commands, made) = self.splice_commands(draft, node, from, to)?;
        let Some(redo) = draft.try_commit(&self.registry_for(draft.document()), commands)? else {
            return Ok(false);
        };
        match self.check_splice(draft, &made) {
            Ok(()) => Ok(true),
            Err(error) => {
                draft.withdraw(redo);
                Err(error)
            }
        }
    }

    /// Returns the configured card for a node kind without editing the document.
    pub fn placement_preview(
        &self,
        draft: &Draft,
        definition: &str,
    ) -> Result<NodeView, HostError> {
        let (node, commands) = self.new_node_commands(draft, definition, [0.0, 0.0])?;
        let mut staged = draft.document().clone();
        let _ = staged.apply(commands).map_err(|error| refused(&error))?;
        self.node_view(&staged, node)
            .ok_or_else(|| refused(&"the new node could not be previewed"))
    }

    /// Creates a node and replaces a wire with its two links as one undoable edit.
    pub fn add_node_into_wire(
        &self,
        draft: &mut Draft,
        definition: &str,
        at: [f32; 2],
        from: PortEnd<'_>,
        to: PortEnd<'_>,
    ) -> Result<NodeId, HostError> {
        let (node, mut commands) = self.new_node_commands(draft, definition, at)?;
        let mut staged = self.staged_node(draft, &commands)?;
        let (splice, made) = self.splice_commands(&staged, node, from, to)?;
        let _ = staged.commit(
            &self.registry_for(staged.document()),
            splice.iter().cloned(),
        )?;
        self.check_splice(&mut staged, &made)?;
        commands.extend(splice);
        let _ = draft.commit(&self.registry_for(draft.document()), commands)?;
        Ok(node)
    }

    fn staged_node(&self, draft: &Draft, commands: &[DocCommand]) -> Result<Draft, HostError> {
        let mut staged = Draft::new(draft.document().clone());
        let _ = staged.commit(
            &self.registry_for(staged.document()),
            commands.iter().cloned(),
        )?;
        Ok(staged)
    }

    fn splice_commands(
        &self,
        draft: &Draft,
        node: NodeId,
        from: PortEnd<'_>,
        to: PortEnd<'_>,
    ) -> Result<(Vec<DocCommand>, Vec<orange_model::id::EdgeId>), HostError> {
        let ends = |id: NodeId| self.node_view(draft.document(), id);
        let source = ends(from.0)
            .and_then(|view| view.ports.into_iter().find(|p| !p.input && p.id == from.1))
            .ok_or_else(|| refused(&"the wire's output is not there"))?;
        let target = ends(to.0)
            .and_then(|view| view.ports.into_iter().find(|p| p.input && p.id == to.1))
            .ok_or_else(|| refused(&"the wire's input is not there"))?;
        let middle = ends(node).ok_or_else(|| refused(&"the document has no such node"))?;
        let fits = |output: &PortView, input: &PortView| {
            output.exec == input.exec
                && (output.exec
                    || input.ty.as_deref().is_none_or(|ty| {
                        output.ty.as_deref().is_none_or(|from| from == ty)
                            || ty.contains('$')
                            || from_open(output.ty.as_deref())
                    }))
        };
        let into = middle
            .ports
            .iter()
            .find(|port| port.input && fits(&source, port))
            .ok_or_else(|| refused(&"the node has no input this wire fits"))?;
        let out = middle
            .ports
            .iter()
            .find(|port| !port.input && fits(port, &target))
            .ok_or_else(|| refused(&"the node has no output to go on with"))?;
        let (from_ref, to_ref) = (port_ref(from)?, port_ref(to)?);
        let mut commands: Vec<DocCommand> = draft
            .document()
            .root()
            .edges
            .values()
            .filter(|edge| edge.from == from_ref && edge.to == to_ref)
            .map(|edge| DocCommand::Disconnect { edge: edge.id })
            .collect();
        if commands.is_empty() {
            return Err(refused(&"there is no such wire"));
        }
        let mut made = Vec::new();
        for (n, (a, b)) in [
            ((from.0, from.1), (node, into.id.as_str())),
            ((node, out.id.as_str()), (to.0, to.1)),
        ]
        .into_iter()
        .enumerate()
        {
            let id = draft.fresh_id(&format!("insert {n}"));
            let edge: Edge = serde_json::from_value(json!({"id": id,
                "from": {"node": a.0.to_string(), "port": a.1},
                "to": {"node": b.0.to_string(), "port": b.1}}))
            .map_err(|error| refused(&error))?;
            made.push(edge.id);
            commands.push(DocCommand::Connect { edge });
        }
        Ok((commands, made))
    }

    fn check_splice(
        &self,
        draft: &mut Draft,
        made: &[orange_model::id::EdgeId],
    ) -> Result<(), HostError> {
        let against: Vec<Diagnostic> = self
            .problems(draft)?
            .into_iter()
            .filter(|diagnostic| {
                diagnostic
                    .subject
                    .edge
                    .is_some_and(|edge| made.contains(&edge))
            })
            .collect();
        if against.is_empty() {
            Ok(())
        } else {
            Err(HostError::Invalid(against))
        }
    }
}

/// Whether a type is still open (a parameter), so it may become anything.
fn from_open(ty: Option<&str>) -> bool {
    ty.is_some_and(|ty| ty.contains('$'))
}
