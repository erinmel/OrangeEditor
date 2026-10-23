//! Provider identity.

use crate::GraphDocument;
use crate::document::ProviderBinding;

/// Exact implementation identity that specialization metadata belongs to: a node's
/// provider binding plus the document lock's digest for that package version. Two
/// providers of one definition can support different argument sets.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProviderKey {
    pub package: String,
    pub version: String,
    pub content_digest: String,
    pub id: String,
}

impl ProviderKey {
    /// The exact identity of a package binding, if the document locks its package version.
    pub fn of(document: &GraphDocument, binding: &ProviderBinding) -> Option<Self> {
        let mut entries = document
            .package_lock()
            .iter()
            .filter(|entry| entry.id == binding.package && entry.version == binding.version);
        let entry = entries.next()?;
        if entries.next().is_some() {
            return None;
        }
        Some(Self {
            package: binding.package.clone(),
            version: binding.version.clone(),
            content_digest: entry.content_digest.clone(),
            id: binding.id.clone(),
        })
    }
}
