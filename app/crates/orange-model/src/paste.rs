//! Copy and paste with full ID remapping (FR-02, 12 §2).
//!
//! Every pasted node, edge and declaration gets a fresh ID, so pasted content never aliases
//! the original. Config references are discovered and rewritten only at fields declared by
//! the exact built-in schema or explicit host-supplied schemas. Other config values are data.
//!
//! Declarations referenced by copied nodes travel with the fragment, transitively through
//! function bodies. On paste, a declaration the target already has (same ID) is reused, so
//! a copied Get Variable pasted into the same document reads the same variable. Otherwise
//! the declaration is pasted under a new ID.
//!
//! The `package_lock` entries of copied provider bindings travel too and are put into the
//! target, so a paste across documents keeps every binding locked. A target that locks the
//! same package version to another digest rejects the paste (`PackageLockConflict`).
//! Layout of pasted nodes is the editor's concern.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde_json::Value as Json;
use uuid::Uuid;

use crate::catalog::{ReferenceField, ReferenceKind, core_reference};
use crate::config_pointer::{pointer_get, pointer_set};
use crate::document::{
    BodyRef, ConfigValue, Edge, EventDecl, FunctionDecl, GraphBody, GraphDocument, NodeInstance,
    PackageLockEntry, SignatureSnapshot, VariableDecl,
};
use crate::edit::{DeclId, DocCommand};
use crate::error::{IdError, PasteError};
use crate::id::{EdgeId, EventDeclId, FunctionId, NodeId, SignatureKey, VariableId};
use crate::signature::semantic_key;

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
    /// Source lock entries of the package versions those nodes are bound to, in source order.
    pub package_lock: Vec<PackageLockEntry>,
    /// Typed config references captured while copying. Constructors populate these; callers
    /// building fragments directly may leave the list empty to preserve config data verbatim.
    pub references: Vec<ConfigReference>,
}

/// A config field whose string value is a typed graph identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigReference {
    /// Original node containing the field, including nodes in copied function bodies.
    pub node: NodeId,
    /// RFC 6901 JSON pointer within the node config, such as `/variable`.
    pub pointer: String,
    pub kind: ReferenceKind,
    /// Original string captured from the field. Paste rewrites only if it is still unchanged.
    pub expected: String,
}

