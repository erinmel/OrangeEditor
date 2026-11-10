//! A durable, file-backed [`Journal`] (04 "Hot update/removal"; K01).
//!
//! One log file per store directory, all integers little-endian:
//!
//! ```text
//! header  "OJOURNAL" | version u16 = 2
//! record  length u32 | header_crc32 u32 | payload_crc32 u32 | length bytes of JSON
//! ```
//!
//! `append` writes one whole record at the end of the file and flushes it before returning.
//! A crash can therefore leave at most one torn record, always at the tail: opening the log
//! replays complete records and truncates only a physically short final frame. The
//! header checksum protects its declared length; a full header or payload with a bad
//! checksum is corruption and refuses open without truncation. Recovery includes all
//! complete committed records (including a write flushed just before a crash, even if
//! the caller never observed its acknowledgement). A failed append truncates its own
//! partial bytes before returning the error.
//!
//! `compact` writes the header and one checkpoint record to a sibling temporary file,
//! flushes it, and renames it over the log. A crash before the rename leaves the old,
//! complete log (open deletes the stale temporary file); after it, exactly the checkpoint.
//!
//! The record JSON is this crate's current entry shape only. A log with another header
//! version, or a CRC-valid record this version cannot decode, is refused rather than
//! migrated: before the first release there are no historical journal readers.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use orange_model::catalog::ProviderKey;
use orange_model::document::DefinitionRef;
use serde::{Deserialize, Serialize};

use crate::journal::{Journal, JournalEntry, JournalError};
use crate::lifecycle::PackageState;
use crate::manifest::{PackageId, PackageKey, PackageVersion};

/// The log file's name inside the journal directory.
pub const JOURNAL_FILE: &str = "packages.journal";
/// The temporary file a compaction writes before renaming it over the log.
pub const JOURNAL_TEMP_FILE: &str = "packages.journal.tmp";
const JOURNAL_LOCK_FILE: &str = "packages.journal.lock";
/// Largest record accepted, in bytes: a full checkpoint must fit.
pub const MAX_JOURNAL_RECORD_BYTES: u32 = 64 << 20;

const HEADER: [u8; 10] = *b"OJOURNAL\x02\x00";
const FRAME_HEADER_BYTES: usize = 12;

/// Releases flock ownership explicitly, even if a process transiently inherited a
/// duplicate descriptor while another process was being spawned.
#[derive(Debug)]
struct WriterLock(File);

impl Drop for WriterLock {
    fn drop(&mut self) {
        // A duplicate descriptor can keep a flock alive after this File closes.
        // Unlock first so process-spawn races cannot extend writer ownership.
        let _ = self.0.unlock();
    }
}

/// A journal stored in [`JOURNAL_FILE`] inside a host-chosen directory.
#[derive(Debug)]
pub struct FileJournal {
    directory: PathBuf,
    file: Option<File>,
    // Declared after `file` so the append handle closes before WriterLock unlocks.
    _lock: WriterLock,
    length: u64,
}

/// What opening a journal found.
#[derive(Debug)]
pub struct OpenedJournal {
    /// The journal, positioned to append after the last complete record.
    pub journal: FileJournal,
    /// Every complete record, oldest first; feed them to
    /// [`Recovery::from_entries`](crate::Recovery::from_entries).
    pub entries: Vec<JournalEntry>,
    /// Bytes of a torn tail record that open truncated; zero after a clean shutdown.
    pub discarded_tail_bytes: u64,
}

fn failure(what: &str, error: &io::Error) -> JournalError {
    // The host's I/O message can name paths; only the operation and error kind are kept.
    JournalError {
        reason: format!("{what}: {:?}", error.kind()),
    }
}

fn invalid(what: &str) -> JournalError {
    JournalError {
        reason: what.to_owned(),
    }
}

