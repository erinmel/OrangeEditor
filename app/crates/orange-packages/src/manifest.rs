//! The package manifest: identity, exact version, content digest, host API range,
//! dependencies and contributed definitions/providers (04 "Package format and SDK").

use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

use orange_model::document::DefinitionRef;
use orange_model::error::{JsonError, Rejected};
use orange_model::id::DefinitionId;
use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;

use crate::json::{self, Problem};
use crate::path::ArchivePath;
use crate::sections::{self, Execution, Localization, PanelId};

/// The only manifest `format` this version reads.
pub const MANIFEST_FORMAT: &str = "orange.plugin";
/// The only manifest `format_version` this version reads.
pub const MANIFEST_FORMAT_VERSION: u32 = 1;
/// Largest manifest accepted, in bytes, before any JSON is decoded.
pub const MAX_MANIFEST_BYTES: usize = 1 << 20;
/// Longest accepted package ID, version, publisher or provider ID, in bytes.
const MAX_NAME_BYTES: usize = 128;

/// Why a manifest is rejected. Messages name the offending field, not payload contents:
/// text that failed a check is held as [`Rejected`], which formats as its length only, and
/// a host that chooses to show it calls [`Rejected::reveal`] (13 §2). Hosts present a
/// failure through [`message_key`](Self::message_key), never through `Display`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ManifestError {
    /// The manifest is larger than [`MAX_MANIFEST_BYTES`].
    #[error("manifest is {bytes} bytes; the limit is {MAX_MANIFEST_BYTES}")]
    TooLarge {
        /// Size of the rejected input.
        bytes: usize,
    },
    /// The JSON is malformed, a field has the wrong shape or an unknown field is present.
    #[error("manifest is not a valid format 1 document: {error}")]
    Decode {
        /// The decoder failure reduced to its kind and position; the decoder's own message,
        /// which can quote input or name unknown fields, is only `Rejected` detail.
        error: JsonError,
    },
    /// `format` is not [`MANIFEST_FORMAT`].
    #[error("unsupported manifest format {0}")]
    Format(Rejected),
    /// `format_version` is not [`MANIFEST_FORMAT_VERSION`].
    #[error("unsupported manifest format version {0}")]
    FormatVersion(u64),
    /// The package ID is not a dotted lowercase namespace.
    #[error("invalid package id {0}")]
    PackageId(Rejected),
    /// The version is not an exact `MAJOR.MINOR.PATCH[-PRE]` version.
    #[error("invalid exact version {0}")]
    Version(Rejected),
    /// The publisher is not a dotted lowercase namespace.
    #[error("invalid publisher {0}")]
    Publisher(Rejected),
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
    #[error("unsupported integrity algorithm {0}")]
    DigestAlgorithm(Rejected),
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
    #[error("invalid node definition id {0}")]
    NodeId(Rejected),
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
    #[error("invalid provider slot {0}")]
    Slot(Rejected),
    /// A provider ID is empty or uses characters outside `[a-z0-9._-]`.
    #[error("invalid provider id {0}")]
    ProviderId(Rejected),
    /// One provider ID is declared twice for one slot.
    #[error("provider `{id}` is declared twice for slot {slot}")]
    DuplicateProvider {
        /// The slot as written.
        slot: Rejected,
        /// The repeated provider ID.
        id: String,
    },
    /// An object repeats a key; readers that keep the first or the last would disagree.
    #[error("manifest repeats key {0}")]
    DuplicateKey(Rejected),
    /// A number is not an integer within ±(2^53 − 1).
    #[error("manifest numbers must be integers within 2^53")]
    Number,
    /// A dependency lock digest is not `sha256:` and 64 lowercase hexadecimal digits.
    #[error("invalid dependency digest {0}")]
    DependencyDigest(Rejected),
    /// `execution.kind` is not `wasm-component` or `worker`. Native libraries are refused.
    #[error("unsupported execution kind {0}")]
    ExecutionKind(Rejected),
    /// The component world is not `orange:expansion/<name>@<x.y.z>`.
    #[error("invalid component world {0}")]
    ComponentWorld(Rejected),
    /// The worker wire version is not [`WORKER_WIRE_VERSION`](crate::WORKER_WIRE_VERSION).
    #[error("unsupported worker wire version {0}")]
    WorkerWireVersion(u64),
    /// A worker section lists no targets.
    #[error("worker execution lists no targets")]
    WorkerTargets,
    /// Two worker targets share an operating system and architecture.
    #[error("worker target {os:?}/{arch:?} is listed twice")]
    DuplicateWorkerTarget {
        /// Target operating system.
        os: crate::TargetOs,
        /// Target architecture.
        arch: crate::TargetArch,
    },
    /// A manifest reference to an archive file is not a valid archive path.
    #[error(transparent)]
    Path(#[from] crate::PathError),
    /// A panel ID is not `<package>/<local-id>`.
    #[error("invalid panel id {0}")]
    PanelId(Rejected),
    /// A panel lies outside the package's own namespace.
    #[error("panel `{panel}` is outside package namespace `{package}`")]
    PanelNamespace {
        /// The panel ID.
        panel: String,
        /// The package ID that owns the namespace.
        package: String,
    },
    /// A panel is listed twice.
    #[error("panel `{0}` is listed twice")]
    DuplicatePanel(String),
    /// `localization.format_version` is not 1.
    #[error("unsupported localization format version {0}")]
    LocalizationFormatVersion(u64),
    /// `localization.format` is not `fluent`.
    #[error("unsupported localization format {0}")]
    LocalizationFormat(Rejected),
    /// `localization.default_locale` is not `en`.
    #[error("default locale {0} is not `en`")]
    DefaultLocale(Rejected),
    /// A catalogue key is not a BCP 47 tag shape.
    #[error("invalid locale tag {0}")]
    LocaleTag(Rejected),
    /// Two catalogue tags are equal ignoring ASCII case.
    #[error("locale {0} is listed twice")]
    DuplicateLocale(Rejected),
    /// No catalogue for the default locale `en`.
    #[error("no catalogue for the default locale")]
    MissingDefaultCatalog,
    /// A locale lists no catalogue files.
    #[error("locale {0} lists no catalogue files")]
    EmptyCatalog(Rejected),
    /// A catalogue file is not an `.ftl` file.
    #[error("catalogue file {0} is not .ftl")]
    CatalogFile(Rejected),
    /// One catalogue file is listed twice, possibly under two locales.
    #[error("catalogue file {0} is listed twice")]
    DuplicateCatalogFile(Rejected),
    /// An argument schema names a message ID outside `[a-z][a-z0-9-]*`.
    #[error("invalid message id {0}")]
    MessageId(Rejected),
    /// An argument name is outside `[a-z][a-z0-9_]*`.
    #[error("invalid argument name {0}")]
    ArgumentName(Rejected),
    /// An argument type is not one of the 13 §3 vocabulary.
    #[error("invalid argument type {0}")]
    ArgumentType(Rejected),
    /// A section lists more items than the format allows.
    #[error("{section} exceeds {limit} items")]
    SectionLimit {
        /// The section, as a manifest path.
        section: &'static str,
        /// The format's limit.
        limit: usize,
    },
}

