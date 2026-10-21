//! OrangeEditor graph document model (WU-00/00b).
//!
//! Contracts: `docs/12-graph-data-structures.md` (layout, JSON, transactions, values) and
//! `docs/11-graph-language.md` (semantics). No Bevy, rendering, async or I/O dependency.
//!
//! Layers, lowest first: `id` and `type_term` (identifiers and type syntax); `document`
//! (persisted structures and the v1 JSON codec); `value` (runtime values and the typed
//! literal codec); `edit` (transactions and undo); `index`, `signature` and `paste`
//! (derived lookups, semantic signatures and copy/paste).

pub mod document;
pub mod edit;
pub mod error;
pub mod id;
pub mod index;
pub mod paste;
pub mod signature;
pub mod type_term;
pub mod value;

pub use document::codec::{from_json, to_json};
pub use document::{BodyRef, GraphDocument};
pub use edit::{ChangeSet, Committed, DeclId, Delta, DocCommand, Inverse, LayoutCommand};
pub use error::{CodecError, IdError, JsonTextError, LiteralError, PasteError, TxError};
pub use index::DocIndex;
pub use value::Value;
