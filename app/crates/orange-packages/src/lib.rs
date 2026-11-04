//! Installed expansion packages: manifests, lifecycle states and registry transactions
//! (04 "Installation, activation and override", "Hot update/removal"; ADR-07; 12 §8).
//!
//! Current profile: pure data and state. [`PackageManifest`] decodes and validates a
//! format 1 manifest. [`PackageStore`] installs, updates, removes, revokes and rolls back
//! package versions by publishing immutable registry generations, journaling each step
//! through the [`Journal`] seam. It performs no I/O, loads no code and never edits a
//! document. Archive handling, permission grants, component instances, non-node slots and
//! explicit rebinds remain later work.
// Product code reports failures as typed errors (10, "Panic policy").
#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]
#![warn(missing_docs)]

mod journal;
mod lifecycle;
mod manifest;
mod store;

pub use journal::{Journal, JournalEntry, JournalError, MemoryJournal, Recovery};
pub use lifecycle::PackageState;
pub use manifest::{
    Dependency, HostApiRange, HostApiVersion, MANIFEST_FORMAT, MANIFEST_FORMAT_VERSION,
    MAX_MANIFEST_BYTES, ManifestError, PackageId, PackageKey, PackageManifest, PackageVersion,
    ProviderContribution, ProviderSlot, SlotKind,
};
pub use store::{PackageContents, PackageRecord, PackageStore, TransactionError};