impl ManifestError {
    /// Stable message key for host presentation (13 §2).
    pub const fn message_key(&self) -> &'static str {
        match self {
            Self::TooLarge { .. } => "packages/manifest-too-large",
            Self::Decode { .. } => "packages/manifest-decode",
            Self::Format(_) => "packages/manifest-format",
            Self::FormatVersion(_) => "packages/manifest-format-version",
            Self::PackageId(_) => "packages/manifest-package-id",
            Self::Version(_) => "packages/manifest-version",
            Self::Publisher(_) => "packages/manifest-publisher",
            Self::PublisherNamespace { .. } => "packages/manifest-publisher-namespace",
            Self::HostApiRange { .. } => "packages/manifest-host-api-range",
            Self::DigestAlgorithm(_) => "packages/manifest-digest-algorithm",
            Self::Digest => "packages/manifest-digest",
            Self::SelfDependency => "packages/manifest-self-dependency",
            Self::DuplicateDependency(_) => "packages/manifest-duplicate-dependency",
            Self::NodeId(_) => "packages/manifest-node-id",
            Self::NodeNamespace { .. } => "packages/manifest-node-namespace",
            Self::DuplicateNode(_) => "packages/manifest-duplicate-node",
            Self::Slot(_) => "packages/manifest-slot",
            Self::ProviderId(_) => "packages/manifest-provider-id",
            Self::DuplicateProvider { .. } => "packages/manifest-duplicate-provider",
            Self::DuplicateKey(_) => "packages/manifest-duplicate-key",
            Self::Number => "packages/manifest-number",
            Self::DependencyDigest(_) => "packages/manifest-dependency-digest",
            Self::ExecutionKind(_) => "packages/manifest-execution-kind",
            Self::ComponentWorld(_) => "packages/manifest-component-world",
            Self::WorkerWireVersion(_) => "packages/manifest-worker-wire-version",
            Self::WorkerTargets => "packages/manifest-worker-targets",
            Self::DuplicateWorkerTarget { .. } => "packages/manifest-duplicate-worker-target",
            Self::Path(_) => "packages/manifest-path",
            Self::PanelId(_) => "packages/manifest-panel-id",
            Self::PanelNamespace { .. } => "packages/manifest-panel-namespace",
            Self::DuplicatePanel(_) => "packages/manifest-duplicate-panel",
            Self::LocalizationFormatVersion(_) => "packages/manifest-localization-format-version",
            Self::LocalizationFormat(_) => "packages/manifest-localization-format",
            Self::DefaultLocale(_) => "packages/manifest-default-locale",
            Self::LocaleTag(_) => "packages/manifest-locale-tag",
            Self::DuplicateLocale(_) => "packages/manifest-duplicate-locale",
            Self::MissingDefaultCatalog => "packages/manifest-missing-default-catalog",
            Self::EmptyCatalog(_) => "packages/manifest-empty-catalog",
            Self::CatalogFile(_) => "packages/manifest-catalog-file",
            Self::DuplicateCatalogFile(_) => "packages/manifest-duplicate-catalog-file",
            Self::MessageId(_) => "packages/manifest-message-id",
            Self::ArgumentName(_) => "packages/manifest-argument-name",
            Self::ArgumentType(_) => "packages/manifest-argument-type",
            Self::SectionLimit { .. } => "packages/manifest-section-limit",
        }
    }
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
            Err(ManifestError::PackageId(text.into()))
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
            Err(ManifestError::Version(text.into()))
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

