//! Editing documents (12 §3.3): atomic transactions, the change sets they report, and the
//! positional patches that make undo and redo exact.

mod change_set;
mod freshness;
mod patch;
mod transaction;

pub use change_set::{ChangeSet, DeclId, Delta};
pub(crate) use change_set::{EdgeEdit, LiteralEdit};
pub use freshness::PlanApplyError;
pub(crate) use freshness::Preconditions;
pub use transaction::{Committed, DocCommand, Inverse, LayoutCommand};
