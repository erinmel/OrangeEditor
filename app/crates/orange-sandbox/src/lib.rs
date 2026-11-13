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

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

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

/// Names Windows gives to devices in every directory.
fn is_device_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.ends_with(|c: char| c.is_ascii_digit()))
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

    /// An existing file, checked for reading.
    fn source(&self, path: &str) -> Result<PathBuf, FsError> {
        let full = inside(&self.root, path, false)?;
        if !full.is_file() {
            return Err(FsError::NotFound);
        }
        self.contained(&full)?;
        Ok(full)
    }

    /// The target of a write: its directory is created and checked, and a link at
    /// the file's own name is refused because the write would follow it.
    fn target(&self, path: &str) -> Result<PathBuf, FsError> {
        self.writable()?;
        let full = inside(&self.root, path, false)?;
        let parent = full.parent().ok_or(FsError::InvalidPath)?;
        std::fs::create_dir_all(parent).map_err(|_| FsError::Failed)?;
        self.contained(parent)?;
        if full.is_symlink() {
            return Err(FsError::InvalidPath);
        }
        Ok(full)
    }

    /// Reads a whole file of at most `max_bytes`.
    pub fn read(&self, path: &str, max_bytes: u64) -> Result<Vec<u8>, FsError> {
        let full = self.source(path)?;
        let size = std::fs::metadata(&full).map_err(|e| io(&e))?.len();
        if size > max_bytes {
            return Err(FsError::TooLarge);
        }
        std::fs::read(&full).map_err(|e| io(&e))
    }

    /// Creates or replaces a file atomically, creating the directories on its path.
    pub fn write(&self, path: &str, data: &[u8]) -> Result<(), FsError> {
        let full = self.target(path)?;
        let name = format!(
            ".orange-{}-{}.tmp",
            std::process::id(),
            self.temporary.fetch_add(1, Ordering::Relaxed)
        );
        let temporary = full.with_file_name(name);
        let written = std::fs::File::create(&temporary)
            .and_then(|mut file| file.write_all(data).and_then(|()| file.sync_all()))
            .and_then(|()| std::fs::rename(&temporary, &full));
        if written.is_err() {
            let _ = std::fs::remove_file(&temporary);
            return Err(FsError::Failed);
        }
        Ok(())
    }

    /// Adds to the end of a file, creating it if needed. The file may not grow past
    /// `max_bytes`.
    pub fn append(&self, path: &str, data: &[u8], max_bytes: u64) -> Result<(), FsError> {
        let full = self.target(path)?;
        let size = std::fs::metadata(&full).map_or(0, |metadata| metadata.len());
        if size.saturating_add(data.len() as u64) > max_bytes {
            return Err(FsError::TooLarge);
        }
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&full)
            .and_then(|mut file| file.write_all(data))
            .map_err(|_| FsError::Failed)
    }

    /// The names in a directory, sorted; `""` is the folder itself and a directory's
    /// name ends with `/`. A folder nothing was written to yet is empty, not missing.
    pub fn list(&self, path: &str, max_entries: usize) -> Result<Vec<String>, FsError> {
        let full = inside(&self.root, path, true)?;
        if path.is_empty() && !full.exists() {
            return Ok(Vec::new());
        }
        self.contained(&full)?;
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
        let full = inside(&self.root, path, true)?;
        if !full.exists() {
            return Ok(false);
        }
        self.contained(&full).map(|()| true)
    }

    /// Deletes a file.
    pub fn remove(&self, path: &str) -> Result<(), FsError> {
        self.writable()?;
        let full = inside(&self.root, path, false)?;
        let parent = full.parent().ok_or(FsError::InvalidPath)?;
        if !parent.exists() {
            return Err(FsError::NotFound);
        }
        self.contained(parent)?;
        std::fs::remove_file(&full).map_err(|e| io(&e))
    }

    /// Moves or renames a file, replacing a file already at `to`.
    pub fn rename(&self, path: &str, to: &str) -> Result<(), FsError> {
        self.writable()?;
        let from = self.source(path)?;
        std::fs::rename(&from, self.target(to)?).map_err(|e| io(&e))
    }

    /// Copies a file of at most `max_bytes`, replacing a file already at `to`.
    pub fn copy(&self, path: &str, to: &str, max_bytes: u64) -> Result<(), FsError> {
        let from = self.source(path)?;
        if std::fs::metadata(&from).map_err(|e| io(&e))?.len() > max_bytes {
            return Err(FsError::TooLarge);
        }
        std::fs::copy(&from, self.target(to)?)
            .map(|_| ())
            .map_err(|e| io(&e))
    }

    /// Creates a directory and the directories on its path.
    pub fn make_directory(&self, path: &str) -> Result<(), FsError> {
        self.writable()?;
        let full = inside(&self.root, path, false)?;
        std::fs::create_dir_all(&full).map_err(|_| FsError::Failed)?;
        self.contained(&full)
    }

    /// Removes an empty directory.
    pub fn remove_directory(&self, path: &str) -> Result<(), FsError> {
        self.writable()?;
        let full = inside(&self.root, path, false)?;
        if !full.is_dir() {
            return Err(FsError::NotFound);
        }
        self.contained(&full)?;
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
}
