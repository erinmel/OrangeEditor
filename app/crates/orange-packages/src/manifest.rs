//! The package manifest: identity, exact version, content digest, host API range,
//! dependencies and contributed definitions/providers (04 "Package format and SDK").

use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

use orange_model::document::DefinitionRef;
use orange_model::id::DefinitionId;
use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;

/// The only manifest `format` this version reads.
pub const MANIFEST_FORMAT: &str = "orange.plugin";
/// The only manifest `format_version` this version reads.
pub const MANIFEST_FORMAT_VERSION: u32 = 1;
/// Largest manifest accepted, in bytes, before any JSON is decoded.
pub const MAX_MANIFEST_BYTES: usize = 1 << 20;
/// Longest accepted package ID, version, publisher or provider ID, in bytes.
const MAX_NAME_BYTES: usize = 128;

/// Why a manifest is rejected. Messages name the offending field, not payload contents.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ManifestError {
    /// The manifest is larger than [`MAX_MANIFEST_BYTES`].
    #[error("manifest is {bytes} bytes; the limit is {MAX_MANIFEST_BYTES}")]
    TooLarge {
        /// Size of the rejected input.
        bytes: usize,
    },
    /// The JSON is malformed, a field has the wrong shape or an unknown field is present.
    #[error("manifest is not a valid format 1 document: {reason}")]
    Decode {
        /// The decoder's description of the first problem.
        reason: String,
    },
    /// `format` is not [`MANIFEST_FORMAT`].
    #[error("unsupported manifest format `{0}`")]
    Format(String),
    /// `format_version` is not [`MANIFEST_FORMAT_VERSION`].
    #[error("unsupported manifest format version {0}")]
    FormatVersion(u64),
    /// The package ID is not a dotted lowercase namespace.
    #[error("invalid package id `{0}`")]
    PackageId(String),
    /// The version is not an exact `MAJOR.MINOR.PATCH[-PRE]` version.
    #[error("invalid exact version `{0}`")]
    Version(String),
    /// The publisher is not a dotted lowercase namespace.
    #[error("invalid publisher `{0}`")]
    Publisher(String),
    /// The package ID lies outside its publisher's namespace.
    #[error("package `{package}` is outside publisher namespace `{publisher}`")]
    PublisherNamespace {
        /// The claimed package ID.
        package: String,
        /// The declared publisher.
        publisher: String,
    },
    /// `host_api.min_minor` is greater than `host_api.max_minor`.
    #[error("host API range {major}.{min_minor}..={major}.{max_minor} is empty")]
    HostApiRange {
        /// Required host API major version.
        major: u32,
        /// Lowest accepted minor version.
        min_minor: u32,
        /// Highest accepted minor version.
        max_minor: u32,
    },
    /// The integrity algorithm is not `sha256`.
    #[error("unsupported integrity algorithm `{0}`")]
    DigestAlgorithm(String),
    /// The payload digest is not 64 lowercase hexadecimal digits.
    #[error("payload digest must be 64 lowercase hexadecimal digits")]
    Digest,
    /// A dependency names the package itself.
    #[error("package depends on itself")]
    SelfDependency,
    /// Two dependencies name one package.
    #[error("package `{0}` is listed as a dependency twice")]
    DuplicateDependency(String),
    /// A contributed node ID is not a valid definition ID.
    #[error("invalid node definition id `{0}`")]
    NodeId(String),
    /// A contributed node lies outside the package's own namespace.
    #[error("node `{node}` is outside package namespace `{package}`")]
    NodeNamespace {
        /// The contributed node definition ID.
        node: String,
        /// The package ID that owns the namespace.
        package: String,
    },
    /// A node definition ID is listed twice.
    #[error("node `{0}` is listed twice")]
    DuplicateNode(String),
    /// A provider slot is malformed or names an unknown slot kind.
    #[error("invalid provider slot `{0}`")]
    Slot(String),
    /// A provider ID is empty or uses characters outside `[a-z0-9._-]`.
    #[error("invalid provider id `{0}`")]
    ProviderId(String),
    /// One provider ID is declared twice for one slot.
    #[error("provider `{id}` is declared twice for slot `{slot}`")]
    DuplicateProvider {
        /// The slot as written.
        slot: String,
        /// The repeated provider ID.
        id: String,
    },
}

/// A package ID: a dotted namespace of lowercase identifiers, such as `org.example.device`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PackageId(String);

impl PackageId {
    /// The ID as written in the manifest.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether definitions in `namespace` belong to this package: only its own ID does.
    fn owns(&self, namespace: &str) -> bool {
        namespace == self.0
    }
}

