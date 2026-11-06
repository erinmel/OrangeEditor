//! Typed manifest sections: `execution` (04 "Execution boundary decision"), the declarative
//! UI contribution list `contributions.panels` (ADR-08) and `localization` (13 §3).
//!
//! These checks are structural. Whether a component instantiates, a panel descriptor renders
//! or a catalogue parses is decided later, against the archive's bytes, and never runs code.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::str::FromStr;

use serde::Deserialize;
use serde_json::Value;

use crate::manifest::{ManifestError, PackageId, decode};
use crate::path::ArchivePath;

/// Most panels one package may contribute.
pub const MAX_PANELS: usize = 256;
/// Most catalogue locales one package may declare.
pub const MAX_LOCALES: usize = 256;
/// Most catalogue files one locale may list.
pub const MAX_CATALOG_FILES: usize = 64;
/// Most messages with declared arguments.
pub const MAX_ARGUMENT_MESSAGES: usize = 4_096;
/// Most declared arguments of one message.
pub const MAX_MESSAGE_ARGUMENTS: usize = 32;
/// Most worker targets.
pub const MAX_WORKER_TARGETS: usize = 16;

/// The only worker wire version this host speaks.
pub const WORKER_WIRE_VERSION: u32 = 1;

/// How the package's code runs. A data-only package has no `execution` section.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Execution {
    /// `wasm-component`: one component binary implementing an Orange expansion world.
    WasmComponent {
        /// The component binary inside the archive.
        entry: ArchivePath,
        /// The world it implements, such as `orange:expansion/plugin@1.0.0`.
        world: ComponentWorld,
    },
    /// `worker`: an explicitly trusted native worker per target (ADR-03). Never executed
    /// during validation.
    Worker {
        /// The worker IPC wire version.
        wire_version: u32,
        /// One executable per target, sorted by target.
        targets: Vec<WorkerTarget>,
    },
}

impl Execution {
    /// The section's `kind` as written.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::WasmComponent { .. } => "wasm-component",
            Self::Worker { .. } => "worker",
        }
    }

    /// Every archive file the section names as executable code.
    pub fn entries(&self) -> Vec<&ArchivePath> {
        match self {
            Self::WasmComponent { entry, .. } => vec![entry],
            Self::Worker { targets, .. } => targets.iter().map(|target| &target.entry).collect(),
        }
    }
}

/// Shows the kind and entry count only.
impl fmt::Debug for Execution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Execution")
            .field("kind", &self.kind())
            .field("entries", &self.entries().len())
            .finish()
    }
}

/// A component world in the `orange:expansion` package: `orange:expansion/<name>@<x.y.z>`.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ComponentWorld(String);

impl ComponentWorld {
    /// The world as written.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for ComponentWorld {
    type Err = ManifestError;

    fn from_str(text: &str) -> Result<Self, ManifestError> {
        let valid = text
            .strip_prefix("orange:expansion/")
            .and_then(|rest| rest.split_once('@'))
            .is_some_and(|(name, version)| {
                is_local_id(name)
                    && version.split('.').count() == 3
                    && version.split('.').all(is_number)
            });
        if valid && text.len() <= 128 {
            Ok(Self(text.to_owned()))
        } else {
            Err(ManifestError::ComponentWorld(text.into()))
        }
    }
}

impl fmt::Debug for ComponentWorld {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ComponentWorld").field(&self.0).finish()
    }
}

/// An operating system a worker binary targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TargetOs {
    /// Windows.
    Windows,
    /// macOS.
    Macos,
    /// Linux.
    Linux,
}

/// A processor architecture a worker binary targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetArch {
    /// 64-bit x86.
    #[serde(rename = "x86_64")]
    X86_64,
    /// 64-bit ARM.
    Aarch64,
}

/// One worker executable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerTarget {
    /// Target operating system.
    pub os: TargetOs,
    /// Target architecture.
    pub arch: TargetArch,
    /// The executable inside the archive.
    pub entry: ArchivePath,
}

/// A declarative panel contributed in the package's own namespace: `<package>/<local-id>`.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PanelId(String);

impl PanelId {
    /// The ID as written.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for PanelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("PanelId").field(&self.0).finish()
    }
}

/// A catalogue argument type (13 §3; encodings in 12 §6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ArgumentType {
    /// `string`.
    String,
    /// `i64`.
    I64,
    /// `u64`.
    U64,
    /// `f64`.
    F64,
    /// `timestamp`.
    Timestamp,
    /// `duration`.
    Duration,
}

impl FromStr for ArgumentType {
    type Err = ManifestError;

