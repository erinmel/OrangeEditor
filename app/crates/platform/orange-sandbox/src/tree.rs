//! Bounded operations on directory trees. Traversal never enters a link or reparse
//! point: each directory is entered through its held parent (see `os.rs`).

use super::os::{Access, Dir};
use super::{Folder, FsError, Info, io, names, split};
use std::io::{Read, Write};

/// One addressable regular file or directory, relative to the directory that was listed.
#[derive(Debug)]
pub struct TreeEntry {
    pub path: String,
    pub info: Info,
}

fn check_cancelled(cancelled: &dyn Fn() -> bool) -> Result<(), FsError> {
    if cancelled() {
        Err(FsError::Cancelled)
    } else {
        Ok(())
    }
}

fn changed(error: FsError) -> FsError {
    match error {
        FsError::NotFound => FsError::Changed,
        _ => error,
    }
}

fn join(parent: &str, child: &str) -> String {
    match (parent, child) {
        ("", child) => child.to_owned(),
        (parent, "") => parent.to_owned(),
        _ => format!("{parent}/{child}"),
    }
}

/// The parent part and the last name of a path relative to a tree.
fn parent_and_name(path: &str) -> (&str, &str) {
    path.rsplit_once('/').unwrap_or(("", path))
}

/// The directory `relative` names below `base`, entered through held handles.
fn below(base: &Dir, relative: &str) -> Result<Dir, FsError> {
    let mut current = base.duplicate()?;
    for name in relative.split('/').filter(|name| !name.is_empty()) {
        current = current.child(name)?;
    }
    Ok(current)
}

impl Folder {
    /// Lists regular files and directories, skipping links and nonportable names.
    /// Every item examined, including skipped entries, counts against `max_entries`.
    pub fn tree(
        &self,
        path: &str,
        recursive: bool,
        max_entries: usize,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Vec<TreeEntry>, FsError> {
        let start = self.walk(&names(path, true)?)?;
        collect(&start, recursive, false, max_entries, cancelled)
    }

    /// The tree at `path`, which must be a directory other than the folder itself, and
    /// its parent.
    fn tree_root<'a>(&self, path: &'a str) -> Result<(Dir, &'a str, Dir), FsError> {
        let (directories, name) = split(path)?;
        let parent = self.walk(&directories)?;
        if !parent.stat(name)?.is_dir() {
            return Err(FsError::NotFound);
        }
        let tree = parent.child(name)?;
        Ok((parent, name, tree))
    }

