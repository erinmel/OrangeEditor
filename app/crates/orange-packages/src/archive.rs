//! The canonical `.orangeplug` archive (04 "Package format and SDK"): one byte encoding per
//! set of files, content digests, and bounded verification that rejects a package before any
//! of its code could run.
//!
//! Layout, all integers little-endian:
//!
//! ```text
//! header  magic "\x89OPKG\r\n\x1a" | version u16 = 1 | flags u16 = 0 | entries u32
//! entry   path_len u16 | path | kind u8 = 0 (file) | mode u8 = 0 | size u64 | size bytes
//! ```
//!
//! The first entry is `manifest.json`; the rest follow in strictly ascending byte order of
//! their paths, and nothing follows the last entry. There is no compression, so unpacked
//! bytes never exceed archive bytes, and deflate's many valid encodings cannot make two
//! readers disagree. Directories are implied by paths. Kinds other than a regular file
//! (directories, symbolic and hard links) and nonzero modes are rejected: executable bits and
//! links are never package-controlled.
//!
//! The content digest, which `integrity.content_digest` declares and every lock records, is
//!
//! ```text
//! SHA-256( "orange.plugin/content/v1" 0x00 | u64 len(M) | M | u32 n |
//!          n x ( u16 len(path) | path | u64 size | SHA-256(bytes) ) )
//! ```
//!
//! over the archive's other entries in archive order, where `M` is the canonical JSON of the
//! manifest without its `integrity` member (see `json`). It therefore covers every byte of
//! metadata and payload, including dependency locks, but not the digest field itself.

use std::collections::BTreeSet;
use std::fmt;
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use orange_model::error::Rejected;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::manifest::{MAX_MANIFEST_BYTES, ManifestError, PackageManifest};
use crate::path::{ArchivePath, MAX_PATH_BYTES, PathError};
use crate::sections::Execution;
use crate::store::PackageContents;

/// The archive magic: a high byte, the format name, and line-ending bytes that text-mode
/// transfer would corrupt.
pub const ARCHIVE_MAGIC: [u8; 8] = *b"\x89OPKG\r\n\x1a";
/// The only archive layout version this reader accepts.
pub const ARCHIVE_VERSION: u16 = 1;
/// The path of the manifest, always the first entry.
pub const MANIFEST_PATH: &str = "manifest.json";
/// Domain separator of the content digest.
const CONTENT_DOMAIN: &[u8] = b"orange.plugin/content/v1\0";
/// Bytes read per step; the reader's only per-file buffer.
const CHUNK: usize = 64 * 1024;
/// Entry kind of a regular file, the only kind accepted.
const KIND_FILE: u8 = 0;

/// Resource quotas for one archive read. Every quota is checked against declared sizes
/// before any byte of that entry is read, and reading never allocates by a declared size.
/// Work is linear in the bytes read, so the byte quotas also bound CPU time.
///
/// Defaults are unmeasured starting points, not qualified product limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchiveLimits {
    /// Most archive bytes read, header included.
    pub max_archive_bytes: u64,
    /// Most entries, the manifest included.
    pub max_entries: u32,
    /// Largest single file.
    pub max_file_bytes: u64,
    /// Most file bytes written to the sink: the disk quota of a staging directory.
    pub max_total_bytes: u64,
}

impl Default for ArchiveLimits {
    fn default() -> Self {
        Self {
            max_archive_bytes: 256 << 20,
            max_entries: 4_096,
            max_file_bytes: 128 << 20,
            max_total_bytes: 256 << 20,
        }
    }
}

