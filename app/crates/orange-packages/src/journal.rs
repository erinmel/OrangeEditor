//! The crash-journal seam (04 "Hot update/removal"): the store writes each lifecycle step and
//! each committed generation before publishing it, so a restart can select the last fully
//! committed package set and report half-staged versions without executing them.

use std::collections::BTreeMap;

use thiserror::Error;

use crate::lifecycle::PackageState;
use crate::manifest::PackageKey;

/// One journal record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JournalEntry {
    /// A package version entered `state`.
    Transition {
        /// The package version.
        package: PackageKey,
        /// The state it entered.
        state: PackageState,
    },
    /// A registry generation became current with exactly these active package versions.
    /// Written before the generation is published; a failed write publishes nothing.
    Committed {
        /// The process-local registry generation that became current.
        generation: u64,
        /// Every active package version in that generation, sorted.
        active: Vec<PackageKey>,
    },
}

/// A journal write that did not become durable.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("package journal write failed: {reason}")]
pub struct JournalError {
    /// Host description of the failure.
    pub reason: String,
}

/// Durable, append-only lifecycle journal supplied by the host. File-backed storage is host
/// work; this crate performs no I/O.
pub trait Journal {
    /// Appends `entry`. An error means the entry is not durable and the store abandons the
    /// step that needed it.
    fn append(&mut self, entry: JournalEntry) -> Result<(), JournalError>;
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
}

/// What a restart learns from a journal.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Recovery {
    /// Active package versions of the last committed generation, or `None` if nothing was
    /// ever committed.
    pub committed: Option<Vec<PackageKey>>,
    /// Versions whose last step is an unfinished installation step and that the last commit
    /// does not include: their staged files are stale and must never be executed.
    pub abandoned: Vec<PackageKey>,
}

impl Recovery {
    /// Replays `entries`, oldest first.
    pub fn from_entries(entries: &[JournalEntry]) -> Self {
        let mut committed: Option<&[PackageKey]> = None;
        let mut last: BTreeMap<&PackageKey, PackageState> = BTreeMap::new();
        for entry in entries {
            match entry {
                JournalEntry::Transition { package, state } => {
                    last.insert(package, *state);
                }
                JournalEntry::Committed { active, .. } => committed = Some(active),
            }
        }
        let abandoned = last
            .into_iter()
            .filter(|(package, state)| {
                state.is_pending() && !committed.is_some_and(|active| active.contains(package))
            })
            .map(|(package, _)| package.clone())
            .collect();
        Self {
            committed: committed.map(<[PackageKey]>::to_vec),
            abandoned,
        }
    }
}
