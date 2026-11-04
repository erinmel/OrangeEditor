//! Package lifecycle states (04 "Installation, activation and override", "Hot update/removal").

/// Where one package version is in its lifecycle.
///
/// An installation attempt advances Discovered → Staged → Validated → Granted → Activating →
/// Active; any pre-active step may fail to Quarantined, which is terminal for that attempt.
/// An active version is Removed (a tombstone) or Revoked; a removed version may also be
/// revoked. Reinstalling a removed version starts a new attempt at Discovered. A rollback is
/// not a lifecycle step: it may return a removed version to Active, or leave a version whose
/// installation it undoes as a Removed tombstone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PackageState {
    /// The manifest was read; nothing is checked yet.
    Discovered,
    /// The immutable contents are held by the store.
    Staged,
    /// Manifest, compatibility, dependency and contribution checks passed.
    Validated,
    /// Requested authority was approved. This version does not interpret permissions; the
    /// caller's install request stands for approval.
    Granted,
    /// The next registry generation is being built and journaled.
    Activating,
    /// Published in the current registry generation.
    Active,
    /// The attempt failed; the previous registry generation stayed current.
    Quarantined,
    /// Hidden from new bindings with a tombstone; existing locks resolve as unavailable.
    Removed,
    /// Removed by policy: its digest cannot be reinstalled and rollback cannot restore it.
    Revoked,
}

impl PackageState {
    /// Whether the lifecycle permits moving from `self` to `next`.
    pub fn can_become(self, next: Self) -> bool {
        use PackageState::*;
        matches!(
            (self, next),
            (Discovered, Staged)
                | (Staged, Validated)
                | (Validated, Granted)
                | (Granted, Activating)
                | (Activating, Active)
                | (
                    Discovered | Staged | Validated | Granted | Activating,
                    Quarantined
                )
                | (Active, Removed | Revoked)
                | (Removed, Revoked)
        )
    }

    /// Whether the version is published in the registry generation that holds this state.
    pub fn is_active(self) -> bool {
        self == Self::Active
    }

    /// Whether this is an unfinished installation step that a crash could leave behind.
    pub fn is_pending(self) -> bool {
        matches!(
            self,
            Self::Discovered | Self::Staged | Self::Validated | Self::Granted | Self::Activating
        )
    }
}