impl FileJournal {
    /// Opens or creates the journal in `directory`, which must exist. It refuses to open
    /// while another process or handle owns that directory's journal. The exclusive
    /// writer lock is held until the returned journal is dropped. Replays complete records,
    /// truncates a torn tail and removes a stale compaction file.
    pub fn open(directory: &Path) -> Result<OpenedJournal, JournalError> {
        let lock_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join(JOURNAL_LOCK_FILE))
            .map_err(|error| failure("open journal writer lock", &error))?;
        match lock_file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                return Err(invalid("journal already has a writer"));
            }
            Err(TryLockError::Error(error)) => {
                return Err(failure("lock journal writer", &error));
            }
        }
        let lock = WriterLock(lock_file);

        let temp = directory.join(JOURNAL_TEMP_FILE);
        match fs::remove_file(&temp) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(failure("remove stale compaction file", &error)),
        }
        let path = directory.join(JOURNAL_FILE);
        let mut bytes = Vec::new();
        match File::open(&path) {
            Ok(mut file) => {
                file.read_to_end(&mut bytes)
                    .map_err(|error| failure("read journal", &error))?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(failure("open journal", &error)),
        }

        // A log shorter than its header is a creation that never completed.
        if bytes.len() < HEADER.len() && HEADER.starts_with(&bytes) {
            write_new(&path, &HEADER).map_err(|error| failure("create journal", &error))?;
            return Ok(OpenedJournal {
                journal: Self::reopen(directory, HEADER.len() as u64, lock)?,
                entries: Vec::new(),
                discarded_tail_bytes: 0,
            });
        }
        if !bytes.starts_with(&HEADER) {
            return Err(invalid("unsupported journal format"));
        }

        let mut entries = Vec::new();
        let mut offset = HEADER.len();
        while let Some((record, next)) = record_at(&bytes, offset)? {
            let wire: WireEntry = serde_json::from_slice(record)
                .map_err(|_| invalid("journal record has an unsupported shape"))?;
            entries.push(wire.into_entry()?);
            offset = next;
        }
        let discarded = (bytes.len() - offset) as u64;
        if discarded > 0 {
            let file = OpenOptions::new()
                .write(true)
                .open(&path)
                .map_err(|error| failure("open journal", &error))?;
            file.set_len(offset as u64)
                .and_then(|()| file.sync_all())
                .map_err(|error| failure("truncate torn record", &error))?;
        }
        Ok(OpenedJournal {
            journal: Self::reopen(directory, offset as u64, lock)?,
            entries,
            discarded_tail_bytes: discarded,
        })
    }

    fn reopen(directory: &Path, length: u64, lock: WriterLock) -> Result<Self, JournalError> {
        let file = open_append(directory)?;
        Ok(Self {
            directory: directory.to_owned(),
            file: Some(file),
            _lock: lock,
            length,
        })
    }

    /// The directory that holds the log.
    pub fn directory(&self) -> &Path {
        &self.directory
    }
}

impl Journal for FileJournal {
    fn append(&mut self, entry: JournalEntry) -> Result<(), JournalError> {
        let frame = frame(&entry)?;
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| invalid("journal is closed after a failed compaction"))?;
        let written = file.write_all(&frame).and_then(|()| file.sync_data());
        match written {
            Ok(()) => {
                self.length += frame.len() as u64;
                Ok(())
            }
            Err(error) => {
                // Remove partial bytes so a later append cannot follow a torn record.
                if file
                    .set_len(self.length)
                    .and_then(|()| file.sync_data())
                    .is_err()
                {
                    self.file = None;
                }
                Err(failure("append journal record", &error))
            }
        }
    }

    fn compact(&mut self, checkpoint: JournalEntry) -> Result<(), JournalError> {
        if !matches!(checkpoint, JournalEntry::Committed { .. }) {
            return Err(invalid("compaction needs a committed checkpoint"));
        }
        let mut bytes = HEADER.to_vec();
        bytes.extend_from_slice(&frame(&checkpoint)?);
        let temp = self.directory.join(JOURNAL_TEMP_FILE);
        if let Err(error) = write_new(&temp, &bytes) {
            let _ = fs::remove_file(&temp);
            return Err(failure("write compacted journal", &error));
        }
        // Close the log before replacing it; the rename is the commit point.
        self.file = None;
        if let Err(error) = fs::rename(&temp, self.directory.join(JOURNAL_FILE)) {
            let _ = fs::remove_file(&temp);
            self.file = Some(open_append(&self.directory)?);
            return Err(failure("replace journal", &error));
        }
        sync_directory(&self.directory);
        self.file = Some(open_append(&self.directory)?);
        self.length = bytes.len() as u64;
        Ok(())
    }
}

