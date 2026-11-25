//! File access confined to one directory.
//!
//! A [`Folder`] is a directory and the only place its methods reach. A path is relative
//! to it, with `/` between plain names; `.` and `..`, absolute and drive forms, device
//! names and links that lead out of the directory are refused. The base file package
//! and the plugin engine both hand file access to code they do not control through
//! this one type, so the rules are written once.
//!
//! Every operation walks from the root to the directory it changes through held
//! directory handles and acts on one name inside the last of them (see `os.rs`), so a
//! directory renamed or replaced by a link after it was checked cannot redirect it.
//!
//! Whole files are read and written. A write goes to a new temporary file in the same
//! directory, is flushed and renamed over the target, so a reader or a crash sees the
//! old file or the new one. [`replace_file`] does the same for a file the host owns
//! outside any folder, such as a saved document.
// Product code reports failures as typed errors; each remaining panic is a documented
// internal invariant (10, "Panic policy").
#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]

use std::io::{Read as _, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

mod os;
mod save;
mod tree;
use os::{Access, Dir, io};
pub use save::replace_file;
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
    /// The folder itself is missing: removed, renamed, or on a drive that is not
    /// connected. A folder made with [`Folder::existing`] is never created again.
    FolderMissing,
    /// The system denied access: permissions, an access list, or a read-only file.
    PermissionDenied,
    /// Another program has the file open without sharing it, or holds a lock on it.
    Busy,
    /// The volume is mounted read-only or its media is write-protected.
    ReadOnlyLocation,
    /// The disk or the user's quota is full.
    NoSpace,
    /// The destination is on another volume, where a move cannot be one rename.
    CrossDevice,
    /// The filesystem does not support the operation.
    Unsupported,
    /// Any other refusal by the operating system.
    Failed,
}

/// What a regular file or directory is, read without following a link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info {
    directory: bool,
    bytes: u64,
    modified: Option<SystemTime>,
    read_only: bool,
    identity: Option<(u64, u64, u64)>,
}

impl Info {
    /// Whether it is a directory; otherwise it is a regular file.
    pub fn is_dir(&self) -> bool {
        self.directory
    }
    /// Whether it is a regular file.
    pub fn is_file(&self) -> bool {
        !self.directory
    }
    /// A file's size in bytes; zero for a directory.
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
    /// When its content last changed, where the system records it.
    pub fn modified(&self) -> Option<SystemTime> {
        self.modified
    }
    /// Whether its permissions refuse writing.
    pub fn read_only(&self) -> bool {
        self.read_only
    }
    /// What identifies the file across a rename: device and inode on Unix; creation
    /// time, last write time and size on Windows, where stable std has no file index.
    pub fn identity(&self) -> Option<(u64, u64, u64)> {
        self.identity
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
    let mut full = root.to_path_buf();
    for name in names(path, root_allowed)? {
        full.push(name);
    }
    Ok(full)
}

/// The plain names of a path, checked the way [`inside`] describes.
fn names(path: &str, root_allowed: bool) -> Result<Vec<&str>, FsError> {
    if path.len() > MAX_PATH_BYTES || (path.is_empty() && !root_allowed) {
        return Err(FsError::InvalidPath);
    }
    let mut names = Vec::new();
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
        names.push(name);
    }
    Ok(names)
}

/// The directories of a path and its last name.
fn split(path: &str) -> Result<(Vec<&str>, &str), FsError> {
    let mut names = names(path, false)?;
    let last = names.pop().ok_or(FsError::InvalidPath)?;
    Ok((names, last))
}

/// One directory and the rules for reaching into it.
#[derive(Debug)]
pub struct Folder {
    root: PathBuf,
    read_only: bool,
    create_root: bool,
    temporary: AtomicU64,
}

impl Folder {
    /// A folder at `root`, which need not exist yet: it is created by the first
    /// write. With `read_only`, every changing method fails.
    pub fn new(root: impl Into<PathBuf>, read_only: bool) -> Self {
        Self {
            root: root.into(),
            read_only,
            create_root: true,
            temporary: AtomicU64::new(0),
        }
    }