/// An exact dependency lock: another package version and its content digest. The store
/// refuses to install against an active version with any other digest, and the lock is part
/// of the dependent's own content digest, so neither side can be swapped silently.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Dependency {
    /// The package depended on.
    pub id: PackageId,
    /// Its exact version.
    pub version: PackageVersion,
    /// Its content digest in lock form, `sha256:<64 hex>`.
    pub content_digest: String,
}

impl Dependency {
    /// The exact version depended on.
    pub fn key(&self) -> PackageKey {
        PackageKey {
            id: self.id.clone(),
            version: self.version.clone(),
        }
    }
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
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
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
        let invalid = || ManifestError::Slot(text.into());
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
/// `execution`, `contributions.panels` and `localization` are typed and enforced
/// ([`Execution`], [`PanelId`], [`Localization`]). `permissions` and `license` are retained as
/// written but not interpreted; granting authority is later work (K03).
#[derive(Clone, PartialEq)]
pub struct PackageManifest {
    key: PackageKey,
    publisher: PackageId,
    host_api: HostApiRange,
    content_digest: String,
    dependencies: Vec<Dependency>,
    nodes: Vec<DefinitionId>,
    providers: Vec<ProviderContribution>,
    execution: Option<Execution>,
    panels: Vec<PanelId>,
    localization: Option<Localization>,
    permissions: Option<Value>,
    license: Option<String>,
}

/// Shows the slot kind and the descriptor's length only: a non-node descriptor is
/// uninterpreted manifest text (13 §2). [`ProviderSlot::Other`] fields are public for
/// authorized inspection.
impl fmt::Debug for ProviderSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Node(definition) => f.debug_tuple("Node").field(definition).finish(),
            Self::Other { kind, descriptor } => f
                .debug_struct("Other")
                .field("kind", kind)
                .field("descriptor", &Rejected::from(descriptor.as_str()))
                .finish(),
        }
    }
}

