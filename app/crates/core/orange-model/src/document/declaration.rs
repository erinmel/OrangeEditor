//! Variable, event, connection, type and graph-local function declarations.

use serde::{Deserialize, Serialize};

use super::keyed::Keyed;
use super::literal::Literal;
use super::{Extensions, GraphBody, NodeKind, PortDescriptor, TypeParamName};
use crate::id::{ConnectionId, EventDeclId, FunctionId, TypeDeclId, TypeExpr, VariableId};
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
    /// What each emission carries. Absent for an event-only channel: its emitters,
    /// handlers and awaiters then have no `payload` port (11 §10).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<TypeExpr>,
    pub scope: EventScope,
    #[serde(flatten)]
    pub extensions: Extensions,
}

impl EventDecl {
    /// The type an emission carries at run time: the declared payload, or for an
    /// event-only channel an option that is always None, so queues, quotas and
    /// workspace schemas treat both alike.
    pub fn carried(&self) -> TypeExpr {
        self.payload
            .clone()
            .unwrap_or_else(|| crate::id::fixed_id("core/option@1<core/bool@1>"))
    }
}

impl Keyed for EventDecl {
    type Key = EventDeclId;

    fn key(&self) -> EventDeclId {
        self.id
    }
}

/// A connection to an external service that nodes use by reference (11 §16.1).
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

/// A record or enum type a document declares itself (11 §5.4), so a graph can have its
/// own choices and structures without a package. Its type is [`LocalTypeDecl::ty`],
/// `local/t<id>@1`, fixed by `id`: renaming the type or a member changes only names.
/// Local types derive `eq`, `hash`, `display` and `serialize` structurally.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalTypeDecl {
    pub id: TypeDeclId,
    /// Display only.
    pub name: String,
    /// `kind` with its `variants` or `fields`, flattened into the declaration.
    #[serde(flatten)]
    pub shape: TypeShape,
}

impl LocalTypeDecl {
    /// The type expression values of this type carry: `local/t<32 hex digits>@1`.
    pub fn ty(&self) -> TypeExpr {
        local_type(self.id)
    }
}

/// The namespace of document-local types; no package may own it.
pub const LOCAL_NAMESPACE: &str = "local";

/// The type expression of local type `id`.
pub fn local_type(id: TypeDeclId) -> TypeExpr {
    let text = format!("{LOCAL_NAMESPACE}/t{}@1", id.as_uuid().simple());
    text.parse().unwrap_or_else(|_| {
        unreachable!("a namespace, a letter and hex digits form a type expression")
    })
}

impl Keyed for LocalTypeDecl {
    type Key = TypeDeclId;

    fn key(&self) -> TypeDeclId {
        self.id
    }
}

/// What a local type holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum TypeShape {
    /// One of several variants, each optionally carrying a value.
    Enum { variants: Vec<VariantDecl> },
    /// Named fields.
    Record { fields: Vec<FieldDecl> },
}

/// One enum variant. `id` is what values and Switch cases store; `name` is shown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VariantDecl {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<TypeExpr>,
}

/// One record field. `id` is what values store; `name` is shown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldDecl {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub ty: TypeExpr,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub optional: bool,
}
