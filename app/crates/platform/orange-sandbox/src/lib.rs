//! File access confined to one directory.
//!
//! A [`Folder`] is a directory and the only place its methods reach. A path is relative
//! to it, with `/` between plain names; `.` and `..`, absolute and drive forms, device
//! names and links that lead out of the directory are refused. The base file package
//! and the plugin engine both hand file access to code they do not control through
//! this one type, so the rules are written once.
//!
//! Whole files are read and written. A write goes to a temporary file in the same
//! directory, is flushed and renamed over the target, so a reader or a crash sees the
//! old file or the new one.
// Product code reports failures as typed errors; each remaining panic is a documented
// internal invariant (10, "Panic policy").
#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]

use std::io::{Read as _, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

mod pin;
mod tree;
use pin::{Pins, open_file, open_read};
pub use tree::TreeEntry;

/// Longest path that can be named, in bytes.
pub const MAX_PATH_BYTES: usize = 1024;

/// Why a file operation was refused. No variant carries a path or operating-system
/// text, so nothing outside the folder leaks through an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FsError {
    /// The file or directory does not exist.
    NotFound,
    /// The path is not a relative path of plain names, or it leaves the folder.
    InvalidPath,
    /// The content or listing is over the limit given.
    TooLarge,
    /// The folder does not allow changes.
    ReadOnly,
    /// The owner cancelled the operation before it completed.
    Cancelled,
    /// A directory changed while its contents were being enumerated.
    Changed,
    /// Any other refusal by the operating system.
    Failed,
}

fn io(error: &std::io::Error) -> FsError {
    if error.kind() == std::io::ErrorKind::NotFound {
        FsError::NotFound
    } else {
        FsError::Failed
    }
}

/// Names Windows gives to devices in every directory. Windows 10 and earlier also
/// resolve the stem before any extension with trailing spaces removed (`nul .txt`), and
/// accept superscript digits after `COM` and `LPT`, so those forms are refused too.
fn is_device_name(name: &str) -> bool {
    let stem = name
        .split('.')
        .next()
        .unwrap_or(name)
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    let numbered = |prefix: &str| {
        stem.strip_prefix(prefix).is_some_and(|rest| {
            let mut chars = rest.chars();
            chars
                .next()
                .is_some_and(|c| c.is_ascii_digit() || matches!(c, '¹' | '²' | '³'))
                && chars.next().is_none()
        })
    };
    matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || numbered("COM")
        || numbered("LPT")
}

/// The path inside `root` that `path` names. `""` is the root when `root_allowed`.
/// This checks the text only; [`Folder`] also follows links.
pub fn inside(root: &Path, path: &str, root_allowed: bool) -> Result<PathBuf, FsError> {
    if path.len() > MAX_PATH_BYTES || (path.is_empty() && !root_allowed) {
        return Err(FsError::InvalidPath);
    }
    let mut full = root.to_path_buf();
    for name in path.split('/').filter(|_| !path.is_empty()) {
        let plain = !name.is_empty()
            && name != "."
            && name != ".."
            && !name.ends_with(['.', ' '])
            && !name
                .chars()
                .any(|c| c.is_control() || "\\:*?\"<>|".contains(c))
            && !is_device_name(name);
        if !plain {
            return Err(FsError::InvalidPath);
        }
        full.push(name);
    }
    Ok(full)
}

/// One directory and the rules for reaching into it.
#[derive(Debug)]
pub struct Folder {
    root: PathBuf,
    read_only: bool,
    temporary: AtomicU64,
}

impl Folder {
    /// A folder at `root`, which need not exist yet: it is created by the first
    /// write. With `read_only`, every changing method fails.
    pub fn new(root: impl Into<PathBuf>, read_only: bool) -> Self {
        Self {
            root: root.into(),
            read_only,
            temporary: AtomicU64::new(0),
        }
    }

