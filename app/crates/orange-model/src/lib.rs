//! OrangeEditor graph document model.
//!
//! Contracts: `docs/12-graph-data-structures.md` (layout, JSON, transactions, values) and
//! `docs/11-graph-language.md` (semantics). No Bevy, rendering, async or I/O dependency.
//!
//! Layers, lowest first: `id`, `type_term` and `origin` (identifiers, declaration identity,
//! type syntax and host provenance); `document`
//! (persisted structures and the v1 JSON codec); `catalog` (host node contracts and provider
//! identity); `value` (runtime values and the typed literal codec); `edit` (transactions and
//! undo); `index`, `signature` and `paste` (derived lookups, semantic signatures and
//! copy/paste); `validation` (one read-only `validate` entry point over node, input,
//! connection and function passes).
// Product code reports failures as typed errors; each remaining panic is a documented
// internal invariant (10, "Panic policy").
#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]

pub mod catalog;
mod config_pointer;
pub mod conversion;
pub mod document;
pub mod edit;
pub mod error;
pub mod id;
pub mod index;
pub mod inference;
pub mod origin;
pub mod paste;
pub mod scalar;
pub mod signature;
pub mod type_term;
pub mod types;
pub mod validation;
pub mod value;
mod variable;

pub use document::codec::{ImportLimits, from_json, from_json_with_limits, to_json};
pub use document::{BodyRef, GraphDocument};
pub use edit::{ChangeSet, Committed, DeclId, Delta, DocCommand, Inverse, LayoutCommand};
pub use error::{
    CodecError, IdError, ImportResource, JsonTextError, LiteralError, PasteError, TxError,
};
pub use index::DocIndex;
pub use validation::{
    AnalysisLimits, AnalysisOptions, ValidationAnalysis, ValidationCheck, ValidationReport,
    analyze, analyze_with_effects, analyze_with_options, required_checks, validate,
    validate_with_effects,
};
pub use value::Value;