    /// A folder the user chose, which must exist. When it goes missing every method
    /// fails with [`FsError::FolderMissing`] and nothing creates it again, so a drive
    /// that was disconnected is not replaced by an empty directory on another one.
    pub fn existing(root: impl Into<PathBuf>, read_only: bool) -> Self {
        Self {
            create_root: false,
            ..Self::new(root, read_only)
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

    fn writable(&self) -> Result<(), FsError> {
        if self.read_only {
            Err(FsError::ReadOnly)
        } else {
            Ok(())
        }
    }

    /// The root, held. A missing root is created first when `create` and the folder
    /// allows it.
    fn open_root(&self, create: bool) -> Result<Dir, FsError> {
        if create && self.create_root {
            std::fs::create_dir_all(&self.root).map_err(|error| io(&error))?;
        }
        match Dir::root(&self.root) {
            Err(FsError::NotFound) if !self.create_root => Err(FsError::FolderMissing),
            other => other,
        }
    }

    /// The existing directory the names lead to, entered one held name at a time.
    fn walk(&self, names: &[&str]) -> Result<Dir, FsError> {
        let root = self.open_root(false)?;
        descend(root, names)
    }

    /// The directory the names lead to, creating each missing one on the way.
    fn walk_making(&self, names: &[&str]) -> Result<Dir, FsError> {
        let mut current = self.open_root(true)?;
        for name in names {
            match current.stat(name) {
                Ok(info) if info.is_dir() => {}
                // A file is in the way.
                Ok(_) => return Err(FsError::InvalidPath),
                Err(FsError::NotFound) => match current.create_dir(name) {
                    // `Failed` covers a directory made by someone else meanwhile;
                    // entering it below decides.
                    Ok(()) | Err(FsError::Failed) => {}
                    Err(error) => return Err(error),
                },
                Err(error) => return Err(error),
            }
            current = match current.child(name) {
                Err(FsError::NotFound) => return Err(FsError::InvalidPath),
                other => other?,
            };
        }
        Ok(current)
    }

    /// The directory holding an existing regular file, and the file's name.
    fn source<'a>(&self, path: &'a str) -> Result<(Dir, &'a str), FsError> {
        let (directories, name) = split(path)?;
        let directory = self.walk(&directories)?;
        if !directory.stat(name)?.is_file() {
            return Err(FsError::NotFound);
        }
        Ok((directory, name))
    }

    /// The directory a file will be written in, created as needed, and the file's
    /// name. Something other than a regular file at the name is refused.
    fn target<'a>(&self, path: &'a str) -> Result<(Dir, &'a str), FsError> {
        self.writable()?;
        let (directories, name) = split(path)?;
        let directory = self.walk_making(&directories)?;
        match directory.stat(name) {
            Ok(info) if info.is_file() => {}
            Err(FsError::NotFound) => {}
            Ok(_) => return Err(FsError::InvalidPath),
            Err(error) => return Err(error),
        }
        Ok((directory, name))
    }

    /// Reads a whole file of at most `max_bytes`.
    pub fn read(&self, path: &str, max_bytes: u64) -> Result<Vec<u8>, FsError> {
        let (directory, name) = self.source(path)?;
        let file = directory.open(name, Access::Read)?;
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
        let (directory, name) = self.source(path)?;
        let file = directory.open(name, Access::Read)?;
        Ok(file.metadata().map_err(|e| io(&e))?.len())
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
        use std::io::{Seek, SeekFrom};
        if length > max_bytes {
            return Err(FsError::TooLarge);
        }
        let (directory, name) = self.source(path)?;
        let mut file = directory.open(name, Access::Read)?;
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
        let (directory, name) = self.target(path)?;
        self.replace_in(&directory, name, |file| {
            file.write_all(data).map_err(|error| io(&error))
        })
    }

    /// Writes a new temporary file in `directory`, flushes it and renames it over
    /// `name`. On any failure the temporary file is removed and whatever was at `name`
    /// is untouched.
    fn replace_in(
        &self,
        directory: &Dir,
        name: &str,
        write: impl FnOnce(&mut std::fs::File) -> Result<(), FsError>,
    ) -> Result<(), FsError> {
        let (temporary, mut file) = self.temporary_in(directory)?;
        let written = write(&mut file)
            .and_then(|()| file.sync_all().map_err(|error| io(&error)))
            .and_then(|()| {
                drop(file);
                directory.rename(&temporary, directory, name)
            });
        match written {
            Ok(()) => directory.sync(),
            Err(_) => {
                let _ = directory.remove_file(&temporary);
            }
        }
        written
    }