    /// The directory itself.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Whether a file name is one of this type's own temporary files.
    pub fn is_temporary(name: &str) -> bool {
        name.starts_with(".orange-") && name.ends_with(".tmp")
    }

    /// Refuses an existing path that a link inside the folder leads out of it.
    fn contained(&self, existing: &Path) -> Result<(), FsError> {
        let root = self.root.canonicalize().map_err(|e| io(&e))?;
        let real = existing.canonicalize().map_err(|e| io(&e))?;
        if real.starts_with(root) {
            Ok(())
        } else {
            Err(FsError::InvalidPath)
        }
    }

    fn writable(&self) -> Result<(), FsError> {
        if self.read_only {
            Err(FsError::ReadOnly)
        } else {
            Ok(())
        }
    }

    /// An existing file, checked for reading, with its directories held.
    fn source(&self, path: &str) -> Result<(PathBuf, Pins), FsError> {
        let (full, metadata, held) = self.checked(path, false)?;
        if !metadata.is_file() {
            return Err(FsError::NotFound);
        }
        Ok((full, held))
    }

    /// The target of a write: its directory is created and checked, and a link at
    /// the file's own name is refused because the write would follow it.
    fn target(&self, path: &str) -> Result<(PathBuf, Pins), FsError> {
        self.writable()?;
        let full = inside(&self.root, path, false)?;
        let parent = full.parent().ok_or(FsError::InvalidPath)?;
        let held = self.create_inside(parent)?;
        match tree::metadata(&full) {
            Ok(info) if info.is_file() => {}
            Err(FsError::NotFound) => {}
            Ok(_) | Err(FsError::InvalidPath) => return Err(FsError::InvalidPath),
            Err(error) => return Err(error),
        }
        Ok((full, held))
    }

