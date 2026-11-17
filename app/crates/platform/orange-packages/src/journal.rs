//! The crash-journal seam (04 "Hot update/removal"): the store writes each lifecycle step and
//! each committed generation before publishing it, so a restart can select the last fully
//! committed package set, keep removals and revocations, and report half-staged versions
//! without executing them.

use std::collections::{BTreeMap, BTreeSet};

use orange_model::catalog::ProviderKey;
use orange_model::document::DefinitionRef;
use thiserror::Error;

use crate::lifecycle::PackageState;
use crate::manifest::PackageKey;

/// One journal record.
///
/// The enum is `#[non_exhaustive]`. Hosts persist the current checkpoint shape, including
/// explicit absent provider selections; unreleased earlier shapes have no compatibility
/// reader in this crate.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum JournalEntry {
    /// A package version entered `state`. Retirements are written before the commit that
    /// publishes them, activations after it.
    Transition {
        /// The package version.
        package: PackageKey,
        /// The state it entered.
        state: PackageState,
    },
    /// A registry generation became current. Each commit is a full checkpoint of the
    /// store's durable identity state. Written before the generation is published; a failed
    /// write publishes nothing.
    Committed {
        /// The process-local registry generation that became current.
        generation: u64,
        /// Every active package version in that generation, sorted.
        active: Vec<PackageKey>,
        /// Every version ever published, with its content digest. Rollback never shrinks
        /// it, so a version keeps its digest for good.
        published: BTreeMap<PackageKey, String>,
        /// Every revoked content digest.
        revoked: BTreeSet<String>,
        /// The exact new-binding selection state of every controlled definition. A value
        /// of `Some(provider)` selects that provider; `None` explicitly clears the choice,
        /// including one supplied by the base registry. Definitions absent from this map
        /// retain their base policy. Controlled keys survive retirement and rollback;
        /// recovery applies them after all referenced providers exist, never install order.
        defaults: BTreeMap<DefinitionRef, Option<ProviderKey>>,
    },
}

/// A journal write that did not become durable.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("package journal write failed: {reason}")]
pub struct JournalError {
    /// Host description of the failure.
    pub reason: String,
}

/// Durable, append-only lifecycle journal supplied by the host. The seam is inert;
/// [`crate::FileJournal`] provides an explicit filesystem-backed implementation.
pub trait Journal {
    /// Appends `entry`. An error means the entry is not durable and the store abandons the
    /// step that needed it.
    fn append(&mut self, entry: JournalEntry) -> Result<(), JournalError>;

    /// Replaces every earlier entry with `checkpoint`, a complete
    /// [`JournalEntry::Committed`] of the store's current state, so a durable journal stays
    /// bounded. Must be atomic: after an error, or a crash, the journal holds either all its
    /// earlier entries or exactly `checkpoint`, and both recover the same committed set,
    /// published digests and revocations. The default does nothing, which suits journals
    /// that never compact.
    fn compact(&mut self, checkpoint: JournalEntry) -> Result<(), JournalError> {
        let _ = checkpoint;
        Ok(())
    }
}

/// An in-memory journal for tests and hosts without persistence. It never fails.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryJournal {
    entries: Vec<JournalEntry>,
}

impl MemoryJournal {
    /// An empty journal.
    pub fn new() -> Self {
        Self::default()
    }

    /// Every entry, oldest first.
    pub fn entries(&self) -> &[JournalEntry] {
        &self.entries
    }
}

impl Journal for MemoryJournal {
    fn append(&mut self, entry: JournalEntry) -> Result<(), JournalError> {
        self.entries.push(entry);
        Ok(())
    }

    fn compact(&mut self, checkpoint: JournalEntry) -> Result<(), JournalError> {
        self.entries = vec![checkpoint];
        Ok(())
    }
}

/// What a restart learns from a journal; [`PackageStore::restore`](crate::PackageStore::restore)
/// rebuilds a store from it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Recovery {
    /// Active package versions of the last committed generation, or `None` if nothing was
    /// ever committed.
    pub committed: Option<Vec<PackageKey>>,
    /// Every version ever published, with its content digest.
    pub published: BTreeMap<PackageKey, String>,
    /// Published versions that are neither active nor revoked: their tombstones.
    pub removed: Vec<PackageKey>,
    /// Revoked content digests.
    pub revoked: BTreeSet<String>,
    /// Versions whose last step is an unfinished installation step and that the last commit
    /// does not include: their staged files are stale and must never be executed.
    pub abandoned: Vec<PackageKey>,
    /// The authoritative selection state of controlled definitions in the last commit, or
    /// an empty map if nothing was committed. Map values of `None` explicitly clear a
    /// default; absent keys retain their base policy. See [`JournalEntry::Committed`].
    pub defaults: BTreeMap<DefinitionRef, Option<ProviderKey>>,
}

impl Recovery {
    /// Replays `entries`, oldest first.
    pub fn from_entries(entries: &[JournalEntry]) -> Self {
        let mut checkpoint = None;
        let mut last: BTreeMap<&PackageKey, PackageState> = BTreeMap::new();
        for entry in entries {
            match entry {
                JournalEntry::Transition { package, state } => {
                    last.insert(package, *state);
                }
                JournalEntry::Committed {
                    active,
                    published,
                    revoked,
                    defaults,
                    ..
                } => checkpoint = Some((active, published, revoked, defaults)),
            }
        }
        let Some((active, published, revoked, defaults)) = checkpoint else {
            return Self {
                abandoned: last
                    .into_iter()
                    .filter(|(_, state)| state.is_pending())
                    .map(|(package, _)| package.clone())
                    .collect(),
                ..Self::default()
            };
        };
        let abandoned = last
            .into_iter()
            .filter(|(package, state)| state.is_pending() && !active.contains(package))
            .map(|(package, _)| package.clone())
            .collect();
        let removed = published
            .iter()
            .filter(|(key, digest)| !active.contains(key) && !revoked.contains(*digest))
            .map(|(key, _)| key.clone())
            .collect();
        Self {
            committed: Some(active.clone()),
            published: published.clone(),
            removed,
            revoked: revoked.clone(),
            abandoned,
            defaults: defaults.clone(),
        }
    }
}
