//! Document transactions (12 §3.3). A transaction applies its commands atomically, yields
//! one [`ChangeSet`] and one [`Inverse`], and increments the revision once.
//!
//! Commands check only what the document's storage cannot represent: unique IDs, edge
//! endpoints in one body, existing targets. Port kinds, fan-in, types, regions and
//! references from config are the validator's job (00d). Runtime effects are never part of
//! a transaction (FR-03).

use std::collections::BTreeMap;

use serde_json::Value as Json;

use super::change_set::{ChangeSet, Tracker};
use super::patch::{Patch, Placed, editor_placement, placement};
use crate::document::literal::Literal;
use crate::document::{
    BodyRef, ConfigValue, Edge, EventDecl, FunctionDecl, GraphDocument, NodeInstance,
    PackageLockEntry, PortRef, ProviderBinding, SignatureSnapshot, TypeParamName, VariableDecl,
    VariadicState, fresh_state_id,
};
use crate::error::TxError;
use crate::id::{
    EdgeId, EventDeclId, FunctionId, NodeId, PortId, PortKey, SignatureKey, TypeExpr, VariableId,
};

/// One edit intent. Replacing an item keeps its position; a new item is appended.
///
/// Commands are built and consumed one transaction at a time, so variants hold their
/// values inline for caller convenience instead of boxing the large ones.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum DocCommand {
    /// `body` is explicit because nodes are stored per body.
    AddNode {
        body: BodyRef,
        node: NodeInstance,
    },
    /// Incident edges in the node's body are removed too.
    RemoveNodes {
        ids: Vec<NodeId>,
    },
    /// A literal on the target port stays in the document, shadowed (11 §4.4).
    Connect {
        edge: Edge,
    },
    Disconnect {
        edge: EdgeId,
    },
    SetLiteral {
        port: PortRef,
        literal: Option<Literal>,
    },
    SetConfig {
        node: NodeId,
        config_version: u32,
        config: ConfigValue,
    },
    SetTypeArgs {
        node: NodeId,
        args: BTreeMap<TypeParamName, TypeExpr>,
    },
    /// Replaces the node's explicit type choices (pins, 11 §5.5).
    SetTypePins {
        node: NodeId,
        pins: BTreeMap<TypeParamName, TypeExpr>,
    },
    /// Allocates the next never-reused counter `k` and places `port#k` at `at` (11 §4.5).
    InsertVariadic {
        node: NodeId,
        port: PortId,
        at: usize,
    },
    /// Removes the instance port with its edges and literal.
    RemoveVariadic {
        node: NodeId,
        port: PortKey,
    },
    UpsertVariable(VariableDecl),
    RemoveVariable(VariableId),
    UpsertEvent(EventDecl),
    RemoveEvent(EventDeclId),
    /// Replaces the whole declaration, body included.
    UpsertFunction(FunctionDecl),
    RemoveFunction(FunctionId),
    /// Explicit provider change only (ADR-07).
    Rebind {
        nodes: Vec<NodeId>,
        provider: ProviderBinding,
    },
    /// Adds a shared signature snapshot. Putting an identical one again changes nothing;
    /// different content under an existing key is rejected.
    PutSignature {
        key: SignatureKey,
        snapshot: SignatureSnapshot,
    },
    /// Points a node at an existing shared snapshot; add it first with `PutSignature`.
    SetSignature {
        node: NodeId,
        key: SignatureKey,
    },
    /// Locks a package version to its content digest, appended to `package_lock`. Putting
    /// the same digest again changes nothing; a different digest for a locked package
    /// version is rejected. Pair it with any `AddNode` or `Rebind` naming that version.
    PutPackageLock(PackageLockEntry),
    Layout(LayoutCommand),
}

/// Edits of the opaque editor layout map. Typed layout commands arrive with `orange-editor`.
#[derive(Debug, Clone, PartialEq)]
pub enum LayoutCommand {
    /// Sets (or with `None`, removes) the value at a key path, creating parent objects.
    Set {
        path: Vec<String>,
        value: Option<Json>,
    },
}

