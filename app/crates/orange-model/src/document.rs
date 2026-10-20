//! Graph document structures (12 §3.1), serialized exactly as format v1 (12 §6).
//!
//! The codec checks structure only; whether nodes, ports and types make sense is the
//! validator's job (00d), so unknown definitions and orphan edges load fine (12 I-4).
//! Every extensible object keeps unknown keys in `extensions` so that a newer writer's
//! fields survive load→save (FR-05).
//!
//! State owned by transactions (revision, bodies, declarations, editor layout) is private
//! and changes only through [`GraphDocument::apply`]. Locks and exports have no document
//! command yet (12 §3.3) and stay public until their flows exist.

use std::collections::BTreeMap;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as Json};

use crate::id::{
    DefinitionId, EdgeId, EventDeclId, FunctionId, GraphId, NodeId, PortId, PortKey, SignatureKey,
    TypeExpr, VariableId,
};
use crate::keyed::Keyed;
use crate::literal::Literal;

/// Unknown keys of an object, preserved in their original order.
pub type Extensions = Map<String, Json>;
/// Node configuration, validated later against the definition's `config_schema`.
pub type ConfigValue = Map<String, Json>;
/// Name of a definition's type parameter, such as `T`.
pub type TypeParamName = String;

/// The format v1 document, without the `"format"` envelope tag, which the codec owns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphDocument {
    pub format_version: u32,
    pub graph_id: GraphId,
    pub(crate) revision: u64,
    /// Identifies the content within this session; see [`GraphDocument::state_id`].
    #[serde(skip)]
    pub(crate) state_id: u64,
    #[serde(default)]
    pub package_lock: Vec<PackageLockEntry>,
    #[serde(default)]
    pub type_lock: Vec<TypeLockEntry>,
    #[serde(default, with = "crate::keyed")]
    pub(crate) variables: IndexMap<VariableId, VariableDecl>,
    #[serde(default, with = "crate::keyed")]
    pub(crate) events: IndexMap<EventDeclId, EventDecl>,
    #[serde(default)]
    pub(crate) signatures: BTreeMap<SignatureKey, SignatureSnapshot>,
    /// Top-level body, persisted as the document's own `nodes` and `edges`.
    #[serde(flatten)]
    pub(crate) root: GraphBody,
    #[serde(default, with = "crate::keyed")]
    pub(crate) functions: IndexMap<FunctionId, FunctionDecl>,
    #[serde(default)]
    pub exports: Vec<GraphExport>,
    /// Layout only; never read by the compiler. Typed by the editor work, opaque here.
    #[serde(default)]
    pub(crate) editor: Map<String, Json>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

/// Which body of a document holds a node or edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BodyRef {
    Root,
    Function(FunctionId),
}

impl GraphDocument {
    /// An empty format v1 document at revision 0.
    pub fn new(graph_id: GraphId) -> Self {
        Self {
            format_version: 1,
            graph_id,
            revision: 0,
            state_id: 0,
            package_lock: Vec::new(),
            type_lock: Vec::new(),
            variables: IndexMap::new(),
            events: IndexMap::new(),
            signatures: BTreeMap::new(),
            root: GraphBody::default(),
            functions: IndexMap::new(),
            exports: Vec::new(),
            editor: Map::new(),
            extensions: Extensions::new(),
        }
    }

    /// +1 per committed transaction, undo and redo included.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Identity of the current content within this session, used to bind undo/redo
    /// inverses. A new edit sets it to the new revision; undo/redo restore an earlier one.
    /// Not persisted: a loaded document starts with its revision.
    pub fn state_id(&self) -> u64 {
        self.state_id
    }

    pub fn root(&self) -> &GraphBody {
        &self.root
    }

    pub fn variables(&self) -> &IndexMap<VariableId, VariableDecl> {
        &self.variables
    }

    pub fn events(&self) -> &IndexMap<EventDeclId, EventDecl> {
        &self.events
    }

    pub fn functions(&self) -> &IndexMap<FunctionId, FunctionDecl> {
        &self.functions
    }

    pub fn signatures(&self) -> &BTreeMap<SignatureKey, SignatureSnapshot> {
        &self.signatures
    }

