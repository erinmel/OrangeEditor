//! Copy and paste with full ID remapping (FR-02, 12 §2).
//!
//! Every pasted node, edge and declaration gets a fresh ID, so pasted content never aliases
//! the original. References are found and rewritten by one rule: a config string exactly
//! equal to a source ID being remapped is a reference. IDs are random 122-bit UUIDs, so this
//! needs no knowledge of which config keys of which definitions hold references, and an
//! accidental match is negligible. Literals are user data and are never rewritten.
//!
//! Declarations referenced by copied nodes travel with the fragment, transitively through
//! function bodies. On paste, a declaration the target already has (same ID) is reused, so
//! a copied Get Variable pasted into the same document reads the same variable. Otherwise
//! the declaration is pasted under a new ID.
//!
//! Layout of pasted nodes is the editor's concern; package-lock merging for package nodes
//! pasted across documents belongs to the package flow (ADR-07).

use std::collections::{BTreeMap, HashMap, HashSet};

use serde_json::Value as Json;
use uuid::Uuid;

use crate::document::{
    BodyRef, ConfigValue, Edge, EventDecl, FunctionDecl, GraphBody, GraphDocument, NodeInstance,
    SignatureSnapshot, VariableDecl,
};
use crate::error::{IdError, PasteError};
use crate::id::{EdgeId, EventDeclId, FunctionId, NodeId, SignatureKey, VariableId};
use crate::signature::semantic_key;
use crate::transaction::DocCommand;

/// Copied content, independent of the source document.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Fragment {
    /// In source body order.
    pub nodes: Vec<NodeInstance>,
    /// Only edges with both endpoints among `nodes`.
    pub edges: Vec<Edge>,
    pub variables: Vec<VariableDecl>,
    pub events: Vec<EventDecl>,
    pub functions: Vec<FunctionDecl>,
    /// Snapshots referenced by `nodes` and by nodes in `functions`.
    pub signatures: BTreeMap<SignatureKey, SignatureSnapshot>,
}

/// Commands for one paste transaction, plus where each copied node went.
#[derive(Debug, Clone, PartialEq)]
pub struct Paste {
    pub commands: Vec<DocCommand>,
    /// Source node ID → pasted node ID, for the fragment's top-level nodes.
    pub nodes: BTreeMap<NodeId, NodeId>,
}

impl Fragment {
    /// Copies `selection` (all from one body) with its internal edges and the declarations
    /// and signatures it references.
    pub fn copy(document: &GraphDocument, selection: &[NodeId]) -> Result<Self, PasteError> {
        let Some(first) = selection.first() else {
            return Ok(Self::default());
        };
        let body_ref = document
            .locate_node(*first)
            .ok_or(PasteError::NodeNotFound(*first))?;
        for id in selection {
            match document.locate_node(*id) {
                None => return Err(PasteError::NodeNotFound(*id)),
                Some(body) if body != body_ref => return Err(PasteError::SelectionSpansBodies),
                Some(_) => {}
            }
        }
        let body = document
            .body(body_ref)
            .ok_or(PasteError::NodeNotFound(*first))?;
        let selected: HashSet<NodeId> = selection.iter().copied().collect();
        let nodes: Vec<NodeInstance> = body
            .nodes
            .values()
            .filter(|node| selected.contains(&node.id))
            .cloned()
            .collect();
        let edges = body
            .edges
            .values()
            .filter(|edge| selected.contains(&edge.from.node) && selected.contains(&edge.to.node))
            .cloned()
            .collect();

        let mut fragment = Self {
            nodes,
            edges,
            ..Self::default()
        };
        fragment.capture_references(document);
        Ok(fragment)
    }