/// Why an archive was rejected. Paths are held as [`Rejected`] text; digests are hex.
#[derive(Debug, Clone, PartialEq, Error)]
#[non_exhaustive]
pub enum ArchiveError {
    /// Reading the archive failed.
    #[error("archive read failed: {0:?}")]
    Read(io::ErrorKind),
    /// The archive ended inside the header or an entry.
    #[error("archive is truncated")]
    Truncated,
    /// The input does not start with [`ARCHIVE_MAGIC`].
    #[error("not an Orange package archive")]
    Magic,
    /// The layout version is not [`ARCHIVE_VERSION`].
    #[error("unsupported archive version {0}")]
    Version(u16),
    /// Reserved header flags are set; v1 defines none (no compression, no encryption).
    #[error("unsupported archive flags {0:#06x}")]
    Flags(u16),
    /// The archive is longer than [`ArchiveLimits::max_archive_bytes`].
    #[error("archive exceeds {limit} bytes")]
    ArchiveTooLarge {
        /// The quota.
        limit: u64,
    },
    /// More entries than [`ArchiveLimits::max_entries`].
    #[error("archive declares {count} entries; the limit is {limit}")]
    TooManyEntries {
        /// Declared entry count.
        count: u32,
        /// The quota.
        limit: u32,
    },
    /// Bytes follow the last declared entry.
    #[error("data follows the last archive entry")]
    TrailingData,
    /// The first entry is not `manifest.json`.
    #[error("the first archive entry must be manifest.json")]
    ManifestFirst,
    /// The manifest is malformed.
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    /// An entry path breaks the archive path rules.
    #[error(transparent)]
    Path(#[from] PathError),
    /// An entry is out of canonical order or repeats a path exactly.
    #[error("entry {0} is out of canonical order")]
    Order(Rejected),
    /// Two entries have the same path ignoring ASCII case.
    #[error("entry {0} duplicates another path ignoring case")]
    DuplicatePath(Rejected),
    /// A file's path is also a directory of another entry, ignoring ASCII case.
    #[error("entry {0} is both a file and a directory")]
    PathConflict(Rejected),
    /// An entry is not a regular file: a directory, a link or an unknown kind.
    #[error("entry {path} has kind {kind}; only regular files are allowed")]
    EntryKind {
        /// The entry path.
        path: Rejected,
        /// The declared kind byte; 1 directory, 2 symbolic link, 3 hard link.
        kind: u8,
    },
    /// An entry declares a nonzero mode.
    #[error("entry {path} declares mode {mode}; modes are host-assigned")]
    EntryMode {
        /// The entry path.
        path: Rejected,
        /// The declared mode byte.
        mode: u8,
    },
    /// A file is larger than [`ArchiveLimits::max_file_bytes`] or, for the manifest,
    /// [`MAX_MANIFEST_BYTES`].
    #[error("entry {path} declares {size} bytes; the limit is {limit}")]
    FileTooLarge {
        /// The entry path.
        path: Rejected,
        /// Declared size.
        size: u64,
        /// The quota.
        limit: u64,
    },
    /// The files together exceed [`ArchiveLimits::max_total_bytes`].
    #[error("archive files exceed {limit} bytes")]
    TotalTooLarge {
        /// The quota.
        limit: u64,
    },
    /// A file the manifest names is not in the archive under exactly that path.
    #[error("manifest names missing file {0}")]
    MissingFile(Rejected),
    /// A declared component entry is not a WebAssembly binary.
    #[error("execution entry {0} is not a WebAssembly binary")]
    ExecutionPayload(Rejected),
    /// A native executable or library that no worker target declares.
    #[error("entry {0} is undeclared native code")]
    UndeclaredNativeCode(Rejected),
    /// The computed content digest differs from `integrity.content_digest`.
    #[error("content digest mismatch: declared {declared}, computed {computed}")]
    ContentDigest {
        /// The manifest's digest, lock form.
        declared: String,
        /// The digest of the archive's bytes, lock form.
        computed: String,
    },
    /// The verified digest differs from the lock the caller holds.
    #[error("package digest {actual} does not match the lock {expected}")]
    LockedDigest {
        /// The lock the caller supplied.
        expected: String,
        /// The verified content digest.
        actual: String,
    },
    /// The sink could not store a file.
    #[error("archive staging failed: {0:?}")]
    Sink(io::ErrorKind),
}

impl ArchiveError {
    /// Stable message key for host presentation (13 §2).
    pub const fn message_key(&self) -> &'static str {
        match self {
            Self::Read(_) => "packages/archive-read",
            Self::Truncated => "packages/archive-truncated",
            Self::Magic => "packages/archive-magic",
            Self::Version(_) => "packages/archive-version",
            Self::Flags(_) => "packages/archive-flags",
            Self::ArchiveTooLarge { .. } => "packages/archive-too-large",
            Self::TooManyEntries { .. } => "packages/archive-too-many-entries",
            Self::TrailingData => "packages/archive-trailing-data",
            Self::ManifestFirst => "packages/archive-manifest-first",
            Self::Manifest(error) => error.message_key(),
            Self::Path(_) => "packages/archive-path",
            Self::Order(_) => "packages/archive-order",
            Self::DuplicatePath(_) => "packages/archive-duplicate-path",
            Self::PathConflict(_) => "packages/archive-path-conflict",
            Self::EntryKind { .. } => "packages/archive-entry-kind",
            Self::EntryMode { .. } => "packages/archive-entry-mode",
            Self::FileTooLarge { .. } => "packages/archive-file-too-large",
            Self::TotalTooLarge { .. } => "packages/archive-total-too-large",
            Self::MissingFile(_) => "packages/archive-missing-file",
            Self::ExecutionPayload(_) => "packages/archive-execution-payload",
            Self::UndeclaredNativeCode(_) => "packages/archive-undeclared-native-code",
            Self::ContentDigest { .. } => "packages/archive-content-digest",
            Self::LockedDigest { .. } => "packages/archive-locked-digest",
            Self::Sink(_) => "packages/archive-sink",
        }
    }
}