    fn from_str(text: &str) -> Result<Self, ManifestError> {
        Ok(match text {
            "string" => Self::String,
            "i64" => Self::I64,
            "u64" => Self::U64,
            "f64" => Self::F64,
            "timestamp" => Self::Timestamp,
            "duration" => Self::Duration,
            _ => return Err(ManifestError::ArgumentType(text.into())),
        })
    }
}

/// The validated `localization` section (13 §3): Fluent catalogues per locale, `en` as the
/// default, and the argument schema of each message that takes arguments.
#[derive(Clone, PartialEq, Eq)]
pub struct Localization {
    catalogs: BTreeMap<String, Vec<ArchivePath>>,
    arguments: BTreeMap<String, BTreeMap<String, ArgumentType>>,
}

impl Localization {
    /// The default locale; v1 requires `en`.
    pub fn default_locale(&self) -> &'static str {
        "en"
    }

    /// Catalogue files per locale tag as written, sorted by tag.
    pub fn catalogs(&self) -> &BTreeMap<String, Vec<ArchivePath>> {
        &self.catalogs
    }

    /// Declared arguments per local message ID.
    pub fn arguments(&self) -> &BTreeMap<String, BTreeMap<String, ArgumentType>> {
        &self.arguments
    }

    /// Every catalogue file.
    pub fn files(&self) -> impl Iterator<Item = &ArchivePath> {
        self.catalogs.values().flatten()
    }
}

/// Shows counts only.
impl fmt::Debug for Localization {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Localization")
            .field("locales", &self.catalogs.len())
            .field("files", &self.files().count())
            .field("argument_messages", &self.arguments.len())
            .finish()
    }
}

pub(crate) fn execution(value: Value) -> Result<Execution, ManifestError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    #[expect(dead_code, reason = "the kind is dispatched on before typed decoding")]
    struct Component {
        kind: String,
        entry: String,
        world: String,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    #[expect(dead_code, reason = "the kind is dispatched on before typed decoding")]
    struct Worker {
        kind: String,
        wire_version: u64,
        targets: Vec<WireTarget>,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct WireTarget {
        os: TargetOs,
        arch: TargetArch,
        entry: String,
    }

    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match kind {
        "wasm-component" => {
            let wire: Component = serde_json::from_value(value).map_err(decode)?;
            Ok(Execution::WasmComponent {
                entry: path(&wire.entry)?,
                world: wire.world.parse()?,
            })
        }
        "worker" => {
            let wire: Worker = serde_json::from_value(value).map_err(decode)?;
            if wire.wire_version != u64::from(WORKER_WIRE_VERSION) {
                return Err(ManifestError::WorkerWireVersion(wire.wire_version));
            }
            if wire.targets.is_empty() {
                return Err(ManifestError::WorkerTargets);
            }
            limit("execution.targets", wire.targets.len(), MAX_WORKER_TARGETS)?;
            let mut targets = Vec::with_capacity(wire.targets.len());
            for target in wire.targets {
                if targets
                    .iter()
                    .any(|seen: &WorkerTarget| (seen.os, seen.arch) == (target.os, target.arch))
                {
                    return Err(ManifestError::DuplicateWorkerTarget {
                        os: target.os,
                        arch: target.arch,
                    });
                }
                targets.push(WorkerTarget {
                    os: target.os,
                    arch: target.arch,
                    entry: path(&target.entry)?,
                });
            }
            targets.sort_by_key(|target| (target.os, target.arch));
            Ok(Execution::Worker {
                wire_version: WORKER_WIRE_VERSION,
                targets,
            })
        }
        other => Err(ManifestError::ExecutionKind(other.into())),
    }
}

pub(crate) fn panels(
    package: &PackageId,
    wire: Vec<String>,
) -> Result<Vec<PanelId>, ManifestError> {
    limit("contributions.panels", wire.len(), MAX_PANELS)?;
    let mut seen = BTreeSet::new();
    let mut panels = Vec::with_capacity(wire.len());
    for text in wire {
        let Some((namespace, local)) = text.split_once('/') else {
            return Err(ManifestError::PanelId(text.into()));
        };
        if !is_local_id(local) || !crate::manifest::is_namespace(namespace) {
            return Err(ManifestError::PanelId(text.as_str().into()));
        }
        if namespace != package.as_str() {
            return Err(ManifestError::PanelNamespace {
                panel: text,
                package: package.to_string(),
            });
        }
        if !seen.insert(text.clone()) {
            return Err(ManifestError::DuplicatePanel(text));
        }
        panels.push(PanelId(text));
    }
    Ok(panels)
}

pub(crate) fn localization(value: Value) -> Result<Localization, ManifestError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Wire {
        format_version: u64,
        format: String,
        default_locale: String,
        catalogs: BTreeMap<String, Vec<String>>,
        #[serde(default)]
        arguments: BTreeMap<String, BTreeMap<String, String>>,
    }

    let wire: Wire = serde_json::from_value(value).map_err(decode)?;
    if wire.format_version != 1 {
        return Err(ManifestError::LocalizationFormatVersion(
            wire.format_version,
        ));
    }
    if wire.format != "fluent" {
        return Err(ManifestError::LocalizationFormat(wire.format.into()));
    }
    if wire.default_locale != "en" {
        return Err(ManifestError::DefaultLocale(wire.default_locale.into()));
    }
    limit("localization.catalogs", wire.catalogs.len(), MAX_LOCALES)?;
    let mut tags = BTreeSet::new();
    let mut files = BTreeSet::new();
    let mut catalogs = BTreeMap::new();
    for (tag, list) in wire.catalogs {
        if !is_locale_tag(&tag) {
            return Err(ManifestError::LocaleTag(tag.into()));
        }
        if !tags.insert(tag.to_ascii_lowercase()) {
            return Err(ManifestError::DuplicateLocale(tag.into()));
        }
        if list.is_empty() {
            return Err(ManifestError::EmptyCatalog(tag.into()));
        }
        limit("localization.catalogs[]", list.len(), MAX_CATALOG_FILES)?;
        let mut paths = Vec::with_capacity(list.len());
        for text in list {
            let file = path(&text)?;
            if !file.has_extension("ftl") {
                return Err(ManifestError::CatalogFile(text.into()));
            }
            if !files.insert(file.normalized()) {
                return Err(ManifestError::DuplicateCatalogFile(text.into()));
            }
            paths.push(file);
        }
        catalogs.insert(tag, paths);
    }
    if !tags.contains("en") {
        return Err(ManifestError::MissingDefaultCatalog);
    }

    limit(
        "localization.arguments",
        wire.arguments.len(),
        MAX_ARGUMENT_MESSAGES,
    )?;
    let mut arguments = BTreeMap::new();
    for (message, declared) in wire.arguments {
        if !is_local_id(&message) {
            return Err(ManifestError::MessageId(message.into()));
        }
        limit(
            "localization.arguments{}",
            declared.len(),
            MAX_MESSAGE_ARGUMENTS,
        )?;
        let mut typed = BTreeMap::new();
        for (name, kind) in declared {
            if !is_argument_name(&name) {
                return Err(ManifestError::ArgumentName(name.into()));
            }
            typed.insert(name, kind.parse()?);
        }
        arguments.insert(message, typed);
    }
    Ok(Localization {
        catalogs,
        arguments,
    })
}