/// A bounded summary: identity, host API range, counts, section shapes and the presence of
/// each uninterpreted section, never its content. Read content through the accessors.
impl fmt::Debug for PackageManifest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let permissions = match &self.permissions {
            None => "absent".to_owned(),
            Some(Value::Array(items)) => format!("array({} items)", items.len()),
            Some(Value::Object(fields)) => format!("object({} fields)", fields.len()),
            Some(_) => "scalar".to_owned(),
        };
        let license = self.license.as_deref().map_or_else(
            || "absent".to_owned(),
            |text| Rejected::from(text).to_string(),
        );
        f.debug_struct("PackageManifest")
            .field("key", &self.key)
            .field("publisher", &self.publisher)
            .field("host_api", &self.host_api)
            .field("dependencies", &self.dependencies.len())
            .field("nodes", &self.nodes.len())
            .field("providers", &self.providers.len())
            .field("execution", &self.execution)
            .field("panels", &self.panels.len())
            .field("localization", &self.localization)
            .field("permissions", &format_args!("{permissions}"))
            .field("license", &format_args!("{license}"))
            .finish_non_exhaustive()
    }
}

impl PackageManifest {
    /// Decodes and validates a format 1 manifest. Unknown fields, duplicate object keys and
    /// non-integer numbers are rejected so every reader sees the same declarations.
    pub fn from_json(bytes: &[u8]) -> Result<Self, ManifestError> {
        Self::decode(bytes).map(|(manifest, _)| manifest)
    }

    /// Decodes a manifest and returns it with its canonical body: the canonical JSON of the
    /// manifest object without its `integrity` member, which the content digest covers.
    pub(crate) fn decode(bytes: &[u8]) -> Result<(Self, Vec<u8>), ManifestError> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(ManifestError::TooLarge { bytes: bytes.len() });
        }
        let mut value = match json::parse(bytes).map_err(decode)? {
            Ok(value) => value,
            Err(Problem::DuplicateKey(key)) => return Err(ManifestError::DuplicateKey(key.into())),
            Err(Problem::Number) => return Err(ManifestError::Number),
        };
        let format = value.get("format").and_then(Value::as_str);
        if format != Some(MANIFEST_FORMAT) {
            return Err(ManifestError::Format(format.unwrap_or_default().into()));
        }
        let format_version = value.get("format_version").and_then(Value::as_u64);
        if format_version != Some(u64::from(MANIFEST_FORMAT_VERSION)) {
            return Err(ManifestError::FormatVersion(
                format_version.unwrap_or_default(),
            ));
        }
        let integrity = value
            .as_object_mut()
            .and_then(|members| members.remove("integrity"));
        let mut body = Vec::with_capacity(bytes.len());
        json::write_canonical(&value, &mut body);
        if let (Some(integrity), Some(members)) = (integrity, value.as_object_mut()) {
            members.insert("integrity".to_owned(), integrity);
        }
        let wire: WireManifest = serde_json::from_value(value).map_err(decode)?;
        Ok((wire.validate()?, body))
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

    /// The declared content digest in lock form, `sha256:<64 hex>`, as document and
    /// dependency locks record it. It covers the canonical manifest body and every other
    /// archive file; [`read_archive`](crate::read_archive) verifies it.
    pub fn content_digest(&self) -> &str {
        &self.content_digest
    }

    /// Exact dependency locks, in manifest order.
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

    /// How the package's code runs; `None` for a data-only package.
    pub fn execution(&self) -> Option<&Execution> {
        self.execution.as_ref()
    }

    /// Declarative panels the package contributes, in manifest order.
    pub fn panels(&self) -> &[PanelId] {
        &self.panels
    }

    /// The localization section (13 §3).
    pub fn localization(&self) -> Option<&Localization> {
        self.localization.as_ref()
    }

    /// The uninterpreted `permissions` section. Nothing is granted from it yet.
    pub fn permissions(&self) -> Option<&Value> {
        self.permissions.as_ref()
    }

    /// The declared license expression, if any.
    pub fn license(&self) -> Option<&str> {
        self.license.as_deref()
    }

    /// Every archive file the manifest names: execution entries and catalogue files.
    pub fn referenced_files(&self) -> impl Iterator<Item = &ArchivePath> {
        self.execution
            .iter()
            .flat_map(Execution::entries)
            .chain(self.localization.iter().flat_map(Localization::files))
    }
}