/// One verified archive file.
#[derive(Clone, PartialEq, Eq)]
pub struct ArchiveEntry {
    path: ArchivePath,
    size: u64,
    digest: [u8; 32],
}

impl ArchiveEntry {
    /// The archive path.
    pub fn path(&self) -> &ArchivePath {
        &self.path
    }

    /// Size in bytes.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// SHA-256 of the file, lowercase hex.
    pub fn digest(&self) -> String {
        hex(&self.digest)
    }
}

impl fmt::Debug for ArchiveEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ArchiveEntry")
            .field("path", &self.path)
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

/// An archive whose structure, quotas, manifest, file references and content digest all
/// verified. It is the only way to obtain [`PackageContents`] from archive bytes.
#[derive(Clone, PartialEq)]
pub struct VerifiedArchive {
    manifest: PackageManifest,
    archive_digest: String,
    entries: Vec<ArchiveEntry>,
}

impl VerifiedArchive {
    /// The validated manifest.
    pub fn manifest(&self) -> &PackageManifest {
        &self.manifest
    }

    /// The verified content digest, lock form: what document and dependency locks record.
    pub fn content_digest(&self) -> &str {
        self.manifest.content_digest()
    }

    /// SHA-256 of the archive bytes, lock form: the origin hash shown for an unsigned local
    /// file. The canonical layout makes it as stable as the content digest.
    pub fn archive_digest(&self) -> &str {
        &self.archive_digest
    }

    /// Every file except the manifest, in archive order.
    pub fn entries(&self) -> &[ArchiveEntry] {
        &self.entries
    }

    /// Fails unless the content digest is `lock` (`sha256:<hex>`), the digest a document,
    /// dependency or revocation list names for this package.
    pub fn require_digest(&self, lock: &str) -> Result<(), ArchiveError> {
        if self.content_digest() == lock {
            Ok(())
        } else {
            Err(ArchiveError::LockedDigest {
                expected: lock.to_owned(),
                actual: self.content_digest().to_owned(),
            })
        }
    }

    /// Pairs the verified manifest with node definitions decoded from its files.
    pub fn into_contents(
        self,
        definitions: Vec<orange_model::catalog::NodeDefinition>,
    ) -> PackageContents {
        PackageContents::new(self.manifest, definitions)
    }
}