    /// Where a tree goes: an existing parent that is not inside `source`, and a name
    /// that is free in it. With `same_allowed`, the name may be `source` itself under
    /// another spelling, as in a case-only rename on a case-insensitive filesystem.
    fn tree_target<'a>(
        &self,
        source: &Dir,
        to: &'a str,
        same_allowed: bool,
    ) -> Result<(Dir, &'a str), FsError> {
        let (directories, name) = split(to)?;
        let mut parent = self.open_root(false)?;
        for directory in &directories {
            parent = parent.child(directory)?;
            if parent.same(source)? {
                return Err(FsError::InvalidPath);
            }
        }
        match parent.stat(name) {
            Err(FsError::NotFound) => Ok((parent, name)),
            Ok(info) if same_allowed && info.is_dir() && parent.child(name)?.same(source)? => {
                Ok((parent, name))
            }
            Ok(_) => Err(FsError::InvalidPath),
            Err(error) => Err(error),
        }
    }

    /// Copies a tree into a new directory whose parent already exists. Refuses a
    /// source containing links or special files. Limits both entries and copied bytes.
    /// A failure removes what was copied.
    pub fn copy_tree(
        &self,
        path: &str,
        to: &str,
        max_entries: usize,
        max_bytes: u64,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<(), FsError> {
        self.writable()?;
        let (_source_parent, _, source) = self.tree_root(path)?;
        let entries = collect(&source, true, true, max_entries, cancelled)?;
        let mut bytes = max_bytes;
        for entry in &entries {
            let _ = names(&join(to, &entry.path), false)?;
            if entry.info.is_file() {
                bytes = bytes
                    .checked_sub(entry.info.bytes())
                    .ok_or(FsError::TooLarge)?;
            }
        }
        check_cancelled(cancelled)?;
        let (target_parent, target_name) = self.tree_target(&source, to, false)?;
        target_parent
            .create_dir(target_name)
            .map_err(|error| match error {
                FsError::Failed => FsError::InvalidPath,
                other => other,
            })?;
        let target = target_parent.child(target_name)?;
        let mut made = Vec::new();
        let result = (|| {
            for entry in &entries {
                check_cancelled(cancelled)?;
                let (parent, name) = parent_and_name(&entry.path);
                let from = below(&source, parent)?;
                let into = below(&target, parent)?;
                let current = from.stat(name)?;
                if current.is_dir() != entry.info.is_dir() {
                    return Err(FsError::Changed);
                }
                if current.is_dir() {
                    into.create_dir(name)?;
                    made.push((entry.path.as_str(), true));
                } else {
                    let input = from.open(name, Access::Read)?;
                    let output = into.create_new(name)?;
                    made.push((entry.path.as_str(), false));
                    copy_exact(input, output, entry.info.bytes(), cancelled)?;
                }
            }
            check_cancelled(cancelled)
        })();
        match &result {
            Ok(()) => target_parent.sync(),
            Err(_) => {
                for (path, directory) in made.into_iter().rev() {
                    let (parent, name) = parent_and_name(path);
                    if let Ok(parent) = below(&target, parent) {
                        let _ = if directory {
                            parent.remove_dir(name)
                        } else {
                            parent.remove_file(name)
                        };
                    }
                }
                drop(target);
                let _ = target_parent.remove_dir(target_name);
            }
        }
        result
    }

    /// Moves a bounded tree by renaming its root. The destination must be new, or the
    /// same directory spelled with another case, and on the same volume: a move to
    /// another volume fails with [`FsError::CrossDevice`] and changes nothing. Refuses a
    /// source containing links or special files.
    pub fn move_tree(
        &self,
        path: &str,
        to: &str,
        max_entries: usize,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<(), FsError> {
        self.writable()?;
        let (source_parent, source_name, source) = self.tree_root(path)?;
        let entries = collect(&source, true, true, max_entries, cancelled)?;
        for entry in entries {
            let _ = names(&join(to, &entry.path), false)?;
        }
        check_cancelled(cancelled)?;
        let (target_parent, target_name) = self.tree_target(&source, to, true)?;
        // The tree itself is renamed, so it cannot stay held; its parents do.
        drop(source);
        check_cancelled(cancelled)?;
        source_parent.rename(source_name, &target_parent, target_name)?;
        target_parent.sync();
        source_parent.sync();
        Ok(())
    }

    /// Deletes a bounded tree bottom-up. Refuses a source containing links or special
    /// files before changing it. Concurrently added entries prevent directory removal.
    pub fn remove_tree(
        &self,
        path: &str,
        max_entries: usize,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<(), FsError> {
        self.writable()?;
        let (parent, name, tree) = self.tree_root(path)?;
        let entries = collect(&tree, true, true, max_entries, cancelled)?;
        for entry in entries.into_iter().rev() {
            check_cancelled(cancelled)?;
            let (inner, entry_name) = parent_and_name(&entry.path);
            let directory = below(&tree, inner)?;
            let current = directory.stat(entry_name)?;
            if current.is_dir() != entry.info.is_dir() {
                return Err(FsError::Changed);
            }
            if current.is_dir() {
                directory.remove_dir(entry_name)?;
            } else {
                directory.remove_file(entry_name)?;
            }
        }
        check_cancelled(cancelled)?;
        drop(tree);
        parent.remove_dir(name)?;
        parent.sync();
        Ok(())
    }
}

/// The entries below `start`. `strict` refuses links, special files and names that are
/// not UTF-8 instead of skipping them, and keeps temporary files.
fn collect(
    start: &Dir,
    recursive: bool,
    strict: bool,
    max_entries: usize,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<TreeEntry>, FsError> {
    check_cancelled(cancelled)?;
    let mut pending = vec![String::new()];
    let mut entries = Vec::new();
    let mut remaining = max_entries;
    while let Some(prefix) = pending.pop() {
        let directory = below(start, &prefix).map_err(changed)?;
        let before = directory.info().map_err(changed)?;
        for name in directory.names().map_err(changed)? {
            check_cancelled(cancelled)?;
            remaining = remaining.checked_sub(1).ok_or(FsError::TooLarge)?;
            let Ok(name) = name.into_string() else {
                if strict {
                    return Err(FsError::InvalidPath);
                }
                continue;
            };
            if !strict && Folder::is_temporary(&name) {
                continue;
            }
            // A name the path rules refuse cannot be addressed afterwards.
            let plain = names(&name, false).is_ok_and(|parts| parts.len() == 1);
            let info = if plain {
                directory.stat(&name)
            } else {
                Err(FsError::InvalidPath)
            };
            match info {
                Ok(info) => {
                    let path = join(&prefix, &name);
                    if recursive && info.is_dir() {
                        pending.push(path.clone());
                    }
                    entries.push(TreeEntry { path, info });
                }
                Err(FsError::InvalidPath) if !strict => {}
                Err(error) => return Err(changed(error)),
            }
        }
        let after = directory.info().map_err(changed)?;
        if before.modified() != after.modified() {
            return Err(FsError::Changed);
        }
    }
    entries.sort_unstable_by(|left, right| left.path.cmp(&right.path));
    Ok(entries)
}

fn copy_exact(
    mut source: std::fs::File,
    mut output: std::fs::File,
    mut remaining: u64,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), FsError> {
    let mut buffer = [0; 8192];
    while remaining > 0 {
        check_cancelled(cancelled)?;
        let length = usize::try_from(remaining)
            .unwrap_or(usize::MAX)
            .min(buffer.len());
        let read = source.read(&mut buffer[..length]).map_err(|e| io(&e))?;
        if read == 0 {
            return Err(FsError::Changed);
        }
        output.write_all(&buffer[..read]).map_err(|e| io(&e))?;
        remaining -= read as u64;
    }
    if source.read(&mut buffer[..1]).map_err(|e| io(&e))? != 0 {
        return Err(FsError::Changed);
    }
    output.sync_all().map_err(|e| io(&e))
}
