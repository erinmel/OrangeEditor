//! OrangeEditor graph document model (WU-00/00b).
//!
//! Contracts: `docs/12-graph-data-structures.md` (layout, JSON) and
//! `docs/11-graph-language.md` (semantics). No Bevy, rendering, async or I/O dependency.

pub mod codec;
pub mod document;
pub mod error;
pub mod id;
pub mod keyed;
pub mod literal;

pub use codec::{from_json, to_json};
pub use document::GraphDocument;
pub use error::{CodecError, IdError, LiteralError};
