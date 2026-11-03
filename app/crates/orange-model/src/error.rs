//! Error types. Messages name the offending identifier or field, never document contents.
//!
//! Text that failed a check came from outside, a document or a host string, and may hold
//! anything. Errors keep it as [`Rejected`], which never appears in `Display` or `Debug`
//! output; a host that chooses to show it calls [`Rejected::reveal`] (13 §2).

use std::fmt;

use thiserror::Error;

use crate::id::{
    EdgeId, EventDeclId, FunctionId, NodeId, PortId, PortKey, SignatureKey, TypeExpr, VariableId,
};

/// Input text an error refers to. Formatting shows only its length in characters.
#[derive(Clone, PartialEq, Eq)]
pub struct Rejected(String);

impl Rejected {
    /// The rejected text itself, for a host-controlled diagnostic path.
    pub fn reveal(&self) -> &str {
        &self.0
    }
}

impl From<&str> for Rejected {
    fn from(text: &str) -> Self {
        Self(text.to_owned())
    }
}

impl From<String> for Rejected {
    fn from(text: String) -> Self {
        Self(text)
    }
}

impl fmt::Debug for Rejected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Rejected({self})")
    }
}

impl fmt::Display for Rejected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<{} chars>", self.0.chars().count())
    }
}

/// An identifier that violates its syntax rule or collides during generation (12 §2).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IdError {
    #[error("generated {kind} id `{id}` is already occupied")]
    GeneratedCollision { kind: &'static str, id: String },
    #[error("value is not a canonical lowercase hyphenated UUID")]
    NonCanonicalUuid { text: Rejected },
    #[error("UUID has version {version}; only v4 and v7 are allowed")]
    UnsupportedUuidVersion { text: Rejected, version: usize },
    #[error("value is not a port id (expected [a-z][a-z0-9_]{{0,63}})")]
    InvalidPortId { text: Rejected },
    #[error(
        "value is not a port key (expected `port_id` or `port_id#k` with canonical k <= 65535)"
    )]
    InvalidPortKey { text: Rejected },
    #[error("value is not a definition id (expected `namespace/name`)")]
    InvalidDefinitionId { text: Rejected },
    #[error("value is not a canonical type expression (11 §5.1)")]
    InvalidTypeExpr { text: Rejected },
    #[error("type expression nests deeper than its budget")]
    TypeTooDeep { text: Rejected },
    #[error("{what} must not be empty or padded with whitespace")]
    Blank { what: &'static str },
}

impl IdError {
    /// Stable message key for host presentation (13 §2).
    pub const fn message_key(&self) -> &'static str {
        match self {
            Self::GeneratedCollision { .. } => "core/id-generated-collision",
            Self::NonCanonicalUuid { .. } => "core/id-non-canonical-uuid",
            Self::UnsupportedUuidVersion { .. } => "core/id-unsupported-uuid-version",
            Self::InvalidPortId { .. } => "core/id-invalid-port-id",
            Self::InvalidPortKey { .. } => "core/id-invalid-port-key",
            Self::InvalidDefinitionId { .. } => "core/id-invalid-definition-id",
            Self::InvalidTypeExpr { .. } => "core/id-invalid-type-expr",
            Self::TypeTooDeep { .. } => "core/id-type-too-deep",
            Self::Blank { .. } => "core/id-blank",
        }
    }
}

/// What kind of JSON failure an import hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonErrorKind {
    /// Not well-formed JSON.
    Syntax,
    /// Well-formed JSON that does not have the document's shape or violates an identifier
    /// or collection rule.
    Data,
    /// The text ended early.
    Eof,
    /// Reading or writing failed.
    Io,
}

impl fmt::Display for JsonErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Syntax => "syntax",
            Self::Data => "data",
            Self::Eof => "unexpected end",
            Self::Io => "I/O",
        })
    }
}

/// A JSON failure reduced to what is safe to show: its kind and position. The parser's own
/// message can quote document text, so it is kept only as [`Rejected`] detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonError {
    pub kind: JsonErrorKind,
    /// 1-based line, or 0 when no position applies.
    pub line: usize,
    /// 1-based column, or 0 when no position applies.
    pub column: usize,
    pub detail: Rejected,
}

impl fmt::Display for JsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} error at line {}, column {}",
            self.kind, self.line, self.column
        )
    }
}

impl From<serde_json::Error> for JsonError {
    fn from(error: serde_json::Error) -> Self {
        use serde_json::error::Category;
        Self {
            kind: match error.classify() {
                Category::Syntax => JsonErrorKind::Syntax,
                Category::Data => JsonErrorKind::Data,
                Category::Eof => JsonErrorKind::Eof,
                Category::Io => JsonErrorKind::Io,
            },
            line: error.line(),
            column: error.column(),
            detail: error.to_string().into(),
        }
    }
}

