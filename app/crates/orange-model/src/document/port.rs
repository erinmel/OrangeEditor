//! Connections and the semantic port descriptors stored in signatures.

use serde::{Deserialize, Serialize};

use super::Extensions;
use super::keyed::Keyed;
use super::literal::Literal;
use crate::id::{EdgeId, NodeId, PortId, PortKey, TypeExpr};

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
    /// Semantic suspension contract for execution and function-kind analysis.
    /// `None` preserves unknown suspension metadata on legacy async/iterator snapshots.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suspends: Option<Suspends>,
    pub ports: Vec<PortDescriptor>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

/// Whether an execution can suspend its strand (11 §§3, 8, 14).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Suspends {
    /// This operation always completes in its current strand segment.
    #[default]
    Never,
    /// Suspension depends on runtime behavior, such as whether data is immediately ready.
    Maybe,
    /// This operation always suspends before it resumes or completes.
    Always,
}

impl SignatureSnapshot {
    /// Returns the known suspension behavior. Older snapshots without a value are
    /// semantically known to be synchronous for non-suspending kinds, but remain
    /// unresolved for async and iterator kinds.
    pub fn suspension(&self) -> Option<Suspends> {
        self.suspends.or(match self.kind {
            NodeKind::Async | NodeKind::Iterator => None,
            _ => Some(Suspends::Never),
        })
    }
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

/// Port descriptor fields from 11 §4.1 that this slice types. Presentation in function
/// interfaces is kept in `extensions`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PortDescriptor {
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub optional: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variadic: Option<Variadic>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lazy: Option<bool>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

/// Instance bounds of a variadic port; `max: None` is unbounded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Variadic {
    pub min: u16,
    pub max: Option<u16>,
}

impl PortDescriptor {
    /// Exec input. The constructors are for static/built-in descriptors and panic if
    /// `id` is not a valid `PortId`.
    pub fn exec_in(id: &str) -> Self {
        Self::new(id, PortDirection::Input, PortKind::Exec, None, None)
    }

    /// Exec output. Panics if `id` is not a valid `PortId`.
    pub fn exec_out(id: &str) -> Self {
        Self::new(id, PortDirection::Output, PortKind::Exec, None, None)
    }

    /// Data input. Panics if `id` is not a valid `PortId`.
    pub fn data_in(id: &str, ty: TypeExpr) -> Self {
        Self::new(id, PortDirection::Input, PortKind::Data, Some(ty), None)
    }

    /// Data output. Panics if `id` is not a valid `PortId`.
    pub fn data_out(id: &str, ty: TypeExpr, source: OutputSource) -> Self {
        Self::new(
            id,
            PortDirection::Output,
            PortKind::Data,
            Some(ty),
            Some(source),
        )
    }

    pub fn lazy(mut self) -> Self {
        self.lazy = Some(true);
        self
    }

    pub fn is_optional(&self) -> bool {
        self.optional.unwrap_or(false)
    }

    pub fn is_lazy(&self) -> bool {
        self.lazy.unwrap_or(false)
    }

    pub fn optional(mut self) -> Self {
        self.optional = Some(true);
        self
    }

    pub fn variadic(mut self, min: u16, max: Option<u16>) -> Self {
        self.variadic = Some(Variadic { min, max });
        self
    }

    pub fn with_default(mut self, default: Literal) -> Self {
        self.default = Some(default);
        self
    }

    fn new(
        id: &str,
        direction: PortDirection,
        kind: PortKind,
        ty: Option<TypeExpr>,
        source: Option<OutputSource>,
    ) -> Self {
        Self {
            port_id: id
                .parse()
                .unwrap_or_else(|_| panic!("invalid static port id {id:?}")),
            direction,
            kind,
            ty,
            source,
            available_on: None,
            default: None,
            sensitivity: None,
            optional: None,
            variadic: None,
            lazy: None,
            extensions: Extensions::new(),
        }
    }
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