    /// Create one component at a time and verify every existing parent before any
    /// mutation. The root itself is the capability explicitly supplied by the host.
    /// Each directory on the way stays held while the returned [`Pins`] live.
    fn create_inside(&self, directory: &Path) -> Result<Pins, FsError> {
        let relative = directory
            .strip_prefix(&self.root)
            .map_err(|_| FsError::InvalidPath)?;
        std::fs::create_dir_all(&self.root).map_err(|error| io(&error))?;
        let mut current = self.root.clone();
        let _ = tree::metadata(&current)?;
        let mut held = Pins::default();
        held.hold(&current)?;
        for component in relative.components() {
            self.contained(&current)?;
            current.push(component);
            match std::fs::create_dir(&current) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(io(&error)),
            }
            if !tree::metadata(&current)?.is_dir() {
                return Err(FsError::InvalidPath);
            }
            held.hold(&current)?;
        }
        self.contained(&current)?;
        Ok(held)
    }

    /// Reads a whole file of at most `max_bytes`.
    pub fn read(&self, path: &str, max_bytes: u64) -> Result<Vec<u8>, FsError> {
        let (full, _held) = self.source(path)?;
        let file = open_read(&full)?;
        let size = file.metadata().map_err(|error| io(&error))?.len();
        if size > max_bytes {
            return Err(FsError::TooLarge);
        }
        let mut data = Vec::new();
        file.take(max_bytes.saturating_add(1))
            .read_to_end(&mut data)
            .map_err(|error| io(&error))?;
        if data.len() as u64 > max_bytes {
            return Err(FsError::TooLarge);
        }
        Ok(data)
    }

    /// The size of a file in bytes.
    pub fn size(&self, path: &str) -> Result<u64, FsError> {
        let (full, _held) = self.source(path)?;
        Ok(open_read(&full)?.metadata().map_err(|e| io(&e))?.len())
    }

    /// Reads up to `length` bytes of a file starting at `offset`: fewer at the end of
    /// the file, none past it. `length` may be at most `max_bytes`.
    pub fn read_range(
        &self,
        path: &str,
        offset: u64,
        length: u64,
        max_bytes: u64,
    ) -> Result<Vec<u8>, FsError> {
        use std::io::{Read, Seek, SeekFrom};
        if length > max_bytes {
            return Err(FsError::TooLarge);
        }
        let (full, _held) = self.source(path)?;
        let mut file = open_read(&full)?;
        let _ = file.seek(SeekFrom::Start(offset)).map_err(|e| io(&e))?;
        let mut data = Vec::new();
        let _ = file
            .take(length)
            .read_to_end(&mut data)
            .map_err(|e| io(&e))?;
        Ok(data)
    }

    /// Creates or replaces a file atomically, creating the directories on its path.
    pub fn write(&self, path: &str, data: &[u8]) -> Result<(), FsError> {
        let (full, _held) = self.target(path)?;
        self.replace_with(&full, |file| {
            file.write_all(data).map_err(|error| io(&error))
        })
    }

    fn replace_with(
        &self,
        full: &Path,
        write: impl FnOnce(&mut std::fs::File) -> Result<(), FsError>,
    ) -> Result<(), FsError> {
        let (temporary, mut file) = self.temporary_beside(full)?;
        let written = write(&mut file)
            .and_then(|()| file.sync_all().map_err(|error| io(&error)))
            .and_then(|()| {
                drop(file);
                std::fs::rename(&temporary, full).map_err(|error| io(&error))
            });
        if written.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        written
    }

    /// A new temporary file in the directory of `full`. It is created exclusively, so
    /// a file or link already at the name, which code with access to the folder may have
    /// put there, is never opened or written through; another name is tried instead.
    /// The name has a part nobody can predict, so such a name is rarely met at all.
    fn temporary_beside(&self, full: &Path) -> Result<(PathBuf, std::fs::File), FsError> {
        use std::hash::{BuildHasher, RandomState};
        const ATTEMPTS: usize = 16;
        let unpredictable = RandomState::new();
        for _ in 0..ATTEMPTS {
            let count = self.temporary.fetch_add(1, Ordering::Relaxed);
            let name = format!(
                ".orange-{}-{count}-{:016x}.tmp",
                std::process::id(),
                unpredictable.hash_one(count)
            );
            let temporary = full.with_file_name(name);
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
            {
                Ok(file) => return Ok((temporary, file)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(_) => return Err(FsError::Failed),
            }
        }
        Err(FsError::Failed)
    }

    /// Adds to the end of a file, creating it if needed. The file may not grow past
    /// `max_bytes`.
    pub fn append(&self, path: &str, data: &[u8], max_bytes: u64) -> Result<(), FsError> {
        if data.len() as u64 > max_bytes {
            return Err(FsError::TooLarge);
        }
        let (full, _held) = self.target(path)?;
        let mut file = open_file(
            std::fs::OpenOptions::new()
                .create(true)
                .read(true)
                .append(true),
            &full,
        )?;
        file.lock().map_err(|error| io(&error))?;
        let size = file.metadata().map_err(|error| io(&error))?.len();
        if size.saturating_add(data.len() as u64) > max_bytes {
            return Err(FsError::TooLarge);
        }
        file.write_all(data).map_err(|error| io(&error))
    }

    /// The names in a directory, sorted; `""` is the folder itself and a directory's
    /// name ends with `/`. A folder nothing was written to yet is empty, not missing.
    pub fn list(&self, path: &str, max_entries: usize) -> Result<Vec<String>, FsError> {
        let full = inside(&self.root, path, true)?;
        if path.is_empty() && !full.exists() {
            return Ok(Vec::new());
        }
        let (_, _, _held) = self.checked(path, true)?;
        let mut names = Vec::new();
        for entry in std::fs::read_dir(&full).map_err(|e| io(&e))? {
            let entry = entry.map_err(|_| FsError::Failed)?;
            let Ok(mut name) = entry.file_name().into_string() else {
                continue;
            };
            if Self::is_temporary(&name) {
                continue;
            }
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                name.push('/');
            }
            if names.len() >= max_entries {
                return Err(FsError::TooLarge);
            }
            names.push(name);
        }
        names.sort();
        Ok(names)
    }

    /// Whether a file or directory exists.
    pub fn exists(&self, path: &str) -> Result<bool, FsError> {
        match self.checked(path, true) {
            Ok(_) => Ok(true),
            Err(FsError::NotFound) => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// Deletes a file.
    pub fn remove(&self, path: &str) -> Result<(), FsError> {
        self.writable()?;
        let (full, _held) = self.source(path)?;
        std::fs::remove_file(&full).map_err(|e| io(&e))
    }

    /// Moves or renames a file, replacing a file already at `to`.
    pub fn rename(&self, path: &str, to: &str) -> Result<(), FsError> {
        self.writable()?;
        let (from, _from_held) = self.source(path)?;
        let (to, _to_held) = self.target(to)?;
        std::fs::rename(&from, &to).map_err(|e| io(&e))
    }

    /// Copies a file of at most `max_bytes`, replacing a file already at `to`.
    pub fn copy(&self, path: &str, to: &str, max_bytes: u64) -> Result<(), FsError> {
        let (from, _from_held) = self.source(path)?;
        let input = open_read(&from)?;
        if input.metadata().map_err(|error| io(&error))?.len() > max_bytes {
            return Err(FsError::TooLarge);
        }
        let (target, _target_held) = self.target(to)?;
        self.replace_with(&target, |output| {
            let count = std::io::copy(&mut input.take(max_bytes.saturating_add(1)), output)
                .map_err(|error| io(&error))?;
            if count > max_bytes {
                return Err(FsError::TooLarge);
            }
            Ok(())
        })
    }

    /// Creates a directory and the directories on its path.
    pub fn make_directory(&self, path: &str) -> Result<(), FsError> {
        self.writable()?;
        let full = inside(&self.root, path, false)?;
        self.create_inside(&full).map(drop)
    }

    /// Removes an empty directory.
    pub fn remove_directory(&self, path: &str) -> Result<(), FsError> {
        self.writable()?;
        let (full, info, mut held) = self.checked(path, false)?;
        if !info.is_dir() {
            return Err(FsError::NotFound);
        }
        held.release_last();
        std::fs::remove_dir(&full).map_err(|_| FsError::Failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_relative_paths_stay_inside_the_directory() {
        let root = Path::new("data");
        assert_eq!(
            inside(root, "a/b.txt", false),
            Ok(root.join("a").join("b.txt"))
        );
        assert_eq!(inside(root, "", true), Ok(root.to_path_buf()));
        for bad in [
            "",
            "..",
            "a/../b",
            "./a",
            "/etc/passwd",
            "a//b",
            "a/",
            "C:/Windows",
            "a\\b",
            "nul",
            "dir/COM1.txt",
            "trailing.",
            "space ",
            "a\0b",
            "nul .txt",
            "Aux  .log",
            "COM¹",
            "lpt³.txt",
            "COM0",
            "CONIN$",
            "conout$.txt",
            "a:stream",
        ] {
            assert_eq!(
                inside(root, bad, false),
                Err(FsError::InvalidPath),
                "{bad:?}"
            );
        }
        assert_eq!(
            inside(root, &"a".repeat(MAX_PATH_BYTES + 1), false),
            Err(FsError::InvalidPath)
        );
        // Names that only look like devices are ordinary files.
        for plain in ["console", "COM10", "LPT", "nul_.txt", "comic.txt", "xnul"] {
            assert!(inside(root, plain, false).is_ok(), "{plain:?}");
        }
    }

    #[test]
    fn a_folder_that_does_not_exist_yet_is_empty_and_made_by_the_first_write() {
        // Nested, so the directory above the folder is this test's own too.
        let above = std::env::temp_dir().join(format!("orange-sandbox-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&above);
        let folder = Folder::new(above.join("folder"), false);
        assert_eq!(folder.list("", 10), Ok(vec![]));
        assert_eq!(folder.read("a.txt", 16), Err(FsError::NotFound));
        assert_eq!(folder.exists("a.txt"), Ok(false));

        folder.write("b/c.bin", &[1, 2, 3]).unwrap();
        folder.write("a.txt", b"hello").unwrap();
        folder.write("a.txt", b"again").unwrap();
        // Replacing leaves no temporary file behind.
        assert_eq!(
            folder.list("", 10),
            Ok(vec!["a.txt".to_owned(), "b/".to_owned()])
        );
        assert_eq!(folder.list("", 1), Err(FsError::TooLarge));
        assert_eq!(folder.read("b/c.bin", 3), Ok(vec![1, 2, 3]));
        assert_eq!(folder.read("b/c.bin", 2), Err(FsError::TooLarge));
        assert_eq!(folder.remove("a.txt"), Ok(()));
        assert_eq!(folder.remove("a.txt"), Err(FsError::NotFound));
        for escape in ["../x", "/x", "b/../../x"] {
            assert_eq!(folder.write(escape, b"x"), Err(FsError::InvalidPath));
            assert_eq!(folder.read(escape, 16), Err(FsError::InvalidPath));
            assert_eq!(folder.remove(escape), Err(FsError::InvalidPath));
        }
        assert!(!above.join("x").exists());

        let locked = Folder::new(above.join("folder"), true);
        assert_eq!(locked.read("b/c.bin", 3), Ok(vec![1, 2, 3]));
        assert_eq!(locked.write("b/c.bin", b"x"), Err(FsError::ReadOnly));
        assert_eq!(locked.rename("b/c.bin", "d"), Err(FsError::ReadOnly));
        assert_eq!(locked.make_directory("e"), Err(FsError::ReadOnly));
        let _ = std::fs::remove_dir_all(&above);
    }

    #[test]
    fn concurrent_appends_enforce_the_same_limit_across_folder_instances() {
        let root =
            std::env::temp_dir().join(format!("orange-sandbox-append-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let folder = Folder::new(&root, false);
        folder.write("shared", b"").unwrap();
        let barrier = std::sync::Barrier::new(2);
        let outcomes = std::thread::scope(|scope| {
            let append = || {
                let folder = Folder::new(&root, false);
                barrier.wait();
                folder.append("shared", b"1234", 4)
            };
            let first = scope.spawn(append);
            let second = append();
            [first.join().unwrap(), second]
        });
        assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
        assert!(outcomes.contains(&Err(FsError::TooLarge)));
        assert_eq!(folder.read("shared", 4).unwrap(), b"1234");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_link_planted_at_a_temporary_name_is_never_written_through() {
        let above =
            std::env::temp_dir().join(format!("orange-sandbox-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&above);
        let inner = above.join("folder");
        std::fs::create_dir_all(&inner).unwrap();
        let outside = above.join("outside.txt");
        std::fs::write(&outside, b"kept").unwrap();
        // Links at the names a counter alone would give the first writes.
        for count in 0..8 {
            let name = format!(".orange-{}-{count}.tmp", std::process::id());
            std::fs::hard_link(&outside, inner.join(name)).unwrap();
        }
        let folder = Folder::new(&inner, false);
        for _ in 0..8 {
            folder.write("a.txt", b"written").unwrap();
        }
        assert_eq!(std::fs::read(&outside).unwrap(), b"kept");
        assert_eq!(folder.read("a.txt", 16), Ok(b"written".to_vec()));
        // Exclusive creation refuses whatever is already at a name.
        let taken = inner.join("taken.tmp");
        std::fs::hard_link(&outside, &taken).unwrap();
        assert!(
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&taken)
                .is_err()
        );
        let _ = std::fs::remove_dir_all(&above);
    }
}