/// A literal that violates 12 §6.1, in its wire shape or for its type. `ty` names the type
/// at the failing position. The validator maps these to `E-LITERAL-TYPE`, except
/// `NoLiteralForm` (`E-HANDLE-LITERAL`).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LiteralError {
    #[error("literal needs exactly one of `value` or `bits`")]
    PayloadCount,
    #[error("literal `bits` must be 8 or 16 lowercase hex digits")]
    InvalidBits,
    #[error(transparent)]
    InvalidType(#[from] IdError),
    #[error("type `{ty}` has an unresolved type parameter")]
    UnresolvedParameter { ty: String },
    #[error("type `{ty}` has no literal form")]
    NoLiteralForm { ty: String },
    #[error("type `{ty}` is neither a core type nor a known record or enum")]
    UnknownType { ty: String },
    #[error("literal does not have the shape of `{ty}`")]
    TypeMismatch { ty: String },
    #[error("value does not match type `{ty}`")]
    ValueMismatch { ty: String },
    #[error("literal is out of range for `{ty}`")]
    OutOfRange { ty: String },
    #[error("literal for `{ty}` is not in canonical form")]
    NonCanonical { ty: String },
    #[error("`core/secret-ref@1` literals must be `credential://` references")]
    InvalidSecretRef,
    #[error("map key type `{ty}` is a float")]
    FloatMapKey { ty: String },
    #[error("map key type `{ty}` does not implement language `hash` and `eq`")]
    IneligibleMapKey { ty: String },
    #[error("map literal repeats a key")]
    DuplicateMapKey,
    #[error("record literal lacks required field `{field}`")]
    MissingField { field: String },
    #[error("record literal has an unknown field")]
    UnknownField { field: Rejected },
    #[error("enum literal has an unknown variant")]
    UnknownVariant { variant: Rejected },
    #[error("literal nests deeper than the codec budget")]
    TooDeep,
    #[error("literal for `{ty}` exceeds the codec size budget")]
    TooLarge { ty: String },
}

impl LiteralError {
    /// Stable message key for host presentation (13 §2).
    pub const fn message_key(&self) -> &'static str {
        match self {
            Self::PayloadCount => "core/literal-payload-count",
            Self::InvalidBits => "core/literal-invalid-bits",
            Self::InvalidType(error) => error.message_key(),
            Self::UnresolvedParameter { .. } => "core/literal-unresolved-parameter",
            Self::NoLiteralForm { .. } => "core/literal-no-literal-form",
            Self::UnknownType { .. } => "core/literal-unknown-type",
            Self::TypeMismatch { .. } => "core/literal-type-mismatch",
            Self::ValueMismatch { .. } => "core/literal-value-mismatch",
            Self::OutOfRange { .. } => "core/literal-out-of-range",
            Self::NonCanonical { .. } => "core/literal-non-canonical",
            Self::InvalidSecretRef => "core/literal-invalid-secret-ref",
            Self::FloatMapKey { .. } => "core/literal-float-map-key",
            Self::IneligibleMapKey { .. } => "core/literal-ineligible-map-key",
            Self::DuplicateMapKey => "core/literal-duplicate-map-key",
            Self::MissingField { .. } => "core/literal-missing-field",
            Self::UnknownField { .. } => "core/literal-unknown-field",
            Self::UnknownVariant { .. } => "core/literal-unknown-variant",
            Self::TooDeep => "core/literal-too-deep",
            Self::TooLarge { .. } => "core/literal-too-large",
        }
    }
}

/// Failure to load or save a graph document.
#[derive(Debug, Error)]
pub enum CodecError {
    /// Malformed JSON, a missing/invalid field, or an invariant enforced while parsing
    /// (identifier syntax, duplicate IDs within one collection, unknown enum token).
    #[error("invalid graph document: {0}")]
    Json(JsonError),
    #[error("unsupported document format (expected `orange.graph`)")]
    UnsupportedFormat { found: Rejected },
    #[error("unsupported format_version {found} (this build reads version 1)")]
    UnsupportedVersion { found: u32 },
    #[error("duplicate {kind} id `{id}` across graph bodies")]
    DuplicateId { kind: &'static str, id: String },
    /// A JSON object repeats a key, at any depth. `key` is the first repeat in document
    /// order, cut to 64 characters.
    #[error("duplicate JSON object key")]
    DuplicateKey { key: Rejected },
    #[error("duplicate {kind} lock identity")]
    DuplicateLock {
        kind: &'static str,
        identity: Rejected,
    },
    #[error("graph import exceeds {resource} limit: {actual} > {max}")]
    ImportLimitExceeded {
        resource: ImportResource,
        max: usize,
        actual: usize,
    },
    /// Objects and arrays nest deeper than the shared document budget.
    #[error("graph document nests JSON deeper than {max} levels")]
    NestingLimit { max: usize },
}

impl From<serde_json::Error> for CodecError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error.into())
    }
}