/// Writes `bytes` to a new file at `path` and flushes it.
fn write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn open_append(directory: &Path) -> Result<File, JournalError> {
    OpenOptions::new()
        .append(true)
        .open(directory.join(JOURNAL_FILE))
        .map_err(|error| failure("open journal", &error))
}

/// Flushes the directory entry of a rename where the platform allows opening directories.
#[cfg(unix)]
fn sync_directory(directory: &Path) {
    if let Ok(handle) = File::open(directory) {
        let _ = handle.sync_all();
    }
}

/// Windows offers no directory flush through std; NTFS journals the rename's metadata.
#[cfg(not(unix))]
fn sync_directory(_directory: &Path) {}

/// The complete record at `offset` and the offset after it, or `None` at the end or at a
/// physically short final frame. A corrupt complete header/payload is refused.
fn record_at(bytes: &[u8], offset: usize) -> Result<Option<(&[u8], usize)>, JournalError> {
    let Some(header) = bytes.get(offset..offset.saturating_add(FRAME_HEADER_BYTES)) else {
        return Ok(None);
    };
    let length_bytes = [header[0], header[1], header[2], header[3]];
    let length = u32::from_le_bytes(length_bytes);
    let header_crc = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
    if crc32(&length_bytes) != header_crc {
        return Err(invalid("journal record header checksum mismatch"));
    }
    let crc = u32::from_le_bytes([header[8], header[9], header[10], header[11]]);
    if length > MAX_JOURNAL_RECORD_BYTES {
        return Err(invalid("journal record exceeds the size limit"));
    }
    let start = offset + FRAME_HEADER_BYTES;
    let Some(end) = start.checked_add(length as usize) else {
        return Err(invalid("journal record exceeds the size limit"));
    };
    let Some(record) = bytes.get(start..end) else {
        return Ok(None);
    };
    if crc32(record) != crc {
        return Err(invalid("journal record payload checksum mismatch"));
    }
    Ok(Some((record, end)))
}

