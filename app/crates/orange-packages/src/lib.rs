//! Installed expansion packages: manifests, lifecycle states and registry transactions
//! (04 "Installation, activation and override", "Hot update/removal"; ADR-07; 12 §8).
//!
//! Current profile: [`PackageManifest`] decodes and validates a format 1 manifest. It is
//! pure data: no I/O, code loading or document edits.
// Product code reports failures as typed errors (10, "Panic policy").
#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]
#![warn(missing_docs)]

mod manifest;

pub use manifest::{
    Dependency, HostApiRange, HostApiVersion, MANIFEST_FORMAT, MANIFEST_FORMAT_VERSION,
    MAX_MANIFEST_BYTES, ManifestError, PackageId, PackageKey, PackageManifest, PackageVersion,
    ProviderContribution, ProviderSlot, SlotKind,
};
