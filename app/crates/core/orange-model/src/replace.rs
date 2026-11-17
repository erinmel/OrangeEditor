//! Replaced definitions (04 "Definition evolution", 12 §6.4).
//!
//! A package may stop shipping a definition version and say which one takes its place, with
//! the ports and config fields that were renamed. When a document is opened, every node
//! whose definition the registry lacks and that a [`Replacements`] table names is rebuilt as
//! a node of the replacing definition: same ID, body, place, literals, type pins, variadic
//! instances and activation, with its wires reconnected to the renamed ports. A node whose
//! definition is still registered is never moved: an installed old version keeps working
//! until its package is removed, and no newer version is chosen behind the user's back.
//!
//! The rewrite is one transaction, so it either applies to every replaced node or to none.

use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;

use crate::binding::{NewNode, NewNodeError, new_node_edit};
use crate::catalog::RegistrySnapshot;
use crate::document::{BodyRef, ConfigValue, DefinitionRef, Edge, GraphDocument, NodeInstance};
use crate::edit::DocCommand;
use crate::error::TxError;
use crate::id::{NodeId, PortId, PortKey};

/// One definition version that takes the place of another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replacement {
    pub from: DefinitionRef,
    pub to: DefinitionRef,
    /// Ports of `from` whose ID changed, old to new. Unlisted ports keep their ID.
    pub ports: BTreeMap<PortId, PortId>,
    /// Top-level config fields of `from` whose name changed, old to new. Unlisted fields
    /// keep their name.
    pub config: BTreeMap<String, String>,
}

/// Why a replacement was not added to a table.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ReplacementError {
    #[error("definition `{}@{}` replaces itself", .0.id, .0.version)]
    SelfReplacement(DefinitionRef),
    /// Two packages, or two files of one, say different things replace one definition.
    #[error("definition `{}@{}` has two different replacements", .0.id, .0.version)]
    Conflict(DefinitionRef),
}

/// The replacements an application knows, by the definition version they replace.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Replacements {
    by_from: BTreeMap<DefinitionRef, Replacement>,
}

impl Replacements {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds `replacement`. Adding an identical one again changes nothing.
    pub fn add(&mut self, replacement: Replacement) -> Result<(), ReplacementError> {
        if replacement.from == replacement.to {
            return Err(ReplacementError::SelfReplacement(replacement.from));
        }
        match self.by_from.get(&replacement.from) {
            Some(known) if *known == replacement => Ok(()),
            Some(_) => Err(ReplacementError::Conflict(replacement.from)),
            None => {
                let _ = self.by_from.insert(replacement.from.clone(), replacement);
                Ok(())
            }
        }
    }

    pub fn get(&self, from: &DefinitionRef) -> Option<&Replacement> {
        self.by_from.get(from)
    }

    pub fn is_empty(&self) -> bool {
        self.by_from.is_empty()
    }

    /// The registered definition `from` ends at, following replacements while the
    /// registry lacks each step, with the renames of every step composed. `None` when
    /// `from` is registered, nothing replaces it, the chain loops, or it ends at a
    /// definition the registry lacks.
    fn resolve(&self, from: &DefinitionRef, registry: &RegistrySnapshot) -> Option<Resolved<'_>> {
        if registry.definition(from).is_some() {
            return None;
        }
        let mut resolved = Resolved {
            to: from.clone(),
            ports: Renames(Vec::new()),
            config: Renames(Vec::new()),
        };
        let mut seen = BTreeSet::new();
        while registry.definition(&resolved.to).is_none() {
            if !seen.insert(resolved.to.clone()) {
                return None;
            }
            let step = self.by_from.get(&resolved.to)?;
            resolved.to = step.to.clone();
            resolved.ports.push(&step.ports);
            resolved.config.push(&step.config);
        }
        Some(resolved)
    }
}

/// Renames composed along a replacement chain.
struct Resolved<'a> {
    to: DefinitionRef,
    ports: Renames<'a, PortId>,
    config: Renames<'a, String>,
}

/// Rename steps along a chain; a name follows every step in order.
struct Renames<'a, K>(Vec<&'a BTreeMap<K, K>>);

impl<'a, K: Ord + Clone> Renames<'a, K> {
    fn push(&mut self, step: &'a BTreeMap<K, K>) {
        self.0.push(step);
    }
    fn rename(&self, name: &K) -> K {
        self.0.iter().fold(name.clone(), |name, step| {
            step.get(&name).cloned().unwrap_or(name)
        })
    }
}

/// One node a rewrite moved to another definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replaced {
    pub node: NodeId,
    pub from: DefinitionRef,
    pub to: DefinitionRef,
}