fn frame(entry: &JournalEntry) -> Result<Vec<u8>, JournalError> {
    let payload = serde_json::to_vec(&WireEntry::from_entry(entry))
        .map_err(|_| invalid("journal record did not encode"))?;
    let length = u32::try_from(payload.len())
        .ok()
        .filter(|length| *length <= MAX_JOURNAL_RECORD_BYTES)
        .ok_or_else(|| invalid("journal record exceeds the size limit"))?;
    let mut frame = Vec::with_capacity(payload.len() + FRAME_HEADER_BYTES);
    let length = length.to_le_bytes();
    frame.extend_from_slice(&length);
    frame.extend_from_slice(&crc32(&length).to_le_bytes());
    frame.extend_from_slice(&crc32(&payload).to_le_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

/// CRC-32 (IEEE 802.3, reflected, as in zlib and PNG): detects torn writes, not tampering.
pub(crate) fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "entry", rename_all = "snake_case", deny_unknown_fields)]
enum WireEntry {
    Transition {
        package: WireKey,
        state: WireState,
    },
    Committed {
        generation: u64,
        active: Vec<WireKey>,
        published: Vec<WirePublished>,
        revoked: Vec<String>,
        defaults: Vec<WireDefault>,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireKey {
    id: String,
    version: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WirePublished {
    id: String,
    version: String,
    content_digest: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireDefault {
    definition: DefinitionRef,
    provider: Option<WireProvider>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireProvider {
    package: String,
    version: String,
    content_digest: String,
    id: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum WireState {
    Discovered,
    Staged,
    Validated,
    Granted,
    Activating,
    Active,
    Quarantined,
    Removed,
    Revoked,
}

impl WireKey {
    fn from_key(key: &PackageKey) -> Self {
        Self {
            id: key.id.to_string(),
            version: key.version.to_string(),
        }
    }

    fn into_key(self) -> Result<PackageKey, JournalError> {
        let id: PackageId = self
            .id
            .parse()
            .map_err(|_| invalid("journal record names an invalid package"))?;
        let version: PackageVersion = self
            .version
            .parse()
            .map_err(|_| invalid("journal record names an invalid version"))?;
        Ok(PackageKey { id, version })
    }
}

impl WireState {
    fn from_state(state: PackageState) -> Self {
        match state {
            PackageState::Discovered => Self::Discovered,
            PackageState::Staged => Self::Staged,
            PackageState::Validated => Self::Validated,
            PackageState::Granted => Self::Granted,
            PackageState::Activating => Self::Activating,
            PackageState::Active => Self::Active,
            PackageState::Quarantined => Self::Quarantined,
            PackageState::Removed => Self::Removed,
            PackageState::Revoked => Self::Revoked,
        }
    }

    fn into_state(self) -> PackageState {
        match self {
            Self::Discovered => PackageState::Discovered,
            Self::Staged => PackageState::Staged,
            Self::Validated => PackageState::Validated,
            Self::Granted => PackageState::Granted,
            Self::Activating => PackageState::Activating,
            Self::Active => PackageState::Active,
            Self::Quarantined => PackageState::Quarantined,
            Self::Removed => PackageState::Removed,
            Self::Revoked => PackageState::Revoked,
        }
    }
}

impl WireEntry {
    fn from_entry(entry: &JournalEntry) -> Self {
        match entry {
            JournalEntry::Transition { package, state } => Self::Transition {
                package: WireKey::from_key(package),
                state: WireState::from_state(*state),
            },
            JournalEntry::Committed {
                generation,
                active,
                published,
                revoked,
                defaults,
            } => Self::Committed {
                generation: *generation,
                active: active.iter().map(WireKey::from_key).collect(),
                published: published
                    .iter()
                    .map(|(key, digest)| WirePublished {
                        id: key.id.to_string(),
                        version: key.version.to_string(),
                        content_digest: digest.clone(),
                    })
                    .collect(),
                revoked: revoked.iter().cloned().collect(),
                defaults: defaults
                    .iter()
                    .map(|(definition, provider)| WireDefault {
                        definition: definition.clone(),
                        provider: provider.as_ref().map(|provider| WireProvider {
                            package: provider.package.clone(),
                            version: provider.version.clone(),
                            content_digest: provider.content_digest.clone(),
                            id: provider.id.clone(),
                        }),
                    })
                    .collect(),
            },
        }
    }

    fn into_entry(self) -> Result<JournalEntry, JournalError> {
        Ok(match self {
            Self::Transition { package, state } => JournalEntry::Transition {
                package: package.into_key()?,
                state: state.into_state(),
            },
            Self::Committed {
                generation,
                active,
                published,
                revoked,
                defaults,
            } => JournalEntry::Committed {
                generation,
                active: active
                    .into_iter()
                    .map(WireKey::into_key)
                    .collect::<Result<_, _>>()?,
                published: published
                    .into_iter()
                    .map(|entry| {
                        let key = WireKey {
                            id: entry.id,
                            version: entry.version,
                        }
                        .into_key()?;
                        Ok((key, entry.content_digest))
                    })
                    .collect::<Result<BTreeMap<_, _>, JournalError>>()?,
                revoked: revoked.into_iter().collect::<BTreeSet<_>>(),
                defaults: defaults
                    .into_iter()
                    .map(|entry| {
                        let provider = entry.provider.map(|provider| ProviderKey {
                            package: provider.package,
                            version: provider.version,
                            content_digest: provider.content_digest,
                            id: provider.id,
                        });
                        (entry.definition, provider)
                    })
                    .collect(),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::crc32;

    #[test]
    fn crc32_matches_the_ieee_check_value() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn writer_guard_unlocks_while_a_duplicate_descriptor_remains_open() {
        use super::{JOURNAL_LOCK_FILE, WriterLock};
        use std::fs::{self, File, OpenOptions};
        use std::time::{SystemTime, UNIX_EPOCH};

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "orange-journal-lock-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let path = directory.join(JOURNAL_LOCK_FILE);

        let locked = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        locked.try_lock().unwrap();
        let guard = WriterLock(locked);
        let inherited_duplicate = guard.0.try_clone().unwrap();

        drop(guard);
        let reacquired = File::open(&path).unwrap();
        assert!(
            reacquired.try_lock().is_ok(),
            "Drop must unlock while an inherited duplicate fd remains open"
        );
        drop((reacquired, inherited_duplicate));
        fs::remove_dir_all(directory).unwrap();
    }
}