    /// Adds every declaration and signature reachable from the fragment's nodes.
    fn capture_references(&mut self, document: &GraphDocument) {
        let mut pending: Vec<NodeInstance> = self.nodes.clone();
        let mut seen = HashSet::new();
        while let Some(node) = pending.pop() {
            if let Some(key) = &node.signature
                && let Some(snapshot) = document.signatures().get(key)
            {
                self.signatures.insert(key.clone(), snapshot.clone());
            }
            for text in config_strings(&node.config) {
                if !seen.insert(text.to_owned()) {
                    continue;
                }
                if let Some(decl) = text
                    .parse::<VariableId>()
                    .ok()
                    .and_then(|id| document.variables().get(&id))
                {
                    self.variables.push(decl.clone());
                } else if let Some(decl) = text
                    .parse::<EventDeclId>()
                    .ok()
                    .and_then(|id| document.events().get(&id))
                {
                    self.events.push(decl.clone());
                } else if let Some(decl) = text
                    .parse::<FunctionId>()
                    .ok()
                    .and_then(|id| document.functions().get(&id))
                {
                    pending.extend(decl.body.nodes.values().cloned());
                    self.functions.push(decl.clone());
                }
            }
        }
    }

    /// Commands that paste the fragment into `body` of `target` as one transaction. New IDs
    /// come from `new_id`, which must produce v4 or v7 UUIDs; the model itself has no source
    /// of randomness.
    pub fn paste(
        &self,
        target: &GraphDocument,
        body: BodyRef,
        mut new_id: impl FnMut() -> Uuid,
    ) -> Result<Paste, IdError> {
        let mut remap = Remap::default();
        let variables: Vec<VariableDecl> = self
            .variables
            .iter()
            .filter(|decl| !target.variables().contains_key(&decl.id))
            .map(|decl| {
                Ok(VariableDecl {
                    id: remap.fresh(decl.id, &mut new_id)?,
                    ..decl.clone()
                })
            })
            .collect::<Result<_, IdError>>()?;
        let events: Vec<EventDecl> = self
            .events
            .iter()
            .filter(|decl| !target.events().contains_key(&decl.id))
            .map(|decl| {
                Ok(EventDecl {
                    id: remap.fresh(decl.id, &mut new_id)?,
                    ..decl.clone()
                })
            })
            .collect::<Result<_, IdError>>()?;
        let mut functions: Vec<FunctionDecl> = Vec::new();
        for decl in self
            .functions
            .iter()
            .filter(|decl| !target.functions().contains_key(&decl.id))
        {
            let id = remap.fresh(decl.id, &mut new_id)?;
            let body = remap.body(&decl.body, &mut new_id)?;
            functions.push(FunctionDecl {
                id,
                body,
                ..decl.clone()
            });
        }
        let nodes = remap.nodes(&self.nodes, &mut new_id)?;
        let edges = remap.edges(&self.edges, &mut new_id)?;

        // Rewrite references only after every new ID is known.
        let signature_keys: HashMap<&SignatureKey, SignatureKey> = self
            .signatures
            .iter()
            .map(|(key, snapshot)| (key, semantic_key(snapshot)))
            .collect();
        let finish = |mut node: NodeInstance| {
            remap.rewrite(&mut node.config);
            node.signature = node
                .signature
                .map(|key| signature_keys.get(&key).cloned().unwrap_or(key));
            node
        };
        let functions: Vec<FunctionDecl> = functions
            .into_iter()
            .map(|mut decl| {
                decl.body.nodes = decl
                    .body
                    .nodes
                    .into_iter()
                    .map(|(id, node)| (id, finish(node)))
                    .collect();
                decl
            })
            .collect();

        let rekeyed: BTreeMap<&SignatureKey, &SignatureSnapshot> = self
            .signatures
            .iter()
            .map(|(key, snapshot)| (&signature_keys[key], snapshot))
            .collect();
        let mut commands: Vec<DocCommand> = rekeyed
            .into_iter()
            .map(|(key, snapshot)| DocCommand::PutSignature {
                key: key.clone(),
                snapshot: snapshot.clone(),
            })
            .collect();
        commands.extend(variables.into_iter().map(DocCommand::UpsertVariable));
        commands.extend(events.into_iter().map(DocCommand::UpsertEvent));
        commands.extend(functions.into_iter().map(DocCommand::UpsertFunction));
        let node_map: BTreeMap<NodeId, NodeId> = self
            .nodes
            .iter()
            .zip(&nodes)
            .map(|(old, new)| (old.id, new.id))
            .collect();
        commands.extend(nodes.into_iter().map(|node| DocCommand::AddNode {
            body,
            node: finish(node),
        }));
        commands.extend(edges.into_iter().map(|edge| DocCommand::Connect { edge }));
        Ok(Paste {
            commands,
            nodes: node_map,
        })
    }
}