    pub fn editor(&self) -> &Map<String, Json> {
        &self.editor
    }

    /// The root body followed by every function body.
    pub fn bodies(&self) -> impl Iterator<Item = &GraphBody> {
        self.bodies_with_refs().map(|(_, body)| body)
    }

    /// Bodies with their references, root first.
    pub fn bodies_with_refs(&self) -> impl Iterator<Item = (BodyRef, &GraphBody)> {
        let functions = self
            .functions
            .iter()
            .map(|(id, function)| (BodyRef::Function(*id), &function.body));
        std::iter::once((BodyRef::Root, &self.root)).chain(functions)
    }

    pub fn body(&self, body: BodyRef) -> Option<&GraphBody> {
        match body {
            BodyRef::Root => Some(&self.root),
            BodyRef::Function(id) => self.functions.get(&id).map(|function| &function.body),
        }
    }

    /// The body holding `node`. Linear in the number of bodies until 00b.3 adds indexes.
    pub fn locate_node(&self, node: NodeId) -> Option<BodyRef> {
        self.bodies_with_refs()
            .find(|(_, body)| body.nodes.contains_key(&node))
            .map(|(body, _)| body)
    }

    pub fn locate_edge(&self, edge: EdgeId) -> Option<BodyRef> {
        self.bodies_with_refs()
            .find(|(_, body)| body.edges.contains_key(&edge))
            .map(|(body, _)| body)
    }

    pub fn node(&self, node: NodeId) -> Option<&NodeInstance> {
        self.bodies().find_map(|body| body.nodes.get(&node))
    }

    pub fn edge(&self, edge: EdgeId) -> Option<&Edge> {
        self.bodies().find_map(|body| body.edges.get(&edge))
    }

    /// The instance literal the compiler would use for `port`: `None` when there is none or
    /// when a connected edge shadows it (11 §4.4). A shadowed literal stays in the document.
    pub fn active_literal(&self, port: &PortRef) -> Option<&Literal> {
        let body = self.body(self.locate_node(port.node)?)?;
        let literal = body.nodes.get(&port.node)?.inputs.get(&port.port)?;
        let shadowed = body.edges.values().any(|edge| edge.to == *port);
        (!shadowed).then_some(literal)
    }
}