    /// A new temporary file in `directory`. It is created exclusively, so a file or
    /// link already at the name, which code with access to the folder may have put
    /// there, is never opened or written through; another name is tried instead. The
    /// name has a part nobody can predict, so such a name is rarely met at all.
    fn temporary_in(&self, directory: &Dir) -> Result<(String, std::fs::File), FsError> {
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
            match directory.create_new(&name) {
                Ok(file) => return Ok((name, file)),
                // Something already has the name: try another.
                Err(FsError::Failed | FsError::InvalidPath) => {}
                Err(error) => return Err(error),
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
        let (directory, name) = self.target(path)?;
        let mut file = directory.open(name, Access::Append)?;
        file.lock().map_err(|error| io(&error))?;
        let size = file.metadata().map_err(|error| io(&error))?.len();
        if size.saturating_add(data.len() as u64) > max_bytes {
            return Err(FsError::TooLarge);
        }
        file.write_all(data).map_err(|error| io(&error))
    }

    /// The names in a directory, sorted; `""` is the folder itself and a directory's
    /// name ends with `/`. A folder nothing was written to yet is empty, not missing.
    /// A name that is not valid UTF-8 cannot be addressed and is left out.
    pub fn list(&self, path: &str, max_entries: usize) -> Result<Vec<String>, FsError> {
        let path_names = names(path, true)?;
        if path.is_empty() && self.create_root && !os::exists(&self.root) {
            return Ok(Vec::new());
        }
        let directory = self.walk(&path_names)?;
        let mut names = Vec::new();
        for name in directory.names()? {
            let Ok(mut name) = name.into_string() else {
                continue;
            };
            if Self::is_temporary(&name) {
                continue;
            }
            match directory.stat(&name) {
                Ok(info) if info.is_dir() => name.push('/'),
                Ok(_) | Err(FsError::InvalidPath) => {}
                // Removed since the directory was read.
                Err(FsError::NotFound) => continue,
                Err(error) => return Err(error),
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
        match self.metadata(path) {
            Ok(_) => Ok(true),
            Err(FsError::NotFound) => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// Deletes a file.
    pub fn remove(&self, path: &str) -> Result<(), FsError> {
        self.writable()?;
        let (directory, name) = self.source(path)?;
        directory.remove_file(name)?;
        directory.sync();
        Ok(())
    }

    /// Moves or renames a file, replacing a file already at `to`. Both stay inside the
    /// folder and on one volume: a move to another volume fails with
    /// [`FsError::CrossDevice`] and changes nothing.
    pub fn rename(&self, path: &str, to: &str) -> Result<(), FsError> {
        self.writable()?;
        let (from, from_name) = self.source(path)?;
        let (target, target_name) = self.target(to)?;
        from.rename(from_name, &target, target_name)?;
        target.sync();
        from.sync();
        Ok(())
    }

    /// Copies a file of at most `max_bytes`, replacing a file already at `to`.
    pub fn copy(&self, path: &str, to: &str, max_bytes: u64) -> Result<(), FsError> {
        let (from, from_name) = self.source(path)?;
        let input = from.open(from_name, Access::Read)?;
        if input.metadata().map_err(|error| io(&error))?.len() > max_bytes {
            return Err(FsError::TooLarge);
        }
        let (target, target_name) = self.target(to)?;
        self.replace_in(&target, target_name, |output| {
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
        self.walk_making(&names(path, false)?).map(drop)
    }

    /// Removes an empty directory.
    pub fn remove_directory(&self, path: &str) -> Result<(), FsError> {
        self.writable()?;
        let (directories, name) = split(path)?;
        let parent = self.walk(&directories)?;
        if !parent.stat(name)?.is_dir() {
            return Err(FsError::NotFound);
        }
        parent.remove_dir(name)?;
        parent.sync();
        Ok(())
    }

    /// What a file or directory is; `""` is the folder itself.
    pub fn metadata(&self, path: &str) -> Result<Info, FsError> {
        if path.is_empty() {
            let _ = names(path, true)?;
            return self.open_root(false)?.info();
        }
        let (directories, name) = split(path)?;
        self.walk(&directories)?.stat(name)
    }
}

/// Enters each name below `directory` in turn.
fn descend(mut directory: Dir, names: &[&str]) -> Result<Dir, FsError> {
    for name in names {
        directory = directory.child(name)?;
    }
    Ok(directory)
}

#[cfg(test)]
mod tests;