/// Source ID → pasted ID, as text (for config rewriting) and per node for edges.
#[derive(Default)]
struct Remap {
    text: HashMap<String, String>,
    nodes: HashMap<NodeId, NodeId>,
}

/// UUID-backed ID types the remap can create.
trait FreshId: Copy + std::fmt::Display {
    fn from_uuid(uuid: Uuid) -> Result<Self, IdError>;
}

macro_rules! fresh_id {
    ($($ty:ty),+) => {$(
        impl FreshId for $ty {
            fn from_uuid(uuid: Uuid) -> Result<Self, IdError> {
                <$ty>::from_uuid(uuid)
            }
        }
    )+};
}
fresh_id!(NodeId, EdgeId, VariableId, EventDeclId, FunctionId);

impl Remap {
    fn fresh<T: FreshId>(
        &mut self,
        old: T,
        new_id: &mut impl FnMut() -> Uuid,
    ) -> Result<T, IdError> {
        let new = T::from_uuid(new_id())?;
        self.text.insert(old.to_string(), new.to_string());
        Ok(new)
    }

    fn nodes(
        &mut self,
        nodes: &[NodeInstance],
        new_id: &mut impl FnMut() -> Uuid,
    ) -> Result<Vec<NodeInstance>, IdError> {
        nodes
            .iter()
            .map(|node| {
                let id = self.fresh(node.id, new_id)?;
                self.nodes.insert(node.id, id);
                Ok(NodeInstance { id, ..node.clone() })
            })
            .collect()
    }

    /// Edges must be remapped after their endpoint nodes.
    fn edges(
        &mut self,
        edges: &[Edge],
        new_id: &mut impl FnMut() -> Uuid,
    ) -> Result<Vec<Edge>, IdError> {
        edges
            .iter()
            .map(|edge| {
                let mut edge = edge.clone();
                edge.id = self.fresh(edge.id, new_id)?;
                edge.from.node = self.nodes[&edge.from.node];
                edge.to.node = self.nodes[&edge.to.node];
                Ok(edge)
            })
            .collect()
    }

    /// Orphan edges of a copied function body are dropped: pasted content starts consistent.
    fn body(
        &mut self,
        body: &GraphBody,
        new_id: &mut impl FnMut() -> Uuid,
    ) -> Result<GraphBody, IdError> {
        let nodes: Vec<NodeInstance> = body.nodes.values().cloned().collect();
        let edges: Vec<Edge> = body
            .edges
            .values()
            .filter(|edge| {
                body.nodes.contains_key(&edge.from.node) && body.nodes.contains_key(&edge.to.node)
            })
            .cloned()
            .collect();
        let nodes = self.nodes(&nodes, new_id)?;
        let edges = self.edges(&edges, new_id)?;
        Ok(GraphBody {
            nodes: nodes.into_iter().map(|node| (node.id, node)).collect(),
            edges: edges.into_iter().map(|edge| (edge.id, edge)).collect(),
        })
    }

    fn rewrite(&self, config: &mut ConfigValue) {
        for value in config.values_mut() {
            rewrite_value(value, &self.text);
        }
    }
}

fn rewrite_value(value: &mut Json, remap: &HashMap<String, String>) {
    match value {
        Json::String(text) => {
            if let Some(new) = remap.get(text.as_str()) {
                *text = new.clone();
            }
        }
        Json::Array(items) => items.iter_mut().for_each(|item| rewrite_value(item, remap)),
        Json::Object(map) => map.values_mut().for_each(|item| rewrite_value(item, remap)),
        _ => {}
    }
}

/// Every string anywhere in a config value.
fn config_strings(config: &ConfigValue) -> Vec<&str> {
    fn walk<'a>(value: &'a Json, out: &mut Vec<&'a str>) {
        match value {
            Json::String(text) => out.push(text),
            Json::Array(items) => items.iter().for_each(|item| walk(item, out)),
            Json::Object(map) => map.values().for_each(|item| walk(item, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    config.values().for_each(|value| walk(value, &mut out));
    out
}