/// Host-declared reference fields for one exact package definition/config version.
///
/// Schemas for exact host-reserved intrinsics are ignored; those fields are owned by the
/// core's built-in schema. Other provider-backed definitions need not use a package prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigReferenceSchema {
    pub definition: crate::document::DefinitionRef,
    pub config_version: u32,
    pub fields: Vec<ConfigReferenceField>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigReferenceField {
    /// RFC 6901 JSON pointer within `config`; the pointed value must be a string ID.
    pub pointer: String,
    pub kind: ReferenceKind,
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
        Self::copy_with_schemas(document, selection, &[])
    }

    /// Copies using the core's built-in reference fields plus exact host schemas for package
    /// definitions. Unknown definitions and fields are copied as ordinary config data.
    pub fn copy_with_schemas(
        document: &GraphDocument,
        selection: &[NodeId],
        schemas: &[ConfigReferenceSchema],
    ) -> Result<Self, PasteError> {
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
        fragment.capture_references(document, schemas);
        Ok(fragment)
    }

    /// Adds every declaration, signature and lock entry reachable from the fragment's nodes.
    fn capture_references(&mut self, document: &GraphDocument, schemas: &[ConfigReferenceSchema]) {
        let mut pending: Vec<NodeInstance> = self.nodes.clone();
        let mut seen_nodes = HashSet::new();
        let mut seen_declarations = HashSet::new();
        let mut bound = HashSet::new();
        while let Some(node) = pending.pop() {
            if !seen_nodes.insert(node.id) {
                continue;
            }
            if let Some(key) = &node.signature
                && let Some(snapshot) = document.signatures().get(key)
            {
                self.signatures.insert(key.clone(), snapshot.clone());
            }
            if let Some(provider) = &node.provider {
                bound.insert((provider.package.clone(), provider.version.clone()));
            }
            for field in reference_fields(&node, schemas) {
                let Ok(declaration) = field.parse(&node.config) else {
                    continue;
                };
                let Some(Json::String(text)) = pointer_get(&node.config, &field.pointer) else {
                    continue;
                };
                self.references.push(ConfigReference {
                    node: node.id,
                    pointer: field.pointer.clone(),
                    kind: field.kind,
                    expected: text.clone(),
                });
                if !seen_declarations.insert((field.kind, text.clone())) {
                    continue;
                }
                match declaration {
                    DeclId::Variable(id) => {
                        if let Some(decl) = document.variables().get(&id) {
                            self.variables.push(decl.clone());
                        }
                    }
                    DeclId::Event(id) => {
                        if let Some(decl) = document.events().get(&id) {
                            self.events.push(decl.clone());
                        }
                    }
                    DeclId::Function(id) => {
                        if let Some(decl) = document.functions().get(&id) {
                            pending.extend(decl.body.nodes.values().cloned());
                            self.functions.push(decl.clone());
                        }
                    }
                }
            }
        }
        self.package_lock = document
            .package_lock()
            .iter()
            .filter(|entry| bound.contains(&(entry.id.clone(), entry.version.clone())))
            .cloned()
            .collect();
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
        let references_by_node = remapped_references(&self.references, &remap.nodes);
        let finish = |mut node: NodeInstance| {
            if let Some(references) = references_by_node.get(&node.id) {
                remap.rewrite(&mut node.config, references);
            }
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
        let mut commands: Vec<DocCommand> = self
            .package_lock
            .iter()
            .cloned()
            .map(DocCommand::PutPackageLock)
            .collect();
        commands.extend(
            rekeyed
                .into_iter()
                .map(|(key, snapshot)| DocCommand::PutSignature {
                    key: key.clone(),
                    snapshot: snapshot.clone(),
                }),
        );
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

/// Typed declaration IDs are remapped separately so equal UUID text in different namespaces
/// cannot redirect a field to the wrong declaration kind.
#[derive(Default)]
struct Remap {
    references: HashMap<(ReferenceKind, String), String>,
    nodes: HashMap<NodeId, NodeId>,
}

/// UUID-backed ID types the remap can create.
trait FreshId: Copy + std::fmt::Display {
    fn from_uuid(uuid: Uuid) -> Result<Self, IdError>;
    const REFERENCE_KIND: Option<ReferenceKind>;
}

macro_rules! fresh_id {
    ($($ty:ty => $kind:expr),+) => {$(
        impl FreshId for $ty {
            fn from_uuid(uuid: Uuid) -> Result<Self, IdError> {
                <$ty>::from_uuid(uuid)
            }
            const REFERENCE_KIND: Option<ReferenceKind> = $kind;
        }
    )+};
}
fresh_id!(
    NodeId => None,
    EdgeId => None,
    VariableId => Some(ReferenceKind::Variable),
    EventDeclId => Some(ReferenceKind::Event),
    FunctionId => Some(ReferenceKind::Function)
);

impl Remap {
    fn fresh<T: FreshId>(
        &mut self,
        old: T,
        new_id: &mut impl FnMut() -> Uuid,
    ) -> Result<T, IdError> {
        let new = T::from_uuid(new_id())?;
        if let Some(kind) = T::REFERENCE_KIND {
            self.references
                .insert((kind, old.to_string()), new.to_string());
        }
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

    fn rewrite(&self, config: &mut ConfigValue, references: &[&ConfigReference]) {
        for reference in references {
            let unchanged = matches!(
                pointer_get(config, &reference.pointer),
                Some(Json::String(value)) if value == &reference.expected
            );
            if !unchanged {
                continue;
            }
            let Some(rewritten) = self
                .references
                .get(&(reference.kind, reference.expected.clone()))
            else {
                continue;
            };
            let _ = pointer_set(config, &reference.pointer, Json::String(rewritten.clone()));
        }
    }
}

fn remapped_references<'a>(
    references: &'a [ConfigReference],
    node_ids: &HashMap<NodeId, NodeId>,
) -> HashMap<NodeId, Vec<&'a ConfigReference>> {
    let mut by_node = HashMap::new();
    for reference in references {
        if let Some(node) = node_ids.get(&reference.node) {
            by_node
                .entry(*node)
                .or_insert_with(Vec::new)
                .push(reference);
        }
    }
    by_node
}

fn reference_fields(node: &NodeInstance, schemas: &[ConfigReferenceSchema]) -> Vec<ReferenceField> {
    let id = node.definition.id.as_str();
    if crate::document::is_intrinsic_definition(id) {
        if node.provider.is_some() {
            return Vec::new();
        }
        let Some(field) = core_reference(id) else {
            return Vec::new();
        };
        if node.definition.version != 1 || node.config_version != 1 {
            return Vec::new();
        }
        return vec![field.clone()];
    }

    if node.provider.is_none() {
        return Vec::new();
    }
    schemas
        .iter()
        .filter(|schema| {
            schema.definition == node.definition && schema.config_version == node.config_version
        })
        .flat_map(|schema| schema.fields.iter())
        .map(|field| ReferenceField {
            pointer: field.pointer.clone(),
            kind: field.kind,
        })
        .collect()
}