/// A committed transaction.
#[derive(Debug, Clone, PartialEq)]
#[must_use]
pub struct Committed {
    pub change_set: ChangeSet,
    /// Undoes this transaction when passed to [`GraphDocument::revert`].
    pub inverse: Inverse,
}

/// Exact undo of one committed transaction. Reverting it is itself a transaction, whose
/// inverse is the redo.
///
/// It is bound to document content, not to the revision: undo and redo advance the revision
/// but restore the content state they return to, so a redo stays valid once other undos
/// and redos have brought the document back to the state it expects.
#[derive(Debug, Clone, PartialEq)]
pub struct Inverse {
    applies_to: u64,
    restores: u64,
    patches: Vec<Patch>,
}

impl Inverse {
    /// The only [`GraphDocument::state_id`] this inverse may be applied to.
    pub fn applies_to_state(&self) -> u64 {
        self.applies_to
    }
}

impl GraphDocument {
    /// Applies `commands` atomically. On error nothing changes, revision included.
    pub fn apply(
        &mut self,
        commands: impl IntoIterator<Item = DocCommand>,
    ) -> Result<Committed, TxError> {
        self.check_revision()?;
        let mut journal = Journal::default();
        for command in commands {
            if let Err(error) = journal.command(self, command) {
                journal.rollback(self);
                return Err(error);
            }
        }
        if journal.is_empty() {
            return Err(TxError::Empty);
        }
        Ok(journal.commit(self, None))
    }

    /// Applies `commands`, lets `observe` read the edited document and its change set, then
    /// restores the document exactly, revision and state identity included. Costs the
    /// edit rather than a copy of the document, which planning would otherwise clone.
    pub(crate) fn scratch<R>(
        &mut self,
        commands: impl IntoIterator<Item = DocCommand>,
        observe: impl FnOnce(&Self, &ChangeSet) -> R,
    ) -> Result<R, TxError> {
        self.check_revision()?;
        let mut journal = Journal::default();
        for command in commands {
            if let Err(error) = journal.command(self, command) {
                journal.rollback(self);
                return Err(error);
            }
        }
        if journal.is_empty() {
            return Err(TxError::Empty);
        }
        let change = std::mem::take(&mut journal.tracker).finish(self, self.revision);
        let result = observe(self, &change);
        journal.rollback(self);
        Ok(result)
    }

    /// Undoes (or, with a redo inverse, redoes) a committed transaction.
    pub fn revert(&mut self, inverse: Inverse) -> Result<Committed, TxError> {
        if inverse.applies_to != self.state_id {
            return Err(TxError::StaleInverse {
                expected: inverse.applies_to,
                actual: self.state_id,
            });
        }
        self.check_revision()?;
        let mut journal = Journal::default();
        for patch in inverse.patches {
            if let Err(error) = journal.patch(self, patch) {
                journal.rollback(self);
                return Err(error);
            }
        }
        Ok(journal.commit(self, Some(inverse.restores)))
    }

    /// Checked before any patch so an exhausted counter (a loaded `u64::MAX`) changes nothing.
    fn check_revision(&self) -> Result<(), TxError> {
        self.revision
            .checked_add(1)
            .map(drop)
            .ok_or(TxError::RevisionExhausted)
    }
}

/// Applied patches of an open transaction, as inverses, plus change tracking.
#[derive(Default)]
struct Journal {
    inverses: Vec<Patch>,
    commands: usize,
    tracker: Tracker,
}

impl Journal {
    fn is_empty(&self) -> bool {
        self.commands == 0
    }

    fn patch(&mut self, document: &mut GraphDocument, patch: Patch) -> Result<(), TxError> {
        let inverse = patch.apply(document)?;
        self.tracker.record(&inverse);
        self.inverses.push(inverse);
        Ok(())
    }

    fn rollback(self, document: &mut GraphDocument) {
        for inverse in self.inverses.into_iter().rev() {
            inverse
                .apply(document)
                .expect("the inverse of an applied patch always applies");
        }
    }