impl FromStr for PackageId {
    type Err = ManifestError;

    fn from_str(text: &str) -> Result<Self, ManifestError> {
        if is_namespace(text) {
            Ok(Self(text.to_owned()))
        } else {
            Err(ManifestError::PackageId(text.to_owned()))
        }
    }
}

impl fmt::Display for PackageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// An exact package version, `MAJOR.MINOR.PATCH` with an optional `-PRERELEASE`. Ranges,
/// wildcards and build metadata are rejected: a package version names one immutable build.
/// Ordering is textual, for use as a key; it is not semver precedence.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PackageVersion(String);

impl PackageVersion {
    /// The version as written in the manifest.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for PackageVersion {
    type Err = ManifestError;

    fn from_str(text: &str) -> Result<Self, ManifestError> {
        let (core, pre) = match text.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (text, None),
        };
        let mut parts = core.split('.');
        let numeric = parts
            .by_ref()
            .take(3)
            .filter(|part| is_number(part))
            .count()
            == 3
            && parts.next().is_none();
        let pre_ok = pre.is_none_or(|pre| {
            pre.split('.').all(|ident| {
                !ident.is_empty()
                    && ident
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            })
        });
        if numeric && pre_ok && text.len() <= MAX_NAME_BYTES {
            Ok(Self(text.to_owned()))
        } else {
            Err(ManifestError::Version(text.to_owned()))
        }
    }
}

impl fmt::Display for PackageVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One exact package version: the identity a document lock and the store key on.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PackageKey {
    /// The package ID.
    pub id: PackageId,
    /// The exact version.
    pub version: PackageVersion,
}

impl fmt::Display for PackageKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.id, self.version)
    }
}

/// The running host's public API version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostApiVersion {
    /// Major version; packages must require exactly this major.
    pub major: u32,
    /// Minor version; must fall inside a package's declared minor range.
    pub minor: u32,
}

/// The host API this foundation build offers packages. Hosts pass it to
/// [`PackageStore::new`](crate::PackageStore::new), which accepts a manifest only when its
/// [`HostApiRange`] contains it. A minor release adds API; a major release breaks it.
pub const HOST_API_VERSION: HostApiVersion = HostApiVersion { major: 1, minor: 0 };

/// The host API versions a package supports: one major, an inclusive minor range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostApiRange {
    /// Required host API major version.
    pub major: u32,
    /// Lowest accepted minor version.
    pub min_minor: u32,
    /// Highest accepted minor version.
    pub max_minor: u32,
}

impl HostApiRange {
    /// Whether `host` has this major and a minor inside the range.
    pub fn accepts(&self, host: HostApiVersion) -> bool {
        host.major == self.major && (self.min_minor..=self.max_minor).contains(&host.minor)
    }
}

/// An exact dependency on another package version.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Dependency {
    /// The package depended on.
    pub id: PackageId,
    /// Its exact version.
    pub version: PackageVersion,
}

/// Kinds of replaceable slot other than node providers (04 "Installation, activation and
/// override"). The current registry snapshot publishes node providers only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SlotKind {
    /// `convert:<from>-><to>`.
    Convert,
    /// `connection:<adapter>@<contract>`.
    Connection,
    /// `audio:<slot>@<contract>`.
    Audio,
    /// `editor:<slot>@<contract>`.
    Editor,
}

/// A replaceable slot a provider fills.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProviderSlot {
    /// `node:<definition>@<version>`: an implementation of one definition version.
    Node(DefinitionRef),
    /// A non-node slot, kept with its descriptor as written after the kind prefix.
    Other {
        /// The slot kind.
        kind: SlotKind,
        /// Everything after `<kind>:`.
        descriptor: String,
    },
}

impl FromStr for ProviderSlot {
    type Err = ManifestError;

    fn from_str(text: &str) -> Result<Self, ManifestError> {
        let invalid = || ManifestError::Slot(text.to_owned());
        let (kind, descriptor) = text.split_once(':').ok_or_else(invalid)?;
        if descriptor.is_empty()
            || descriptor.len() > MAX_NAME_BYTES * 2
            || descriptor
                .chars()
                .any(|c| c.is_whitespace() || c.is_control())
        {
            return Err(invalid());
        }
        let kind = match kind {
            "node" => {
                let (id, version) = descriptor.rsplit_once('@').ok_or_else(invalid)?;
                if !is_number(version) {
                    return Err(invalid());
                }
                let id: DefinitionId = id.parse().map_err(|_| invalid())?;
                let version = version.parse().map_err(|_| invalid())?;
                return Ok(Self::Node(DefinitionRef { id, version }));
            }
            "convert" => SlotKind::Convert,
            "connection" => SlotKind::Connection,
            "audio" => SlotKind::Audio,
            "editor" => SlotKind::Editor,
            _ => return Err(invalid()),
        };
        Ok(Self::Other {
            kind,
            descriptor: descriptor.to_owned(),
        })
    }
}