impl fmt::Debug for VerifiedArchive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VerifiedArchive")
            .field("manifest", &self.manifest)
            .field("entries", &self.entries.len())
            .finish_non_exhaustive()
    }
}

/// Where verified bytes go while an archive is read. The reader calls `begin`, `write`s
/// the file's bytes in order and `end`s it, once per entry, manifest first. After any
/// failure, before or after the sink saw bytes, it calls `abort` and nothing it received
/// may be used.
pub trait ArchiveSink {
    /// Starts a file of `size` bytes at `path`.
    fn begin(&mut self, path: &ArchivePath, size: u64) -> io::Result<()>;
    /// Appends bytes to the current file.
    fn write(&mut self, bytes: &[u8]) -> io::Result<()>;
    /// Completes the current file.
    fn end(&mut self) -> io::Result<()>;
    /// Discards everything received. Called once after a failure.
    fn abort(&mut self) {}
}

/// A sink that keeps only digests: verification without storing files.
#[derive(Debug, Clone, Copy, Default)]
pub struct DiscardSink;

impl ArchiveSink for DiscardSink {
    fn begin(&mut self, _path: &ArchivePath, _size: u64) -> io::Result<()> {
        Ok(())
    }

    fn write(&mut self, _bytes: &[u8]) -> io::Result<()> {
        Ok(())
    }