    /// Bumps the revision. A new edit gets a fresh content-state identity; an undo or redo
    /// returns to the identity it `restores`.
    fn commit(mut self, document: &mut GraphDocument, restores: Option<u64>) -> Committed {
        let (from_revision, from_state) = (document.revision, document.state_id);
        document.revision += 1; // checked by `check_revision` before the first patch
        document.state_id = restores.unwrap_or_else(fresh_state_id);
        self.inverses.reverse();
        Committed {
            change_set: self.tracker.finish(document, from_revision),
            inverse: Inverse {
                applies_to: document.state_id,
                restores: from_state,
                patches: self.inverses,
            },
        }
    }

    /// Checks one command against the current state and applies its patches.
    fn command(
        &mut self,
        document: &mut GraphDocument,
        command: DocCommand,
    ) -> Result<(), TxError> {
        self.commands += 1;
        match command {
            DocCommand::AddNode { body, node } => {
                if document.locate_node(node.id).is_some() {
                    return Err(TxError::NodeExists(node.id));
                }
                let index = document
                    .body(body)
                    .ok_or_else(|| missing_body(body))?
                    .nodes
                    .len();
                self.patch(
                    document,
                    Patch::Node {
                        body,
                        id: node.id,
                        entry: Some(Placed {
                            index,
                            value: Box::new(node),
                        }),
                    },
                )
            }
            DocCommand::RemoveNodes { ids } => ids
                .into_iter()
                .try_for_each(|id| self.remove_node(document, id)),
            DocCommand::Connect { edge } => {
                if document.locate_edge(edge.id).is_some() {
                    return Err(TxError::EdgeExists(edge.id));
                }
                let body = edge_body(&edge, |node| document.locate_node(node))?;
                let index = document.body(body).map_or(0, |body| body.edges.len());
                self.patch(
                    document,
                    Patch::Edge {
                        body,
                        id: edge.id,
                        entry: Some(Placed { index, value: edge }),
                    },
                )
            }
            DocCommand::Disconnect { edge } => {
                let body = document
                    .locate_edge(edge)
                    .ok_or(TxError::EdgeNotFound(edge))?;
                self.patch(
                    document,
                    Patch::Edge {
                        body,
                        id: edge,
                        entry: None,
                    },
                )
            }
            DocCommand::SetLiteral { port, literal } => {
                self.modify_node(document, port.node, |node| {
                    match literal {
                        Some(literal) => node.inputs.insert(port.port, literal),
                        None => node.inputs.remove(&port.port),
                    };
                    Ok(())
                })
            }
            DocCommand::SetConfig {
                node,
                config_version,
                config,
            } => self.modify_node(document, node, |node| {
                node.config_version = config_version;
                node.config = config;
                Ok(())
            }),
            DocCommand::SetTypeArgs { node, args } => self.modify_node(document, node, |node| {
                node.type_args = args;
                Ok(())
            }),
            DocCommand::SetTypePins { node, pins } => self.modify_node(document, node, |node| {
                node.type_pins = pins;
                Ok(())
            }),
            DocCommand::InsertVariadic { node: id, port, at } => {
                self.modify_node(document, id, |node| {
                    let state = node.variadic.entry(port.clone()).or_insert(VariadicState {
                        order: Vec::new(),
                        next: 0,
                    });
                    if at > state.order.len() {
                        return Err(TxError::VariadicPositionOutOfRange {
                            node: id,
                            port,
                            at,
                            len: state.order.len(),
                        });
                    }
                    let k = state.next;
                    // Counters are never reused, so an exhausted counter fails instead of wrapping.
                    state.next = k
                        .checked_add(1)
                        .ok_or(TxError::VariadicExhausted { node: id, port })?;
                    state.order.insert(at, k);
                    Ok(())
                })
            }
            DocCommand::RemoveVariadic { node: id, port } => {
                self.remove_variadic(document, id, port)
            }
            DocCommand::UpsertVariable(decl) => {
                let index = placement(&document.variables, &decl.id);
                self.patch(
                    document,
                    Patch::Variable {
                        id: decl.id,
                        entry: Some(Placed { index, value: decl }),
                    },
                )
            }
            DocCommand::RemoveVariable(id) => {
                if !document.variables.contains_key(&id) {
                    return Err(TxError::VariableNotFound(id));
                }
                self.patch(document, Patch::Variable { id, entry: None })
            }
            DocCommand::UpsertEvent(decl) => {
                let index = placement(&document.events, &decl.id);
                self.patch(
                    document,
                    Patch::Event {
                        id: decl.id,
                        entry: Some(Placed { index, value: decl }),
                    },
                )
            }
            DocCommand::RemoveEvent(id) => {
                if !document.events.contains_key(&id) {
                    return Err(TxError::EventNotFound(id));
                }
                self.patch(document, Patch::Event { id, entry: None })
            }
            DocCommand::UpsertFunction(decl) => {
                check_function_body(document, &decl)?;
                let (id, index) = (decl.id, placement(&document.functions, &decl.id));
                let entry = Some(Placed {
                    index,
                    value: Box::new(decl),
                });
                self.patch(document, Patch::Function { id, entry })
            }
            DocCommand::RemoveFunction(id) => {
                if !document.functions.contains_key(&id) {
                    return Err(TxError::FunctionNotFound(id));
                }
                self.patch(document, Patch::Function { id, entry: None })
            }
            DocCommand::Rebind { nodes, provider } => nodes.into_iter().try_for_each(|id| {
                self.modify_node(document, id, |node| {
                    node.provider = Some(provider.clone());
                    Ok(())
                })
            }),
            DocCommand::PutSignature { key, snapshot } => match document.signatures.get(&key) {
                Some(existing) if *existing == snapshot => Ok(()),
                Some(_) => Err(TxError::SignatureConflict(key)),
                None => self.patch(
                    document,
                    Patch::Signature {
                        key,
                        entry: Some(Placed {
                            index: 0,
                            value: snapshot,
                        }),
                    },
                ),
            },
            DocCommand::PutPackageLock(entry) => {
                let key = (entry.id.clone(), entry.version.clone());
                let locked = document
                    .package_lock
                    .iter()
                    .find(|locked| (&locked.id, &locked.version) == (&key.0, &key.1));
                match locked {
                    Some(locked) if locked.content_digest == entry.content_digest => Ok(()),
                    Some(_) => Err(TxError::PackageLockConflict {
                        package: key.0,
                        version: key.1,
                    }),
                    None => {
                        let index = document.package_lock.len();
                        self.patch(
                            document,
                            Patch::PackageLock {
                                key,
                                entry: Some(Placed {
                                    index,
                                    value: entry,
                                }),
                            },
                        )
                    }
                }
            }
            DocCommand::SetSignature { node, key } => {
                if !document.signatures.contains_key(&key) {
                    return Err(TxError::SignatureNotFound(key));
                }
                self.modify_node(document, node, |node| {
                    node.signature = Some(key);
                    Ok(())
                })
            }
            DocCommand::Layout(LayoutCommand::Set { path, value }) => {
                if path.is_empty() {
                    return Err(TxError::EmptyLayoutPath);
                }
                let index = editor_placement(&document.editor, &path);
                self.patch(
                    document,
                    Patch::Editor {
                        path,
                        entry: value.map(|value| Placed { index, value }),
                    },
                )
            }
        }
    }