/// Why the rewrite did not apply. The document is unchanged.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ReplaceError {
    /// The replacing definition could not be bound for this node, for example because
    /// no provider is selected for it.
    #[error("node {node} cannot become `{}@{}`: {error}", .to.id, .to.version)]
    Bind {
        node: NodeId,
        to: DefinitionRef,
        error: NewNodeError,
    },
    #[error("the replaced nodes could not be written: {0}")]
    Transaction(#[from] TxError),
}

/// Rewrites every node of `document` whose definition `registry` lacks and `table`
/// replaces (module documentation). Returns the nodes rewritten, in document order; an
/// empty list leaves the document untouched, revision included.
pub fn replace_definitions(
    document: &mut GraphDocument,
    registry: &RegistrySnapshot,
    table: &Replacements,
) -> Result<Vec<Replaced>, ReplaceError> {
    if table.is_empty() {
        return Ok(Vec::new());
    }
    let mut targets: BTreeMap<NodeId, Resolved<'_>> = BTreeMap::new();
    let mut replaced = Vec::new();
    for body in document.bodies() {
        for node in body.nodes.values() {
            if let Some(resolved) = table.resolve(&node.definition, registry) {
                replaced.push(Replaced {
                    node: node.id,
                    from: node.definition.clone(),
                    to: resolved.to.clone(),
                });
                let _ = targets.insert(node.id, resolved);
            }
        }
    }
    if replaced.is_empty() {
        return Ok(replaced);
    }
    let mut commands = vec![DocCommand::RemoveNodes {
        ids: targets.keys().copied().collect(),
    }];
    let mut wires = Vec::new();
    for (body_ref, body) in document.bodies_with_refs() {
        for node in body.nodes.values() {
            if let Some(resolved) = targets.get(&node.id) {
                commands.extend(rebuilt(document, registry, body_ref, node, resolved)?);
            }
        }
        for edge in body.edges.values() {
            let from = targets.get(&edge.from.node);
            let to = targets.get(&edge.to.node);
            if from.is_some() || to.is_some() {
                let mut edge = edge.clone();
                if let Some(resolved) = from {
                    edge.from.port = renamed_key(&edge.from.port, resolved);
                }
                if let Some(resolved) = to {
                    edge.to.port = renamed_key(&edge.to.port, resolved);
                }
                wires.push(edge);
            }
        }
    }
    commands.extend(
        wires
            .into_iter()
            .map(|edge: Edge| DocCommand::Connect { edge }),
    );
    let _ = document.apply(commands)?;
    Ok(replaced)
}

fn renamed_key(key: &PortKey, resolved: &Resolved<'_>) -> PortKey {
    PortKey::new(resolved.ports.rename(key.port()), key.instance())
}

/// The commands that add `node` again as a node of its replacing definition.
fn rebuilt(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    body: BodyRef,
    node: &NodeInstance,
    resolved: &Resolved<'_>,
) -> Result<Vec<DocCommand>, ReplaceError> {
    let config: ConfigValue = node
        .config
        .iter()
        .map(|(field, value)| (resolved.config.rename(field), value.clone()))
        .collect();
    let inputs = node
        .inputs
        .iter()
        .map(|(port, literal)| (renamed_key(port, resolved), literal.clone()))
        .collect();
    let carried = |instance: &mut NodeInstance| {
        instance.variadic = node
            .variadic
            .iter()
            .map(|(port, state)| (resolved.ports.rename(port), state.clone()))
            .collect();
        instance.activation.clone_from(&node.activation);
        instance.extensions.clone_from(&node.extensions);
    };
    let registered = registry
        .definition(&resolved.to)
        .ok_or_else(|| ReplaceError::Bind {
            node: node.id,
            to: resolved.to.clone(),
            error: NewNodeError::UnknownDefinition(resolved.to.clone()),
        })?;
    if resolved.to.id.is_core() {
        // Core intrinsics have no provider or saved signature, as when one is added.
        let mut instance = NodeInstance {
            id: node.id,
            definition: resolved.to.clone(),
            config_version: registered.config_version,
            config,
            type_args: BTreeMap::new(),
            type_pins: node.type_pins.clone(),
            provider: None,
            signature: None,
            inputs,
            variadic: BTreeMap::new(),
            activation: None,
            extensions: Default::default(),
        };
        carried(&mut instance);
        return Ok(vec![DocCommand::AddNode {
            body,
            node: instance,
        }]);
    }
    let new = NewNode {
        id: node.id,
        body,
        definition: resolved.to.clone(),
        config,
        type_pins: node.type_pins.clone(),
        inputs,
    };
    let mut commands = new_node_edit(document, registry, new)
        .map_err(|error| ReplaceError::Bind {
            node: node.id,
            to: resolved.to.clone(),
            error,
        })?
        .commands;
    for command in &mut commands {
        if let DocCommand::AddNode { node: instance, .. } = command {
            carried(instance);
        }
    }
    Ok(commands)
}
