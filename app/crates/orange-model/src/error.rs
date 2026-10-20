//! Error types. Messages name the offending identifier or field, never document contents.

use thiserror::Error;

/// A textual identifier that violates its syntax rule (12 §2).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IdError {
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
    #[error("{what} must not be empty or padded with whitespace")]
    Blank { what: &'static str },
}

/// A literal whose wire shape violates 12 §6.1.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LiteralError {
    #[error("literal needs exactly one of `value` or `bits`")]
    PayloadCount,
    #[error("literal `bits` must be 8 or 16 lowercase hex digits")]
    InvalidBits,
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
}
