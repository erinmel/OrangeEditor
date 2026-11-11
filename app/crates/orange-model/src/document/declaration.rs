//! Variable, event and graph-local function declarations.

use serde::{Deserialize, Serialize};

use super::keyed::Keyed;
use super::literal::Literal;
use super::{Extensions, GraphBody, NodeKind, PortDescriptor, TypeParamName};
use crate::id::{ConnectionId, EventDeclId, FunctionId, TypeExpr, VariableId};
use serde_json::{Map, Value as Json};

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

/// A connection to an external service that nodes use by reference (11 §8.6).
///
/// The declaration names an adapter and holds its settings; it carries no secret and
/// opens nothing. The embedding host owns the live session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectionDecl {
    pub id: ConnectionId,
    pub name: String,
    /// Namespaced adapter kind, such as `org.orange.net/http`.
    pub adapter: String,
    pub scope: ConnectionScope,
    /// Adapter settings. Empty for a shared connection, whose settings the host holds.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub settings: Map<String, Json>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

impl Keyed for ConnectionDecl {
    type Key = ConnectionId;

    fn key(&self) -> ConnectionId {
        self.id
    }
}

/// Who owns a connection's settings and session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConnectionScope {
    /// Settings are in this document; each run of it has its own session.
    Graph,
    /// The host holds the settings under this ID; every graph that declares the same
    /// ID uses the same session.
    Shared,
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
    pub inputs: Vec<PortDescriptor>,
    #[serde(default)]
    pub outputs: Vec<PortDescriptor>,
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