    /// Replaces a node in place with an edited copy; the edit may reject the change.
    fn modify_node(
        &mut self,
        document: &mut GraphDocument,
        id: NodeId,
        edit: impl FnOnce(&mut NodeInstance) -> Result<(), TxError>,
    ) -> Result<(), TxError> {
        let body = document.locate_node(id).ok_or(TxError::NodeNotFound(id))?;
        let nodes = &document.body(body).ok_or_else(|| missing_body(body))?.nodes;
        let index = placement(nodes, &id);
        let mut node = nodes[&id].clone();
        edit(&mut node)?;
        self.patch(
            document,
            Patch::Node {
                body,
                id,
                entry: Some(Placed {
                    index,
                    value: Box::new(node),
                }),
            },
        )
    }

    fn remove_node(&mut self, document: &mut GraphDocument, id: NodeId) -> Result<(), TxError> {
        let body = document.locate_node(id).ok_or(TxError::NodeNotFound(id))?;
        self.remove_edges_where(document, body, |edge| {
            edge.from.node == id || edge.to.node == id
        })?;
        self.patch(
            document,
            Patch::Node {
                body,
                id,
                entry: None,
            },
        )
    }

    fn remove_variadic(
        &mut self,
        document: &mut GraphDocument,
        id: NodeId,
        port: PortKey,
    ) -> Result<(), TxError> {
        let not_found = || TxError::VariadicPortNotFound {
            node: id,
            port: port.clone(),
        };
        let node = document.node(id).ok_or(TxError::NodeNotFound(id))?;
        let k = port.instance().ok_or_else(not_found)?;
        let listed = node
            .variadic
            .get(port.port())
            .is_some_and(|state| state.order.contains(&k));
        if !listed {
            return Err(not_found());
        }
        let body = document.locate_node(id).ok_or(TxError::NodeNotFound(id))?;
        let touches = |end: &PortRef| end.node == id && end.port == port;
        self.remove_edges_where(document, body, |edge| {
            touches(&edge.from) || touches(&edge.to)
        })?;
        self.modify_node(document, id, |node| {
            if let Some(state) = node.variadic.get_mut(port.port()) {
                state.order.retain(|&instance| instance != k);
            }
            node.inputs.remove(&port);
            Ok(())
        })
    }