/// One provider the package contributes: a slot and the package-local provider ID.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProviderContribution {
    /// The slot it fills.
    pub slot: ProviderSlot,
    /// Package-local provider ID; one ID may fill several slots.
    pub id: String,
}

/// A validated package manifest. Only [`PackageManifest::from_json`] constructs one, so every
/// value satisfies the format 1 rules.
///
/// `execution`, `permissions`, `localization`, `panels` and `license` are retained as
/// written but not interpreted yet; granting authority and loading code are later work.
#[derive(Debug, Clone, PartialEq)]
pub struct PackageManifest {
    key: PackageKey,
    publisher: PackageId,
    host_api: HostApiRange,
    content_digest: String,
    dependencies: Vec<Dependency>,
    nodes: Vec<DefinitionId>,
    providers: Vec<ProviderContribution>,
    opaque: Opaque,
}

/// Manifest sections retained but not interpreted by this version.
#[derive(Debug, Clone, PartialEq, Default)]
struct Opaque {
    execution: Option<Value>,
    permissions: Option<Value>,
    localization: Option<Value>,
    panels: Option<Value>,
    license: Option<String>,
}

impl PackageManifest {
    /// Decodes and validates a format 1 manifest. Unknown fields are rejected so a misspelt
    /// field cannot silently drop a declaration.
    pub fn from_json(bytes: &[u8]) -> Result<Self, ManifestError> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(ManifestError::TooLarge { bytes: bytes.len() });
        }
        let value: Value = serde_json::from_slice(bytes).map_err(decode)?;
        let format = value.get("format").and_then(Value::as_str);
        if format != Some(MANIFEST_FORMAT) {
            return Err(ManifestError::Format(format.unwrap_or_default().to_owned()));
        }
        let format_version = value.get("format_version").and_then(Value::as_u64);
        if format_version != Some(u64::from(MANIFEST_FORMAT_VERSION)) {
            return Err(ManifestError::FormatVersion(
                format_version.unwrap_or_default(),
            ));
        }
        let wire: WireManifest = serde_json::from_value(value).map_err(decode)?;
        wire.validate()
    }

    /// The package ID and exact version.
    pub fn key(&self) -> &PackageKey {
        &self.key
    }

    /// The publisher namespace that owns the package ID.
    pub fn publisher(&self) -> &PackageId {
        &self.publisher
    }

    /// Supported host API versions.
    pub fn host_api(&self) -> HostApiRange {
        self.host_api
    }

    /// The payload digest in lock form, `sha256:<64 hex>`, as document locks record it.
    pub fn content_digest(&self) -> &str {
        &self.content_digest
    }

    /// Exact dependencies, in manifest order.
    pub fn dependencies(&self) -> &[Dependency] {
        &self.dependencies
    }

    /// Node definition IDs the package defines, all in its own namespace.
    pub fn nodes(&self) -> &[DefinitionId] {
        &self.nodes
    }

    /// Providers the package contributes, in manifest order.
    pub fn providers(&self) -> &[ProviderContribution] {
        &self.providers
    }

    /// The declared license expression, if any.
    pub fn license(&self) -> Option<&str> {
        self.opaque.license.as_deref()
    }

    /// The uninterpreted `execution` section.
    pub fn execution(&self) -> Option<&Value> {
        self.opaque.execution.as_ref()
    }

    /// The uninterpreted `permissions` section. Nothing is granted from it yet.
    pub fn permissions(&self) -> Option<&Value> {
        self.opaque.permissions.as_ref()
    }

    /// The uninterpreted `localization` section (13 §3).
    pub fn localization(&self) -> Option<&Value> {
        self.opaque.localization.as_ref()
    }

    /// The uninterpreted `contributions.panels` section.
    pub fn panels(&self) -> Option<&Value> {
        self.opaque.panels.as_ref()
    }
}

