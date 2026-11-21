use serde::{Deserialize, Serialize};

use super::ConverterRegistrationError;

/// Exact package implementation identity for a converter entry.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PackageProvider {
    pub package: String,
    pub version: String,
    pub content_digest: String,
    pub id: String,
    pub contract_version: u32,
}

impl PackageProvider {
    pub(super) fn validate(&self) -> Result<(), ConverterRegistrationError> {
        if [&self.package, &self.version, &self.id]
            .iter()
            .any(|value| value.trim().is_empty() || value.trim() != value.as_str())
            || self.contract_version == 0
        {
            return Err(ConverterRegistrationError::InvalidProviderIdentity);
        }
        let Some(hex) = self.content_digest.strip_prefix("sha256:") else {
            return Err(ConverterRegistrationError::InvalidProviderDigest {
                digest: self.content_digest.clone(),
            });
        };
        if hex.len() != 64
            || !hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(ConverterRegistrationError::InvalidProviderDigest {
                digest: self.content_digest.clone(),
            });
        }
        Ok(())
    }
}

/// An implementation alternative for a semantic entry.
///
/// `Core` is the language-owned implementation represented by a missing node provider.
/// Package alternatives carry exact artifact identity. The package host supplies the
/// authority argument to the builder; this module does not authenticate packages.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "provider", rename_all = "kebab-case")]
pub enum ConverterProvider {
    Core,
    Package(PackageProvider),
}