    fn remove_edges_where(
        &mut self,
        document: &mut GraphDocument,
        body: BodyRef,
        matches: impl Fn(&Edge) -> bool,
    ) -> Result<(), TxError> {
        let edges: Vec<EdgeId> = document
            .body(body)
            .ok_or_else(|| missing_body(body))?
            .edges
            .values()
            .filter(|edge| matches(edge))
            .map(|edge| edge.id)
            .collect();
        edges.into_iter().try_for_each(|id| {
            self.patch(
                document,
                Patch::Edge {
                    body,
                    id,
                    entry: None,
                },
            )
        })
    }
}

fn missing_body(body: BodyRef) -> TxError {
    match body {
        BodyRef::Function(id) => TxError::FunctionNotFound(id),
        BodyRef::Root => unreachable!("the root body always exists"),
    }
}

/// The single body holding both endpoints of `edge`, located with `locate`.
fn edge_body(edge: &Edge, locate: impl Fn(NodeId) -> Option<BodyRef>) -> Result<BodyRef, TxError> {
    let endpoint = |node| {
        locate(node).ok_or(TxError::EdgeEndpointMissing {
            edge: edge.id,
            node,
        })
    };
    let (from, to) = (endpoint(edge.from.node)?, endpoint(edge.to.node)?);
    if from == to {
        Ok(from)
    } else {
        Err(TxError::CrossBodyEdge(edge.id))
    }
}

/// A function body obeys the same rules as edits elsewhere: its IDs are not used by another
/// body and its edges stay inside it.
fn check_function_body(document: &GraphDocument, decl: &FunctionDecl) -> Result<(), TxError> {
    let this = BodyRef::Function(decl.id);
    for (key, node) in &decl.body.nodes {
        if *key != node.id {
            return Err(TxError::FunctionNodeKeyMismatch {
                key: *key,
                item: node.id,
            });
        }
    }
    for (key, edge) in &decl.body.edges {
        if *key != edge.id {
            return Err(TxError::FunctionEdgeKeyMismatch {
                key: *key,
                item: edge.id,
            });
        }
    }
    let foreign = |found: Option<BodyRef>| found.is_some_and(|body| body != this);
    if let Some(id) = decl
        .body
        .nodes
        .keys()
        .find(|id| foreign(document.locate_node(**id)))
    {
        return Err(TxError::NodeExists(*id));
    }
    if let Some(id) = decl
        .body
        .edges
        .keys()
        .find(|id| foreign(document.locate_edge(**id)))
    {
        return Err(TxError::EdgeExists(*id));
    }
    let local = |node: NodeId| decl.body.nodes.contains_key(&node).then_some(this);
    decl.body
        .edges
        .values()
        .try_for_each(|edge| edge_body(edge, local).map(|_| ()))
}