fn decode(error: serde_json::Error) -> ManifestError {
    ManifestError::Decode {
        reason: error.to_string(),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[expect(dead_code, reason = "format fields are checked before typed decoding")]
struct WireManifest {
    format: String,
    format_version: u32,
    id: String,
    version: String,
    publisher: String,
    host_api: HostApiRange,
    #[serde(default)]
    localization: Option<Value>,
    #[serde(default)]
    execution: Option<Value>,
    #[serde(default)]
    dependencies: Vec<WireDependency>,
    #[serde(default)]
    contributions: WireContributions,
    #[serde(default)]
    permissions: Option<Value>,
    integrity: WireIntegrity,
    #[serde(default)]
    license: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireDependency {
    id: String,
    version: String,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireContributions {
    #[serde(default)]
    nodes: Vec<String>,
    #[serde(default)]
    panels: Option<Value>,
    #[serde(default)]
    providers: Vec<WireProvider>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireProvider {
    slot: String,
    id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireIntegrity {
    algorithm: String,
    payload_digest: String,
}

impl WireManifest {
    fn validate(self) -> Result<PackageManifest, ManifestError> {
        let id: PackageId = self.id.parse()?;
        let version: PackageVersion = self.version.parse()?;
        let publisher: PackageId = self
            .publisher
            .parse()
            .map_err(|_| ManifestError::Publisher(self.publisher.clone()))?;
        let within_publisher = id.as_str() == publisher.as_str()
            || id
                .as_str()
                .strip_prefix(publisher.as_str())
                .is_some_and(|rest| rest.starts_with('.'));
        if !within_publisher {
            return Err(ManifestError::PublisherNamespace {
                package: id.to_string(),
                publisher: publisher.to_string(),
            });
        }
        let host_api = self.host_api;
        if host_api.min_minor > host_api.max_minor {
            return Err(ManifestError::HostApiRange {
                major: host_api.major,
                min_minor: host_api.min_minor,
                max_minor: host_api.max_minor,
            });
        }
        if self.integrity.algorithm != "sha256" {
            return Err(ManifestError::DigestAlgorithm(self.integrity.algorithm));
        }
        let digest = &self.integrity.payload_digest;
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(ManifestError::Digest);
        }

        let mut seen = BTreeSet::new();
        let mut dependencies = Vec::with_capacity(self.dependencies.len());
        for dependency in self.dependencies {
            let dependency = Dependency {
                id: dependency.id.parse()?,
                version: dependency.version.parse()?,
            };
            if dependency.id == id {
                return Err(ManifestError::SelfDependency);
            }
            if !seen.insert(dependency.id.clone()) {
                return Err(ManifestError::DuplicateDependency(
                    dependency.id.to_string(),
                ));
            }
            dependencies.push(dependency);
        }

        let mut seen = BTreeSet::new();
        let mut nodes = Vec::with_capacity(self.contributions.nodes.len());
        for node in self.contributions.nodes {
            let parsed: DefinitionId = node
                .parse()
                .map_err(|_| ManifestError::NodeId(node.clone()))?;
            if !id.owns(parsed.namespace()) {
                return Err(ManifestError::NodeNamespace {
                    node,
                    package: id.to_string(),
                });
            }
            if !seen.insert(parsed.clone()) {
                return Err(ManifestError::DuplicateNode(node));
            }
            nodes.push(parsed);
        }

        let mut seen = BTreeSet::new();
        let mut providers = Vec::with_capacity(self.contributions.providers.len());
        for provider in self.contributions.providers {
            let slot: ProviderSlot = provider.slot.parse()?;
            if !is_provider_id(&provider.id) {
                return Err(ManifestError::ProviderId(provider.id));
            }
            let contribution = ProviderContribution {
                slot,
                id: provider.id,
            };
            if !seen.insert(contribution.clone()) {
                return Err(ManifestError::DuplicateProvider {
                    slot: provider.slot,
                    id: contribution.id,
                });
            }
            providers.push(contribution);
        }

        Ok(PackageManifest {
            content_digest: format!("sha256:{digest}"),
            key: PackageKey { id, version },
            publisher,
            host_api,
            dependencies,
            nodes,
            providers,
            opaque: Opaque {
                execution: self.execution,
                permissions: self.permissions,
                localization: self.localization,
                panels: self.contributions.panels,
                license: self.license,
            },
        })
    }
}

/// `ident { "." ident }` with `ident = [a-z][a-z0-9_]*` (11 §5.1 namespaces).
fn is_namespace(text: &str) -> bool {
    text.len() <= MAX_NAME_BYTES
        && text.split('.').all(|ident| {
            let mut bytes = ident.bytes();
            bytes.next().is_some_and(|first| first.is_ascii_lowercase())
                && bytes
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        })
}

/// Decimal digits without a leading zero, fitting `u32`.
fn is_number(text: &str) -> bool {
    !text.is_empty()
        && text.bytes().all(|byte| byte.is_ascii_digit())
        && (text == "0" || !text.starts_with('0'))
        && text.parse::<u32>().is_ok()
}

fn is_provider_id(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= MAX_NAME_BYTES
        && text.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
}