/// Nodes and edges of one body. Edges never cross bodies.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GraphBody {
    #[serde(default, with = "crate::keyed")]
    pub nodes: IndexMap<NodeId, NodeInstance>,
    #[serde(default, with = "crate::keyed")]
    pub edges: IndexMap<EdgeId, Edge>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackageLockEntry {
    pub id: String,
    pub version: String,
    pub content_digest: String,
    #[serde(flatten)]
    pub extensions: Extensions,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypeLockEntry {
    #[serde(rename = "type")]
    pub ty: TypeExpr,
    pub codec: String,
    #[serde(flatten)]
    pub extensions: Extensions,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeInstance {
    pub id: NodeId,
    #[serde(flatten)]
    pub definition: DefinitionRef,
    pub config_version: u32,
    #[serde(default)]
    pub config: ConfigValue,
    /// Persisted inference result.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub type_args: BTreeMap<TypeParamName, TypeExpr>,
    /// `None` only for `core/` intrinsics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderBinding>,
    /// Required for every non-intrinsic node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<SignatureKey>,
    /// Includes literals shadowed by a connected edge.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub inputs: BTreeMap<PortKey, Literal>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub variadic: BTreeMap<PortId, VariadicState>,
    /// Entries only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activation: Option<ActivationPolicy>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

impl Keyed for NodeInstance {
    type Key = NodeId;

    fn key(&self) -> NodeId {
        self.id
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefinitionRef {
    #[serde(rename = "definition_id")]
    pub id: DefinitionId,
    #[serde(rename = "definition_version")]
    pub version: u32,
}

/// Exact provider selected for a node; changed only by an explicit rebind (ADR-07).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderBinding {
    pub package: String,
    pub version: String,
    pub id: String,
    #[serde(flatten)]
    pub extensions: Extensions,
}

/// Variadic instance order plus the next never-reused counter (11 §4.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VariadicState {
    pub order: Vec<u16>,
    pub next: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActivationPolicy {
    pub mode: ReentrancyMode,
    /// Required by `parallel`; checked by the validator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_parallel: Option<u32>,
    pub capacity: u32,
    pub overflow: OverflowPolicy,
    pub on_fault: FaultPolicy,
    pub order: u32,
    #[serde(flatten)]
    pub extensions: Extensions,
}

/// 11 §10.1. Packages cannot add modes (11 §16), so the set is closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReentrancyMode {
    Queue,
    Drop,
    Replace,
    Parallel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OverflowPolicy {
    Backpressure,
    DropOldest,
    DropNewest,
    CoalesceLatest,
    Fault,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FaultPolicy {
    Continue,
    StopRun,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    pub id: EdgeId,
    pub from: PortRef,
    pub to: PortRef,
    #[serde(flatten)]
    pub extensions: Extensions,
}

impl Keyed for Edge {
    type Key = EdgeId;

    fn key(&self) -> EdgeId {
        self.id
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PortRef {
    pub node: NodeId,
    pub port: PortKey,
}

/// Semantic port projection shared by nodes with the same resolved signature (12 §3.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignatureSnapshot {
    pub kind: NodeKind,
    pub ports: Vec<ResolvedPort>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

/// Kinds that can appear in a signature or function interface. Control and zone
/// intrinsics have no stored signature (11 §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NodeKind {
    Pure,
    Volatile,
    Action,
    Async,
    Entry,
    Iterator,
}

/// Port descriptor fields from 11 §4.1 that this slice types. The rest (`optional`,
/// `variadic`, `lazy`, presentation in function interfaces) is kept in `extensions`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedPort {
    pub port_id: PortId,
    pub direction: PortDirection,
    pub kind: PortKind,
    /// Absent for exec ports.
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub ty: Option<TypeExpr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<OutputSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub available_on: Option<Vec<PortId>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Literal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensitivity: Option<Sensitivity>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PortDirection {
    Input,
    Output,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PortKind {
    Data,
    Exec,
    Event,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OutputSource {
    Computed,
    Execution,
    State,
    Payload,
    Iteration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Sensitivity {
    Normal,
    Sensitive,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VariableDecl {
    pub id: VariableId,
    /// Display only; identity is `id`.
    pub name: String,
    #[serde(rename = "type")]
    pub ty: TypeExpr,
    pub scope: VarScope,
    /// `None` → `core/UninitializedVariable` on read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub init: Option<Literal>,
    #[serde(default)]
    pub persistent: bool,
    #[serde(default)]
    pub conflict: ConflictPolicy,
    #[serde(flatten)]
    pub extensions: Extensions,
}

impl Keyed for VariableDecl {
    type Key = VariableId;

    fn key(&self) -> VariableId {
        self.id
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VarScope {
    Local,
    Run,
    Workspace,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConflictPolicy {
    #[default]
    Reject,
    LastWriterWins,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventDecl {
    pub id: EventDeclId,
    pub name: String,
    pub payload: TypeExpr,
    pub scope: EventScope,
    #[serde(flatten)]
    pub extensions: Extensions,
}

impl Keyed for EventDecl {
    type Key = EventDeclId;

    fn key(&self) -> EventDeclId {
        self.id
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EventScope {
    Run,
    Workspace,
}

/// Graph-local function (11 §14); its body is persisted as its own `nodes` and `edges`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionDecl {
    pub id: FunctionId,
    pub name: String,
    pub kind: NodeKind,
    #[serde(default)]
    pub type_params: Vec<TypeParam>,
    #[serde(default)]
    pub inputs: Vec<ResolvedPort>,
    #[serde(default)]
    pub outputs: Vec<ResolvedPort>,
    #[serde(flatten)]
    pub body: GraphBody,
    #[serde(flatten)]
    pub extensions: Extensions,
}

impl Keyed for FunctionDecl {
    type Key = FunctionId;

    fn key(&self) -> FunctionId {
        self.id
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypeParam {
    pub name: TypeParamName,
    #[serde(default)]
    pub constraints: Vec<String>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

/// An output evaluated for preview or the CLI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphExport {
    pub id: String,
    pub node: NodeId,
    pub port: PortKey,
    #[serde(flatten)]
    pub extensions: Extensions,
}
