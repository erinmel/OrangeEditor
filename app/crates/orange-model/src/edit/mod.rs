//! Editing documents (12 §3.3): atomic transactions, the change sets they report, the
//! positional patches that make undo and redo exact, and explicit provider rebinds checked
//! on a copy (12 §8).

mod change_set;
mod freshness;
mod patch;
mod rebind;
mod transaction;

pub(crate) use change_set::NodeEdit;
pub use change_set::{ChangeSet, DeclId, Delta};
pub use freshness::PlanApplyError;
pub(crate) use freshness::Preconditions;
pub use rebind::{
    RebindError, RebindOutcome, RebindPlan, RebindRequest, RebindSelection, apply_rebind,
    plan_rebind,
};
pub use transaction::{Committed, DocCommand, Inverse, LayoutCommand};
