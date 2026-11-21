//! Explicit persistent-value documents. No executable state or storage service.
use orange_model::catalog::RegistrySnapshot;
use orange_model::document::VarScope;
use orange_model::document::literal::{Literal, LiteralPayload};
use orange_model::id::{GraphId, TypeExpr, VariableId};
use orange_model::value::{literal_codec, shape::ValueShape};
use orange_model::{GraphDocument, Value};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;
mod identity;
mod wire;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CheckpointScope {
    Run,
    Workspace,
}
impl CheckpointScope {
    fn variable_scope(self) -> VarScope {
        match self {
            Self::Run => VarScope::Run,
            Self::Workspace => VarScope::Workspace,
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub struct CheckpointLimits {
    pub max_variables: usize,
    pub max_bytes: usize,
    pub max_value_bytes: usize,
    pub max_state_bytes: usize,
    pub max_identity_items: usize,
}
impl Default for CheckpointLimits {
    fn default() -> Self {
        Self {
            max_variables: 4096,
            max_bytes: 32 * 1024 * 1024,
            max_value_bytes: 16 * 1024 * 1024,
            max_state_bytes: 32 * 1024 * 1024,
            max_identity_items: 65536,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum CheckpointError {
    #[error("The checkpoint value document is invalid.")]
    Invalid,
    #[error("The checkpoint format version is unsupported.")]
    Version,
    #[error("The checkpoint exceeds a configured bound.")]
    Limit,
    #[error("The checkpoint belongs to another graph or scope.")]
    Owner,
    #[error("The checkpoint requires a selected persistent variable.")]
    Selection,
    #[error("The checkpoint type, schema or provider identity is incompatible.")]
    Incompatible,
    #[error("The checkpoint contains a value outside the supported codec profile.")]
    UnsupportedValue,
    #[error("The run is not stopped at a checkpoint boundary.")]
    Boundary,
    #[error("The checkpoint cannot start the requested run or workspace.")]
    Startup,
}
impl CheckpointError {
    pub const fn message_key(self) -> &'static str {
        match self {
            Self::Invalid => "core/checkpoint-invalid",
            Self::Version => "core/checkpoint-version",
            Self::Limit => "core/checkpoint-limit",
            Self::Owner => "core/checkpoint-owner",
            Self::Selection => "core/checkpoint-selection",
            Self::Incompatible => "core/checkpoint-incompatible",
            Self::UnsupportedValue => "core/checkpoint-unsupported-value",
            Self::Boundary => "core/checkpoint-boundary",
            Self::Startup => "core/checkpoint-startup",
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    format: String,
    format_version: u32,
    scope: CheckpointScope,
    graph_id: GraphId,
    compatibility: String,
    values: Vec<Stored>,
}
struct Stored {
    id: VariableId,
    ty: TypeExpr,
    value: Option<Literal>,
}
impl Serialize for Stored {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        #[serde(tag = "encoding", content = "data", rename_all = "kebab-case")]
        enum BorrowedPayload<'a> {
            Value(&'a serde_json::Value),
            Bits(&'a str),
        }
        #[derive(Serialize)]
        struct View<'a> {
            id: VariableId,
            #[serde(rename = "type")]
            ty: &'a TypeExpr,
            value: Option<BorrowedPayload<'a>>,
        }
        View {
            id: self.id,
            ty: &self.ty,
            value: self.value.as_ref().map(|l| match &l.payload {
                LiteralPayload::Value(v) => BorrowedPayload::Value(v),
                LiteralPayload::Bits(b) => BorrowedPayload::Bits(b),
            }),
        }
        .serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for Stored {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            id: VariableId,
            #[serde(rename = "type")]
            ty: TypeExpr,
            #[serde(deserialize_with = "present_payload")]
            value: Option<Payload>,
        }
        fn present_payload<'de, D: serde::Deserializer<'de>>(
            d: D,
        ) -> Result<Option<Payload>, D::Error> {
            Option::<Payload>::deserialize(d)
        }
        let raw = Raw::deserialize(deserializer)?;
        if raw.value.as_ref().is_some_and(|p|matches!(p,Payload::Bits(b) if !matches!(b.len(),8|16) || !b.bytes().all(|c|c.is_ascii_digit()||(b'a'..=b'f').contains(&c)))) {
            return Err(serde::de::Error::custom("invalid checkpoint bits"));
        }
        let value = raw.value.map(|p| Literal {
            ty: raw.ty.clone(),
            display: None,
            payload: match p {
                Payload::Value(v) => LiteralPayload::Value(v),
                Payload::Bits(b) => LiteralPayload::Bits(b),
            },
        });
        Ok(Self {
            id: raw.id,
            ty: raw.ty,
            value,
        })
    }
}
/// Explicit payload tag keeps uninitialized, JSON null and Option absence distinct.
#[derive(Deserialize)]
#[serde(
    tag = "encoding",
    content = "data",
    rename_all = "kebab-case",
    deny_unknown_fields
)]
enum Payload {
    Value(serde_json::Value),
    Bits(String),
}
/// Read-only portable values. Host policy owns sensitivity and durable storage.
pub struct ValueCheckpoint {
    document: Document,
    limits: CheckpointLimits,
}
impl std::fmt::Debug for ValueCheckpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ValueCheckpoint")
            .field("version", &1)
            .field("scope", &self.scope())
            .field("graph", &self.document.graph_id)
            .field("variables", &self.len())
            .finish()
    }
}
pub(crate) type Values = BTreeMap<VariableId, Option<Value>>;
impl ValueCheckpoint {
    pub fn scope(&self) -> CheckpointScope {
        self.document.scope
    }
    pub fn len(&self) -> usize {
        self.document.values.len()
    }
    pub fn is_empty(&self) -> bool {
        self.document.values.is_empty()
    }
    pub fn graph_id(&self) -> GraphId {
        self.document.graph_id
    }
    /// Bounded structural decoding only. Restoration rechecks current authority/schema.
    pub fn from_bytes(bytes: &[u8], limits: CheckpointLimits) -> Result<Self, CheckpointError> {
        if bytes.len() > limits.max_bytes {
            return Err(CheckpointError::Limit);
        }
        wire::preflight(bytes, limits.max_variables)?;
        let document: Document =
            serde_json::from_slice(bytes).map_err(|_| CheckpointError::Invalid)?;
        if document.format != "orange.checkpoint" || document.format_version != 1 {
            return Err(CheckpointError::Version);
        }
        if document.values.len() > limits.max_variables {
            return Err(CheckpointError::Limit);
        }
        let mut ids = BTreeSet::new();
        if document.values.iter().any(|v| !ids.insert(v.id)) {
            return Err(CheckpointError::Invalid);
        }
        if document.compatibility.len() != 64
            || !document
                .compatibility
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(CheckpointError::Invalid);
        }
        Ok(Self { document, limits })
    }
    pub fn to_bytes(&self) -> Result<Vec<u8>, CheckpointError> {
        wire::encode(&self.document, self.limits.max_bytes)
    }
    pub(crate) fn capture<'a>(
        document: &GraphDocument,
        registry: &RegistrySnapshot,
        scope: CheckpointScope,
        selection: &[VariableId],
        limits: CheckpointLimits,
        mut get: impl FnMut(VariableId) -> Result<Option<&'a Value>, CheckpointError>,
    ) -> Result<Self, CheckpointError> {
        if selection.len() > limits.max_variables {
            return Err(CheckpointError::Limit);
        }
        let ids: BTreeSet<_> = selection.iter().copied().collect();
        if ids.len() != selection.len() {
            return Err(CheckpointError::Selection);
        }
        let compatibility = identity::digest(document, registry, scope, &ids, limits)?;
        let mut values = Vec::new();
        let mut bytes = 0usize;
        for id in ids {
            let declaration = &document.variables()[&id];
            let shape = shape(&declaration.ty, registry)?;
            let value = get(id)?;
            bytes = bytes
                .checked_add(match value {
                    Some(v) => shape
                        .checked_charge(v, limits.max_value_bytes)
                        .map_err(|_| CheckpointError::Limit)?,
                    None => 16,
                })
                .filter(|n| *n <= limits.max_state_bytes)
                .ok_or(CheckpointError::Limit)?;
            let value = value
                .map(|v| {
                    literal_codec::encode(&declaration.ty, v, registry.types().as_ref())
                        .map_err(|_| CheckpointError::UnsupportedValue)
                })
                .transpose()?;
            values.push(Stored {
                id,
                ty: declaration.ty.clone(),
                value,
            });
        }
        let checkpoint = Self {
            document: Document {
                format: "orange.checkpoint".into(),
                format_version: 1,
                scope,
                graph_id: document.graph_id(),
                compatibility,
                values,
            },
            limits,
        };
        // Fail before publishing an object that cannot fit its declared wire bound.
        let bytes = checkpoint.to_bytes()?;
        // Publish only documents admitted by the same bounded reader used after
        // restart, including its nesting and structural checks.
        Self::from_bytes(&bytes, limits)
    }
    pub(crate) fn restore(
        &self,
        document: &GraphDocument,
        registry: &RegistrySnapshot,
        scope: CheckpointScope,
    ) -> Result<Values, CheckpointError> {
        if self.scope() != scope || self.graph_id() != document.graph_id() {
            return Err(CheckpointError::Owner);
        }
        let ids = self.document.values.iter().map(|v| v.id).collect();
        if identity::digest(document, registry, scope, &ids, self.limits)?
            != self.document.compatibility
        {
            return Err(CheckpointError::Incompatible);
        }
        let mut values = BTreeMap::new();
        let mut bytes = 0usize;
        for stored in &self.document.values {
            let declaration = &document.variables()[&stored.id];
            if stored.ty != declaration.ty {
                return Err(CheckpointError::Incompatible);
            }
            let shape = shape(&stored.ty, registry)?;
            let remaining = self
                .limits
                .max_value_bytes
                .min(self.limits.max_state_bytes.saturating_sub(bytes));
            let value = stored
                .value
                .as_ref()
                .map(|literal| {
                    literal_codec::decode_with_budget(literal, registry.types().as_ref(), remaining)
                        .map_err(|e| {
                            if matches!(e, orange_model::LiteralError::TooLarge { .. }) {
                                CheckpointError::Limit
                            } else {
                                CheckpointError::UnsupportedValue
                            }
                        })
                })
                .transpose()?;
            bytes = bytes
                .checked_add(match &value {
                    Some(v) => shape
                        .checked_charge(v, self.limits.max_value_bytes)
                        .map_err(|_| CheckpointError::UnsupportedValue)?,
                    None => 16,
                })
                .filter(|n| *n <= self.limits.max_state_bytes)
                .ok_or(CheckpointError::Limit)?;
            values.insert(stored.id, value);
        }
        Ok(values)
    }
}
fn shape(ty: &TypeExpr, registry: &RegistrySnapshot) -> Result<ValueShape, CheckpointError> {
    let term = orange_model::type_term::TypeTerm::parse(ty)
        .map_err(|_| CheckpointError::UnsupportedValue)?;
    if registry.types().literal_codec_identity(&term) != Some(literal_codec::CODEC_ID) {
        return Err(CheckpointError::UnsupportedValue);
    }
    let shape = ValueShape::resolve_in(&term, registry.types().as_ref())
        .ok_or(CheckpointError::UnsupportedValue)?;
    fn admit(s: &ValueShape) -> bool {
        match s {
            ValueShape::Handle(_)
            | ValueShape::Stream(_)
            | ValueShape::Error
            | ValueShape::Result(_) => false,
            ValueShape::List(s) | ValueShape::Option(s) => admit(s),
            ValueShape::Map(k, v) => admit(k) && admit(v),
            ValueShape::Record(r) => {
                (0..r.len()).all(|i| r.field(i).is_some_and(|(s, _)| admit(s)))
            }
            ValueShape::Enum(e) => (0..e.len()).all(|i| {
                u32::try_from(i)
                    .ok()
                    .and_then(|i| e.payload(i))
                    .is_some_and(|s| s.is_none_or(admit))
            }),
            ValueShape::Bool
            | ValueShape::I32
            | ValueShape::I64
            | ValueShape::U32
            | ValueShape::U64
            | ValueShape::F32
            | ValueShape::F64
            | ValueShape::Json
            | ValueShape::String
            | ValueShape::Duration
            | ValueShape::Timestamp
            | ValueShape::Bytes => true,
            _ => false,
        }
    }
    if admit(&shape) {
        Ok(shape)
    } else {
        Err(CheckpointError::UnsupportedValue)
    }
}
impl crate::Workspace {
    pub fn checkpoint(
        &self,
        document: &GraphDocument,
        selection: &[VariableId],
        limits: CheckpointLimits,
    ) -> Result<ValueCheckpoint, CheckpointError> {
        let snapshot = self.snapshot();
        snapshot
            .check_schema(document, snapshot.registry())
            .map_err(|_| CheckpointError::Incompatible)?;
        ValueCheckpoint::capture(
            document,
            snapshot.registry(),
            CheckpointScope::Workspace,
            selection,
            limits,
            |id| snapshot.get(id).map_err(|_| CheckpointError::Selection),
        )
    }
    /// Construct a fresh workspace. Existing actors/snapshots are never edited.
    pub fn from_checkpoint(
        document: &GraphDocument,
        registry: std::sync::Arc<RegistrySnapshot>,
        limits: crate::WorkspaceLimits,
        checkpoint: &ValueCheckpoint,
    ) -> Result<Self, CheckpointError> {
        let values = checkpoint.restore(document, &registry, CheckpointScope::Workspace)?;
        Self::with_values(document, registry, limits, Some(&values)).map_err(|error| {
            if error == crate::WorkspaceError::Limit {
                CheckpointError::Limit
            } else {
                CheckpointError::Startup
            }
        })
    }
}
