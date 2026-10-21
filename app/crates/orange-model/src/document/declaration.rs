//! Variable, event and graph-local function declarations.

use serde::{Deserialize, Serialize};

use super::keyed::Keyed;
use super::literal::Literal;
use super::{Extensions, GraphBody, NodeKind, ResolvedPort, TypeParamName};
use crate::id::{EventDeclId, FunctionId, TypeExpr, VariableId};

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
