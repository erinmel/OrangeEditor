//! Error types. Messages name the offending identifier or field, never document contents.

use thiserror::Error;

use crate::id::{
    EdgeId, EventDeclId, FunctionId, NodeId, PortId, PortKey, SignatureKey, TypeExpr, VariableId,
};

/// An identifier that violates its syntax rule or collides during generation (12 §2).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IdError {
    #[error("generated {kind} id `{id}` is already occupied")]
    GeneratedCollision { kind: &'static str, id: String },
    #[error("`{text}` is not a canonical lowercase hyphenated UUID")]
    NonCanonicalUuid { text: String },
    #[error("UUID `{text}` has version {version}; only v4 and v7 are allowed")]
    UnsupportedUuidVersion { text: String, version: usize },
    #[error("`{text}` is not a port id (expected [a-z][a-z0-9_]{{0,63}})")]
    InvalidPortId { text: String },
    #[error(
        "`{text}` is not a port key (expected `port_id` or `port_id#k` with canonical k <= 65535)"
    )]
    InvalidPortKey { text: String },
    #[error("`{text}` is not a definition id (expected `namespace/name`)")]
    InvalidDefinitionId { text: String },
    #[error("`{text}` is not a canonical type expression (11 §5.1)")]
    InvalidTypeExpr { text: String },
    #[error("`{text}` nests deeper than the type expression budget")]
    TypeTooDeep { text: String },
    #[error("{what} must not be empty or padded with whitespace")]
    Blank { what: &'static str },
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
    #[error("record literal has unknown field `{field}`")]
    UnknownField { field: String },
    #[error("enum literal has unknown variant `{variant}`")]
    UnknownVariant { variant: String },
    #[error("literal nests deeper than the codec budget")]
    TooDeep,
    #[error("literal for `{ty}` exceeds the codec size budget")]
    TooLarge { ty: String },
}

/// Failure to load or save a graph document.
#[derive(Debug, Error)]
pub enum CodecError {
    /// Malformed JSON, a missing/invalid field, or an invariant enforced while parsing
    /// (identifier syntax, duplicate IDs within one collection, unknown enum token).
    #[error("invalid graph document: {0}")]
    Json(#[from] serde_json::Error),
    #[error("unsupported document format `{found}` (expected `orange.graph`)")]
    UnsupportedFormat { found: String },
    #[error("unsupported format_version {found} (this build reads version 1)")]
    UnsupportedVersion { found: u32 },
    #[error("duplicate {kind} id `{id}` across graph bodies")]
    DuplicateId { kind: &'static str, id: String },
    /// A JSON object repeats a key, at any depth. `key` is the first repeat in document
    /// order, cut to 64 characters.
    #[error("duplicate JSON object key {key:?}")]
    DuplicateKey { key: String },
    #[error("duplicate {kind} lock identity `{identity}`")]
    DuplicateLock {
        kind: &'static str,
        identity: String,
    },
    #[error("graph import exceeds {resource} limit: {actual} > {max}")]
    ImportLimitExceeded {
        resource: ImportResource,
        max: usize,
        actual: usize,
    },
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
}

impl std::fmt::Display for ImportResource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Bytes => "byte",
            Self::Nodes => "node",
            Self::Edges => "edge",
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