    fn end(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// A sink that holds the files in memory, bounded by [`ArchiveLimits::max_total_bytes`].
#[derive(Debug, Clone, Default)]
pub struct MemorySink {
    files: Vec<(ArchivePath, Vec<u8>)>,
}

impl MemorySink {
    /// An empty sink.
    pub fn new() -> Self {
        Self::default()
    }

    /// The received files, manifest first.
    pub fn files(&self) -> &[(ArchivePath, Vec<u8>)] {
        &self.files
    }

    /// The bytes of `path`, if received.
    pub fn file(&self, path: &str) -> Option<&[u8]> {
        self.files
            .iter()
            .find(|(candidate, _)| candidate.as_str() == path)
            .map(|(_, bytes)| bytes.as_slice())
    }
}

impl ArchiveSink for MemorySink {
    fn begin(&mut self, path: &ArchivePath, _size: u64) -> io::Result<()> {
        self.files.push((path.clone(), Vec::new()));
        Ok(())
    }

    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        if let Some((_, file)) = self.files.last_mut() {
            file.extend_from_slice(bytes);
        }
        Ok(())
    }

    fn end(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn abort(&mut self) {
        self.files.clear();
    }
}

/// A sink that stages files in a new directory. It creates every directory itself, refuses
/// a staging parent that is a link or reparse point, opens each file with `create_new`, and
/// flushes each file to disk before `end` returns. `abort` deletes the directory.
#[derive(Debug)]
pub struct DirectorySink {
    root: PathBuf,
    current: Option<File>,
    created: BTreeSet<PathBuf>,
}

impl DirectorySink {
    /// Creates `parent/name`, which must not exist. `parent` must be an existing plain
    /// directory; `name` must be one archive path component.
    pub fn create(parent: &Path, name: &str) -> io::Result<Self> {
        let valid = name
            .parse::<ArchivePath>()
            .is_ok_and(|path| path.components().count() == 1);
        if !valid {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        if !is_plain_dir(&fs::symlink_metadata(parent)?) {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        let root = parent.join(name);
        fs::create_dir(&root)?;
        Ok(Self {
            root,
            current: None,
            created: BTreeSet::new(),
        })
    }

    /// The staging directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn directory(&mut self, path: &Path) -> io::Result<()> {
        if self.created.contains(path) {
            return Ok(());
        }
        fs::create_dir(path)?;
        if !is_plain_dir(&fs::symlink_metadata(path)?) {
            return Err(io::Error::from(io::ErrorKind::InvalidData));
        }
        self.created.insert(path.to_owned());
        Ok(())
    }
}

impl ArchiveSink for DirectorySink {
    fn begin(&mut self, path: &ArchivePath, _size: u64) -> io::Result<()> {
        let mut target = self.root.clone();
        let components: Vec<&str> = path.components().collect();
        let (file, directories) = components
            .split_last()
            .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
        for component in directories {
            target.push(component);
            self.directory(&target)?;
        }
        target.push(file);
        self.current = Some(
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(target)?,
        );
        Ok(())
    }

    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        match &mut self.current {
            Some(file) => file.write_all(bytes),
            None => Err(io::Error::from(io::ErrorKind::InvalidInput)),
        }
    }

    fn end(&mut self) -> io::Result<()> {
        match self.current.take() {
            Some(file) => file.sync_all(),
            None => Err(io::Error::from(io::ErrorKind::InvalidInput)),
        }
    }

    fn abort(&mut self) {
        self.current = None;
        // Best effort: a failed cleanup leaves a staging directory no generation names.
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn is_plain_dir(metadata: &Metadata) -> bool {
    metadata.is_dir() && !metadata.file_type().is_symlink() && !is_reparse_point(metadata)
}

#[cfg(windows)]
fn is_reparse_point(metadata: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_point(_metadata: &Metadata) -> bool {
    false
}

/// Reads and verifies a canonical archive, streaming each file into `sink`. On success
/// every file has been delivered and the manifest's content digest matches the bytes; on
/// failure `sink.abort()` has been called.
pub fn read_archive(
    reader: impl Read,
    limits: &ArchiveLimits,
    sink: &mut impl ArchiveSink,
) -> Result<VerifiedArchive, ArchiveError> {
    let result = Reader {
        input: reader,
        read: 0,
        limits,
        archive_hash: Sha256::new(),
    }
    .run(sink);
    if result.is_err() {
        sink.abort();
    }
    result
}

struct Reader<'a, R> {
    input: R,
    read: u64,
    limits: &'a ArchiveLimits,
    archive_hash: Sha256,
}

/// Path state used to enforce canonical order and collision freedom.
#[derive(Default)]
struct Seen {
    previous: Option<ArchivePath>,
    files: BTreeSet<String>,
    directories: BTreeSet<String>,
}

impl Seen {
    fn admit(&mut self, path: &ArchivePath) -> Result<(), ArchiveError> {
        if self
            .previous
            .as_ref()
            .is_some_and(|previous| previous.as_str().as_bytes() >= path.as_str().as_bytes())
        {
            return Err(ArchiveError::Order(path.as_str().into()));
        }
        let normalized = path.normalized();
        if self.files.contains(&normalized) {
            return Err(ArchiveError::DuplicatePath(path.as_str().into()));
        }
        if self.directories.contains(&normalized) {
            return Err(ArchiveError::PathConflict(path.as_str().into()));
        }
        let mut prefix = String::new();
        let components: Vec<&str> = normalized.split('/').collect();
        for component in &components[..components.len() - 1] {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(component);
            if self.files.contains(&prefix) {
                return Err(ArchiveError::PathConflict(path.as_str().into()));
            }
            self.directories.insert(prefix.clone());
        }
        self.files.insert(normalized);
        self.previous = Some(path.clone());
        Ok(())
    }
}

/// What the first bytes of a file say it is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Sniffed {
    Wasm,
    Native,
    Other,
}

fn sniff(head: &[u8]) -> Sniffed {
    match head {
        [0x00, b'a', b's', b'm', ..] => Sniffed::Wasm,
        // PE/COFF, ELF, Mach-O (32/64-bit, both byte orders) and Mach-O universal.
        [b'M', b'Z', ..]
        | [0x7f, b'E', b'L', b'F', ..]
        | [0xfe, 0xed, 0xfa, 0xce | 0xcf, ..]
        | [0xce | 0xcf, 0xfa, 0xed, 0xfe, ..]
        | [0xca, 0xfe, 0xba, 0xbe, ..] => Sniffed::Native,
        _ => Sniffed::Other,
    }
}

impl<R: Read> Reader<'_, R> {
    fn run(mut self, sink: &mut impl ArchiveSink) -> Result<VerifiedArchive, ArchiveError> {
        let mut magic = [0; 8];
        self.exact(&mut magic)?;
        if magic != ARCHIVE_MAGIC {
            return Err(ArchiveError::Magic);
        }
        let version = self.u16()?;
        if version != ARCHIVE_VERSION {
            return Err(ArchiveError::Version(version));
        }
        let flags = self.u16()?;
        if flags != 0 {
            return Err(ArchiveError::Flags(flags));
        }
        let count = self.u32()?;
        if count > self.limits.max_entries {
            return Err(ArchiveError::TooManyEntries {
                count,
                limit: self.limits.max_entries,
            });
        }
        if count == 0 {
            return Err(ArchiveError::ManifestFirst);
        }

        let mut total = 0u64;
        let (path, size) = self.entry_header(&mut total)?;
        if path.as_str() != MANIFEST_PATH {
            return Err(ArchiveError::ManifestFirst);
        }
        let manifest_limit = MAX_MANIFEST_BYTES as u64;
        if size > manifest_limit {
            return Err(ArchiveError::FileTooLarge {
                path: path.as_str().into(),
                size,
                limit: manifest_limit,
            });
        }
        let mut manifest_bytes = vec![0; usize::try_from(size).unwrap_or(MAX_MANIFEST_BYTES)];
        self.exact(&mut manifest_bytes)?;
        let (manifest, body) = PackageManifest::decode(&manifest_bytes)?;
        sink.begin(&path, size).map_err(sink_error)?;
        sink.write(&manifest_bytes).map_err(sink_error)?;
        sink.end().map_err(sink_error)?;
        drop(manifest_bytes);

        let workers: BTreeSet<&str> = match manifest.execution() {
            Some(Execution::Worker { targets, .. }) => {
                targets.iter().map(|target| target.entry.as_str()).collect()
            }
            _ => BTreeSet::new(),
        };
        let component = match manifest.execution() {
            Some(Execution::WasmComponent { entry, .. }) => Some(entry.as_str()),
            _ => None,
        };

        let mut seen = Seen::default();
        seen.admit(&path)?;
        seen.previous = None;
        let mut entries = Vec::with_capacity(count as usize - 1);
        let mut buffer = vec![0; CHUNK];
        for _ in 1..count {
            let (path, size) = self.entry_header(&mut total)?;
            if path.as_str() == MANIFEST_PATH {
                return Err(ArchiveError::Order(path.as_str().into()));
            }
            seen.admit(&path)?;
            sink.begin(&path, size).map_err(sink_error)?;
            let mut file_hash = Sha256::new();
            let mut remaining = size;
            let mut first = true;
            while remaining > 0 {
                let step = usize::try_from(remaining.min(CHUNK as u64)).unwrap_or(CHUNK);
                let chunk = &mut buffer[..step];
                self.exact(chunk)?;
                if first {
                    first = false;
                    let kind = sniff(chunk);
                    if kind == Sniffed::Native && !workers.contains(path.as_str()) {
                        return Err(ArchiveError::UndeclaredNativeCode(path.as_str().into()));
                    }
                    if component == Some(path.as_str()) && kind != Sniffed::Wasm {
                        return Err(ArchiveError::ExecutionPayload(path.as_str().into()));
                    }
                }
                file_hash.update(&*chunk);
                sink.write(chunk).map_err(sink_error)?;
                remaining -= step as u64;
            }
            if size == 0 && component == Some(path.as_str()) {
                return Err(ArchiveError::ExecutionPayload(path.as_str().into()));
            }
            sink.end().map_err(sink_error)?;
            entries.push(ArchiveEntry {
                path,
                size,
                digest: file_hash.finalize().into(),
            });
        }

        let mut probe = [0; 1];
        match self.input.read(&mut probe) {
            Ok(0) => {}
            Ok(_) => return Err(ArchiveError::TrailingData),
            Err(error) => return Err(ArchiveError::Read(error.kind())),
        }

        for file in manifest.referenced_files() {
            if !entries.iter().any(|entry| entry.path == *file) {
                return Err(ArchiveError::MissingFile(file.as_str().into()));
            }
        }
        let computed = digest_lock(&content_digest_of(&body, &entries));
        if computed != manifest.content_digest() {
            return Err(ArchiveError::ContentDigest {
                declared: manifest.content_digest().to_owned(),
                computed,
            });
        }
        Ok(VerifiedArchive {
            manifest,
            archive_digest: digest_lock(&self.archive_hash.finalize().into()),
            entries,
        })
    }

    /// Reads one entry header and checks kind, mode and quotas before any content.
    fn entry_header(&mut self, total: &mut u64) -> Result<(ArchivePath, u64), ArchiveError> {
        let length = usize::from(self.u16()?);
        if length == 0 || length > MAX_PATH_BYTES {
            let mut shown = vec![0; length.min(MAX_PATH_BYTES)];
            self.exact(&mut shown)?;
            return Err(ArchiveError::Path(PathError {
                path: String::from_utf8_lossy(&shown).as_ref().into(),
                reason: crate::path::PathRule::Length,
            }));
        }
        let mut raw = vec![0; length];
        self.exact(&mut raw)?;
        let text = String::from_utf8_lossy(&raw);
        let path: ArchivePath = text.parse()?;
        let kind = self.u8()?;
        if kind != KIND_FILE {
            return Err(ArchiveError::EntryKind {
                path: path.as_str().into(),
                kind,
            });
        }
        let mode = self.u8()?;
        if mode != 0 {
            return Err(ArchiveError::EntryMode {
                path: path.as_str().into(),
                mode,
            });
        }
        let size = self.u64()?;
        if size > self.limits.max_file_bytes {
            return Err(ArchiveError::FileTooLarge {
                path: path.as_str().into(),
                size,
                limit: self.limits.max_file_bytes,
            });
        }
        *total = total.saturating_add(size);
        if *total > self.limits.max_total_bytes {
            return Err(ArchiveError::TotalTooLarge {
                limit: self.limits.max_total_bytes,
            });
        }
        if self.read.saturating_add(size) > self.limits.max_archive_bytes {
            return Err(ArchiveError::ArchiveTooLarge {
                limit: self.limits.max_archive_bytes,
            });
        }
        Ok((path, size))
    }

    fn exact(&mut self, buffer: &mut [u8]) -> Result<(), ArchiveError> {
        let length = buffer.len() as u64;
        if self.read.saturating_add(length) > self.limits.max_archive_bytes {
            return Err(ArchiveError::ArchiveTooLarge {
                limit: self.limits.max_archive_bytes,
            });
        }
        self.input.read_exact(buffer).map_err(|error| {
            if error.kind() == io::ErrorKind::UnexpectedEof {
                ArchiveError::Truncated
            } else {
                ArchiveError::Read(error.kind())
            }
        })?;
        self.read += length;
        self.archive_hash.update(&*buffer);
        Ok(())
    }

    fn u8(&mut self) -> Result<u8, ArchiveError> {
        let mut bytes = [0; 1];
        self.exact(&mut bytes)?;
        Ok(bytes[0])
    }

    fn u16(&mut self) -> Result<u16, ArchiveError> {
        let mut bytes = [0; 2];
        self.exact(&mut bytes)?;
        Ok(u16::from_le_bytes(bytes))
    }

    fn u32(&mut self) -> Result<u32, ArchiveError> {
        let mut bytes = [0; 4];
        self.exact(&mut bytes)?;
        Ok(u32::from_le_bytes(bytes))
    }

    fn u64(&mut self) -> Result<u64, ArchiveError> {
        let mut bytes = [0; 8];
        self.exact(&mut bytes)?;
        Ok(u64::from_le_bytes(bytes))
    }
}

fn sink_error(error: io::Error) -> ArchiveError {
    ArchiveError::Sink(error.kind())
}

fn content_digest_of(body: &[u8], entries: &[ArchiveEntry]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(CONTENT_DOMAIN);
    hash.update((body.len() as u64).to_le_bytes());
    hash.update(body);
    hash.update((entries.len() as u32).to_le_bytes());
    for entry in entries {
        let path = entry.path.as_str().as_bytes();
        hash.update((path.len() as u16).to_le_bytes());
        hash.update(path);
        hash.update(entry.size.to_le_bytes());
        hash.update(entry.digest);
    }
    hash.finalize().into()
}

fn digest_lock(digest: &[u8; 32]) -> String {
    format!("sha256:{}", hex(digest))
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(char::from(DIGITS[usize::from(byte >> 4)]));
        text.push(char::from(DIGITS[usize::from(byte & 0xf)]));
    }
    text
}

/// A package's files, sorted into canonical order and checked for collisions, ready to
/// digest and encode. The SDK packer path: compute [`content_digest`](Self::content_digest)
/// with any placeholder in `integrity.content_digest`, write it into the manifest, then
/// [`encode`](Self::encode).
#[derive(Clone)]
pub struct ArchiveFiles<'a> {
    files: Vec<(ArchivePath, &'a [u8])>,
}

impl fmt::Debug for ArchiveFiles<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ArchiveFiles")
            .field("files", &self.files.len())
            .finish()
    }
}