fn path(text: &str) -> Result<ArchivePath, ManifestError> {
    text.parse().map_err(ManifestError::Path)
}

fn limit(section: &'static str, count: usize, max: usize) -> Result<(), ManifestError> {
    if count > max {
        Err(ManifestError::SectionLimit {
            section,
            limit: max,
        })
    } else {
        Ok(())
    }
}

/// `[a-z][a-z0-9-]*`, at most 64 bytes: message, panel and world names (13 §2).
fn is_local_id(text: &str) -> bool {
    let mut bytes = text.bytes();
    text.len() <= 64
        && bytes.next().is_some_and(|first| first.is_ascii_lowercase())
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

/// `[a-z][a-z0-9_]*`, at most 64 bytes: Fluent argument names as the host declares them.
fn is_argument_name(text: &str) -> bool {
    let mut bytes = text.bytes();
    text.len() <= 64
        && bytes.next().is_some_and(|first| first.is_ascii_lowercase())
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

/// A BCP 47 tag shape: a 2–3 or 5–8 letter language subtag, then `-`-separated subtags of
/// 1–8 ASCII letters or digits; at most 35 bytes.
fn is_locale_tag(text: &str) -> bool {
    let mut subtags = text.split('-');
    let language = subtags.next().unwrap_or_default();
    text.len() <= 35
        && matches!(language.len(), 2 | 3 | 5..=8)
        && language.bytes().all(|byte| byte.is_ascii_alphabetic())
        && subtags.all(|subtag| {
            (1..=8).contains(&subtag.len())
                && subtag.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
}

fn is_number(text: &str) -> bool {
    !text.is_empty()
        && text.bytes().all(|byte| byte.is_ascii_digit())
        && (text == "0" || !text.starts_with('0'))
        && text.parse::<u32>().is_ok()
}