impl CodecError {
    /// Stable message key for host presentation (13 §2).
    pub const fn message_key(&self) -> &'static str {
        match self {
            Self::Json(_) => "core/import-json",
            Self::UnsupportedFormat { .. } => "core/import-unsupported-format",
            Self::UnsupportedVersion { .. } => "core/import-unsupported-version",
            Self::DuplicateId { .. } => "core/import-duplicate-id",
            Self::DuplicateKey { .. } => "core/import-duplicate-key",
            Self::DuplicateLock { .. } => "core/import-duplicate-lock",
            Self::ImportLimitExceeded { .. } => "core/import-limit-exceeded",
            Self::NestingLimit { .. } => "core/import-nesting-limit",
        }
    }
}

/// Resource dimension checked by the graph document importer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportResource {
    /// Raw UTF-8 document bytes, checked before any JSON parsing.
    Bytes,
    /// Nodes across the root and all function bodies, counted while parsing.
    Nodes,
    /// Edges across the root and all function bodies, counted while parsing.
    Edges,
    /// JSON values (containers and primitives, including the root), counted while scanning.
    JsonValues,
    /// Decoded UTF-8 bytes in all object keys, counted while scanning.
    JsonKeyBytes,
    /// Decoded UTF-8 bytes in all string values, counted while scanning.
    JsonStringBytes,
}

impl std::fmt::Display for ImportResource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Bytes => "byte",
            Self::Nodes => "node",
            Self::Edges => "edge",
            Self::JsonValues => "JSON value",
            Self::JsonKeyBytes => "JSON key byte",
            Self::JsonStringBytes => "JSON string byte",
        })
    }
}

/// A copy that cannot be made.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PasteError {
    #[error("node `{0}` does not exist")]
    NodeNotFound(NodeId),
    #[error("a copied selection must come from a single body")]
    SelectionSpansBodies,
}

/// A document transaction that was rejected. The document is unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TxError {
    #[error("variadic port `{port}` on node `{node}` has inconsistent counters")]
    InvalidVariadicState { node: NodeId, port: PortId },
    #[error("a transaction needs at least one command")]
    Empty,
    #[error("node `{0}` already exists")]
    NodeExists(NodeId),
    #[error("node `{0}` does not exist")]
    NodeNotFound(NodeId),
    #[error("edge `{0}` already exists")]
    EdgeExists(EdgeId),
    #[error("edge `{0}` does not exist")]
    EdgeNotFound(EdgeId),
    #[error("edge `{edge}` endpoint node `{node}` does not exist in the edge's body")]
    EdgeEndpointMissing { edge: EdgeId, node: NodeId },
    #[error("edge `{0}` would connect nodes in different bodies")]
    CrossBodyEdge(EdgeId),
    #[error("variable `{0}` does not exist")]
    VariableNotFound(VariableId),
    #[error("event `{0}` does not exist")]
    EventNotFound(EventDeclId),
    #[error("function `{0}` does not exist")]
    FunctionNotFound(FunctionId),
    #[error("function body node map key `{key}` does not match item id `{item}`")]
    FunctionNodeKeyMismatch { key: NodeId, item: NodeId },
    #[error("function body edge map key `{key}` does not match item id `{item}`")]
    FunctionEdgeKeyMismatch { key: EdgeId, item: EdgeId },
    #[error("variadic position {at} is past the {len} instances of `{port}` on node `{node}`")]
    VariadicPositionOutOfRange {
        node: NodeId,
        port: PortId,
        at: usize,
        len: usize,
    },
    #[error("variadic port `{port}` on node `{node}` has no unused instance counter left")]
    VariadicExhausted { node: NodeId, port: PortId },
    #[error("node `{node}` has no variadic instance port `{port}`")]
    VariadicPortNotFound { node: NodeId, port: PortKey },
    #[error("signature `{0}` already exists with different content")]
    SignatureConflict(SignatureKey),
    #[error("signature `{0}` does not exist")]
    SignatureNotFound(SignatureKey),
    #[error("package `{package}` {version} is already locked to a different content digest")]
    PackageLockConflict { package: String, version: String },
    #[error("type `{0}` is already locked to a different codec")]
    TypeLockConflict(TypeExpr),
    #[error("type `{0}` has no lock entry")]
    TypeLockNotFound(TypeExpr),
    #[error("export `{0}` does not exist")]
    ExportNotFound(String),
    #[error("the document revision counter is exhausted")]
    RevisionExhausted,
    #[error("a layout path needs at least one key")]
    EmptyLayoutPath,
    /// The edit would make the saved document nest deeper than the shared budget, so it
    /// could not be read back.
    #[error("the edit would nest the saved document deeper than {max} JSON levels")]
    NestingLimit { max: usize },
    #[error("layout path `{path}` passes through a value that is not an object")]
    LayoutPathConflict { path: String },
    #[error("inverse applies to content state {expected}, but the document is in state {actual}")]
    StaleInverse { expected: u64, actual: u64 },
}

/// JSON text rejected for `core/json@1` (03 codec budgets).
#[derive(Debug, Error)]
pub enum JsonTextError {
    #[error("JSON text of {len} bytes exceeds the {max}-byte budget")]
    TooLarge { len: usize, max: usize },
    #[error("JSON text nests deeper than the codec budget")]
    TooDeep,
    #[error("invalid JSON text: {0}")]
    Invalid(#[from] serde_json::Error),
}
