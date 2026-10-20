//! OrangeEditor graph document model (WU-00/00b).
//!
//! Contracts: `docs/12-graph-data-structures.md` (layout, JSON, transactions) and
//! `docs/11-graph-language.md` (semantics). No Bevy, rendering, async or I/O dependency.

pub mod change_set;
pub mod codec;
pub mod document;
pub mod error;
pub mod id;
pub mod keyed;
pub mod literal;
mod patch;
pub mod transaction;

pub use change_set::{ChangeSet, DeclId, Delta};
pub use codec::{from_json, to_json};
pub use document::{BodyRef, GraphDocument};
pub use error::{CodecError, IdError, LiteralError, TxError};
pub use transaction::{Committed, DocCommand, Inverse, LayoutCommand};