pub(crate) fn decode(error: serde_json::Error) -> ManifestError {
    ManifestError::Decode {
        error: error.into(),
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
    content_digest: String,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireContributions {
    #[serde(default)]
    nodes: Vec<String>,
    #[serde(default)]
    panels: Vec<String>,
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
    content_digest: String,
}

impl WireManifest {
    fn validate(self) -> Result<PackageManifest, ManifestError> {
        let id: PackageId = self.id.parse()?;
        let version: PackageVersion = self.version.parse()?;
        let publisher: PackageId = self
            .publisher
            .parse()
            .map_err(|_| ManifestError::Publisher(self.publisher.as_str().into()))?;
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
            return Err(ManifestError::DigestAlgorithm(
                self.integrity.algorithm.into(),
            ));
        }
        let digest = &self.integrity.content_digest;
        if !is_hex_digest(digest) {
            return Err(ManifestError::Digest);
        }

        let mut seen = BTreeSet::new();
        let mut dependencies = Vec::with_capacity(self.dependencies.len());
        for dependency in self.dependencies {
            let valid_digest = dependency
                .content_digest
                .strip_prefix("sha256:")
                .is_some_and(is_hex_digest);
            let parsed = Dependency {
                id: dependency.id.parse()?,
                version: dependency.version.parse()?,
                content_digest: dependency.content_digest,
            };
            if !valid_digest {
                return Err(ManifestError::DependencyDigest(
                    parsed.content_digest.into(),
                ));
            }
            if parsed.id == id {
                return Err(ManifestError::SelfDependency);
            }
            if !seen.insert(parsed.id.clone()) {
                return Err(ManifestError::DuplicateDependency(parsed.id.to_string()));
            }
            dependencies.push(parsed);
        }

        let mut seen = BTreeSet::new();
        let mut nodes = Vec::with_capacity(self.contributions.nodes.len());
        for node in self.contributions.nodes {
            let parsed: DefinitionId = node
                .parse()
                .map_err(|_| ManifestError::NodeId(node.as_str().into()))?;
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
                return Err(ManifestError::ProviderId(provider.id.into()));
            }
            let contribution = ProviderContribution {
                slot,
                id: provider.id,
            };
            if !seen.insert(contribution.clone()) {
                return Err(ManifestError::DuplicateProvider {
                    slot: provider.slot.into(),
                    id: contribution.id,
                });
            }
            providers.push(contribution);
        }

        let execution = self.execution.map(sections::execution).transpose()?;
        let panels = sections::panels(&id, self.contributions.panels)?;
        let localization = self.localization.map(sections::localization).transpose()?;

        Ok(PackageManifest {
            content_digest: format!("sha256:{digest}"),
            key: PackageKey { id, version },
            publisher,
            host_api,
            dependencies,
            nodes,
            providers,
            execution,
            panels,
            localization,
            permissions: self.permissions,
            license: self.license,
        })
    }
}

/// 64 lowercase hexadecimal digits.
fn is_hex_digest(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// `ident { "." ident }` with `ident = [a-z][a-z0-9_]*` (11 §5.1 namespaces).
pub(crate) fn is_namespace(text: &str) -> bool {
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