impl<'a> ArchiveFiles<'a> {
    /// Sorts `files` (every file but the manifest) and checks that their paths are distinct
    /// ignoring case and never both a file and a directory.
    pub fn new(mut files: Vec<(ArchivePath, &'a [u8])>) -> Result<Self, ArchiveError> {
        files.sort_by(|(a, _), (b, _)| a.as_str().as_bytes().cmp(b.as_str().as_bytes()));
        let mut seen = Seen::default();
        let manifest: ArchivePath = MANIFEST_PATH.parse()?;
        seen.admit(&manifest)?;
        seen.previous = None;
        for (path, _) in &files {
            if path.as_str() == MANIFEST_PATH {
                return Err(ArchiveError::Order(path.as_str().into()));
            }
            seen.admit(path)?;
        }
        Ok(Self { files })
    }

    /// The content digest, lock form, of `manifest` with these files. The manifest must be
    /// valid; its `integrity.content_digest` value is ignored.
    pub fn content_digest(&self, manifest: &[u8]) -> Result<String, ArchiveError> {
        let (_, body) = PackageManifest::decode(manifest)?;
        let entries: Vec<ArchiveEntry> = self
            .files
            .iter()
            .map(|(path, bytes)| ArchiveEntry {
                path: path.clone(),
                size: bytes.len() as u64,
                digest: Sha256::digest(bytes).into(),
            })
            .collect();
        Ok(digest_lock(&content_digest_of(&body, &entries)))
    }

    /// The canonical archive bytes of `manifest` and these files. Encoding checks layout
    /// only; [`read_archive`] verifies the result.
    pub fn encode(&self, manifest: &[u8]) -> Vec<u8> {
        let size = 16
            + manifest.len()
            + 25
            + self
                .files
                .iter()
                .map(|(path, bytes)| 12 + path.as_str().len() + bytes.len())
                .sum::<usize>();
        let mut out = Vec::with_capacity(size);
        out.extend_from_slice(&ARCHIVE_MAGIC);
        out.extend_from_slice(&ARCHIVE_VERSION.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&((self.files.len() + 1) as u32).to_le_bytes());
        let mut entry = |path: &str, bytes: &[u8]| {
            out.extend_from_slice(&(path.len() as u16).to_le_bytes());
            out.extend_from_slice(path.as_bytes());
            out.push(KIND_FILE);
            out.push(0);
            out.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
            out.extend_from_slice(bytes);
        };
        entry(MANIFEST_PATH, manifest);
        for (path, bytes) in &self.files {
            entry(path.as_str(), bytes);
        }
        out
    }
}
