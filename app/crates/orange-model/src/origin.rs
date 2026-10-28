//! Provenance of host-owned metadata: the identity of the catalogue a registry and its
//! messages come from. It sits below the catalog and validation layers, which both use it.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Validated immutable identity of the host catalogue bytes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct HostOrigin {
    kind: HostOriginKind,
    version: String,
    build_digest: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum HostOriginKind {
    Host,
}

impl HostOrigin {
    pub fn new(
        version: impl Into<String>,
        build_digest: impl Into<String>,
    ) -> Result<Self, OriginError> {
        let version = version.into();
        let build_digest = build_digest.into();
        if version.is_empty() || version.trim() != version || version.chars().any(char::is_control)
        {
            return Err(OriginError::InvalidVersion);
        }
        if !is_sha256(&build_digest) {
            return Err(OriginError::InvalidDigest);
        }
        Ok(Self {
            kind: HostOriginKind::Host,
            version,
            build_digest,
        })
    }

    pub fn version(&self) -> &str {
        &self.version
    }
    pub fn build_digest(&self) -> &str {
        &self.build_digest
    }
}

impl<'de> Deserialize<'de> for HostOrigin {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct WireOrigin {
            kind: HostOriginKind,
            version: String,
            build_digest: String,
        }

        let wire = WireOrigin::deserialize(deserializer)?;
        let mut origin =
            Self::new(wire.version, wire.build_digest).map_err(serde::de::Error::custom)?;
        origin.kind = wire.kind;
        Ok(origin)
    }
}

fn is_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum OriginError {
    #[error("host version must be non-empty, unpadded text without control characters")]
    InvalidVersion,
    #[error("host build digest must be sha256 followed by 64 lowercase hexadecimal digits")]
    InvalidDigest,
}
