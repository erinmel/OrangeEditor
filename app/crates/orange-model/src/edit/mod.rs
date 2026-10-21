//! Editing documents (12 §3.3): atomic transactions, the change sets they report, and the
//! positional patches that make undo and redo exact.

mod change_set;
mod patch;
mod transaction;

pub use change_set::{ChangeSet, DeclId, Delta};
pub use transaction::{Committed, DocCommand, Inverse, LayoutCommand};
