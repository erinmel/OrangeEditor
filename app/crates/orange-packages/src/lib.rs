//! Installed expansion packages: archives, manifests, lifecycle states and registry
//! transactions (04 "Package format and SDK", "Installation, activation and override",
//! "Hot update/removal"; ADR-07; 12 §8).
//!
//! [`read_archive`] verifies a canonical `.orangeplug` archive under resource quotas and
//! checks its content digest before anything uses its bytes. [`PackageManifest`] decodes and
//! validates a format 1 manifest, including its typed execution, panel and localization
//! sections. [`PackageStore`] installs, updates, removes, revokes and rolls back package
//! versions by publishing immutable registry generations, journaling each step through the
//! [`Journal`] seam; [`FileJournal`] is its durable implementation. Nothing here loads or
//! runs package code or edits a document. Permission grants, component instances, non-node
//! slots and explicit rebinds remain later work.
// Product code reports failures as typed errors (10, "Panic policy").
#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]
#![warn(missing_docs)]

mod archive;
mod file_journal;
mod journal;
mod json;
mod lifecycle;
mod manifest;
mod path;
mod sections;
mod store;

pub use archive::{
    ARCHIVE_MAGIC, ARCHIVE_VERSION, ArchiveEntry, ArchiveError, ArchiveFiles, ArchiveLimits,
    ArchiveSink, DirectorySink, DiscardSink, MANIFEST_PATH, MemorySink, VerifiedArchive,
    read_archive,
};
pub use file_journal::{
    FileJournal, JOURNAL_FILE, JOURNAL_TEMP_FILE, MAX_JOURNAL_RECORD_BYTES, OpenedJournal,
};
pub use journal::{Journal, JournalEntry, JournalError, MemoryJournal, Recovery};
pub use lifecycle::PackageState;
pub use manifest::{
    Dependency, HOST_API_VERSION, HostApiRange, HostApiVersion, MANIFEST_FORMAT,
    MANIFEST_FORMAT_VERSION, MAX_MANIFEST_BYTES, ManifestError, PackageId, PackageKey,
    PackageManifest, PackageVersion, ProviderContribution, ProviderSlot, SlotKind,
};
pub use path::{ArchivePath, MAX_PATH_BYTES, MAX_PATH_DEPTH, PathError, PathRule};
pub use sections::{
    ArgumentType, ComponentWorld, Execution, Localization, MAX_ARGUMENT_MESSAGES,
    MAX_CATALOG_FILES, MAX_LOCALES, MAX_MESSAGE_ARGUMENTS, MAX_PANELS, MAX_WORKER_TARGETS, PanelId,
    TargetArch, TargetOs, WORKER_WIRE_VERSION, WorkerTarget,
};
pub use store::{PackageContents, PackageRecord, PackageStore, RetentionPolicy, TransactionError};
