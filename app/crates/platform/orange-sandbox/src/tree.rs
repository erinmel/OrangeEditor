//! Bounded operations on directory trees. Directory traversal excludes reparse points.

use super::pin::{Pins, open_read};
use super::{Folder, FsError, inside, io};
use std::fs::{self, Metadata};
use std::io::Write;
use std::path::{Path, PathBuf};

/// One addressable regular file or directory, relative to the directory that was listed.
#[derive(Debug)]
pub struct TreeEntry {
    pub path: String,
    pub metadata: Metadata,
}

pub(super) fn is_link(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

pub(super) fn metadata(path: &Path) -> Result<Metadata, FsError> {
    let metadata = fs::symlink_metadata(path).map_err(|e| io(&e))?;
    if is_link(&metadata) || !(metadata.is_file() || metadata.is_dir()) {
        return Err(FsError::InvalidPath);
    }
    Ok(metadata)
}

fn check_cancelled(cancelled: &dyn Fn() -> bool) -> Result<(), FsError> {
    if cancelled() {
        Err(FsError::Cancelled)
    } else {
        Ok(())
    }
}

impl Folder {
    /// Resolves a path without accepting symlinks, junctions or other reparse points
    /// at any component. A non-UTF-8 name is not addressable by the graph. Every
    /// directory on the path, the last one included, stays held while the returned
    /// [`Pins`] live, so keep them until the operation on the path is done.
    pub(super) fn checked(
        &self,
        path: &str,
        root_allowed: bool,
    ) -> Result<(PathBuf, Metadata, Pins), FsError> {
        let full = inside(&self.root, path, root_allowed)?;
        let mut pins = Pins::default();
        let mut prefix = self.root.clone();
        let mut info = metadata(&prefix)?;
        pins.hold(&prefix)?;
        for name in path.split('/').filter(|_| !path.is_empty()) {
            if !info.is_dir() {
                return Err(FsError::NotFound);
            }
            prefix.push(name);
            info = metadata(&prefix)?;
            if info.is_dir() {
                pins.hold(&prefix)?;
            }
        }
        self.contained(&full)?;
        Ok((full, info, pins))
    }

    /// Metadata for one addressable regular file or directory.
    pub fn metadata(&self, path: &str) -> Result<Metadata, FsError> {
        self.checked(path, true).map(|(_, info, _)| info)
    }

    /// Lists regular files and directories, skipping links and nonportable names.
    /// Every item examined, including skipped entries, counts against `max_entries`.
    pub fn tree(
        &self,
        path: &str,
        recursive: bool,
        max_entries: usize,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Vec<TreeEntry>, FsError> {
        self.collect_tree(path, recursive, false, max_entries, cancelled)
    }

    fn collect_tree(
        &self,
        path: &str,
        recursive: bool,
        strict: bool,
        max_entries: usize,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Vec<TreeEntry>, FsError> {
        check_cancelled(cancelled)?;
        let (_, info, _held) = self.checked(path, !strict)?;
        if !info.is_dir() {
            return Err(FsError::NotFound);
        }
        let mut directories = vec![String::new()];
        let mut entries = Vec::new();
        let mut remaining = max_entries;
        while let Some(prefix) = directories.pop() {
            let directory = join(path, &prefix);
            let (full, directory_info, _listed) =
                self.checked(&directory, !strict).map_err(changed)?;
            for entry in fs::read_dir(&full).map_err(|e| changed(io(&e)))? {
                check_cancelled(cancelled)?;
                remaining = remaining.checked_sub(1).ok_or(FsError::TooLarge)?;
                let entry = entry.map_err(|e| changed(io(&e)))?;
                let relative = entry
                    .file_name()
                    .into_string()
                    .ok()
                    .map(|name| join(&prefix, &name));
                let item = relative.ok_or(FsError::InvalidPath).and_then(|relative| {
                    let name = join(path, &relative);
                    let (_, metadata, _) = self.checked(&name, false).map_err(changed)?;
                    Ok(TreeEntry {
                        path: relative,
                        metadata,
                    })
                });
                match item {
                    Ok(item) => {
                        if !strict && entry.file_name().to_str().is_some_and(Self::is_temporary) {
                            continue;
                        }
                        if recursive && item.metadata.is_dir() {
                            directories.push(item.path.clone());
                        }
                        entries.push(item);
                    }
                    Err(FsError::InvalidPath) if !strict => {}
                    Err(error) => return Err(error),
                }
            }
            let after = metadata(&full).map_err(changed)?;
            if directory_info.modified().ok() != after.modified().ok() {
                return Err(FsError::Changed);
            }
        }
        entries.sort_unstable_by(|left, right| left.path.cmp(&right.path));
        Ok(entries)
    }

    /// A new path for a tree, outside `from`, with its parent's directories held.
    fn tree_target(&self, from: &Path, to: &str) -> Result<(PathBuf, Pins), FsError> {
        let full = inside(&self.root, to, false)?;
        let (parent, _) = to.rsplit_once('/').unwrap_or(("", to));
        let (parent, info, held) = self.checked(parent, true)?;
        let real_parent = parent.canonicalize().map_err(|e| io(&e))?;
        if !info.is_dir() || real_parent.starts_with(from) {
            return Err(FsError::InvalidPath);
        }
        match fs::symlink_metadata(&full) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok((full, held)),
            _ => Err(FsError::InvalidPath),
        }
    }

    /// Copies a tree into a new directory whose parent already exists. Refuses a
    /// source containing links or special files. Limits both entries and copied bytes.
    pub fn copy_tree(
        &self,
        path: &str,
        to: &str,
        max_entries: usize,
        max_bytes: u64,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<(), FsError> {
        self.writable()?;
        let (source, _, _source_held) = self.checked(path, false)?;
        let real_source = source.canonicalize().map_err(|e| io(&e))?;
        let _ = self.tree_target(&real_source, to)?;
        let entries = self.collect_tree(path, true, true, max_entries, cancelled)?;
        let mut bytes = max_bytes;
        for entry in &entries {
            let _ = inside(&self.root, &join(to, &entry.path), false)?;
            if entry.metadata.is_file() {
                bytes = bytes
                    .checked_sub(entry.metadata.len())
                    .ok_or(FsError::TooLarge)?;
            }
        }
        check_cancelled(cancelled)?;
        let (target, target_held) = self.tree_target(&real_source, to)?;
        fs::create_dir(target).map_err(|e| io(&e))?;
        drop(target_held);
        let mut made = Vec::new();
        let result = (|| {
            for entry in &entries {
                check_cancelled(cancelled)?;
                let source_name = join(path, &entry.path);
                let (source, current, _source_held) = self.checked(&source_name, false)?;
                let name = join(to, &entry.path);
                let parent = name.rsplit_once('/').ok_or(FsError::InvalidPath)?.0;
                let (_, _, _parent_held) = self.checked(parent, true)?;
                let target = inside(&self.root, &name, false)?;
                let directory = entry.metadata.is_dir();
                if current.is_dir() != directory {
                    return Err(FsError::Failed);
                }
                if directory {
                    fs::create_dir(&target).map_err(|e| io(&e))?;
                    made.push((name, true));
                } else {
                    let output = fs::File::create_new(&target).map_err(|e| io(&e))?;
                    made.push((name, false));
                    copy_exact(&source, output, entry.metadata.len(), cancelled)?;
                }
            }
            check_cancelled(cancelled)
        })();
        if result.is_err() {
            for (name, directory) in made.into_iter().rev() {
                if let Ok((target, _, mut held)) = self.checked(&name, false) {
                    let _ = if directory {
                        held.release_last();
                        fs::remove_dir(target)
                    } else {
                        fs::remove_file(target)
                    };
                }
            }
            if let Ok((target, _, mut held)) = self.checked(to, false) {
                held.release_last();
                let _ = fs::remove_dir(target);
            }
        }
        result
    }

    /// Moves a bounded tree by renaming its root. The destination must be new and
    /// on the same filesystem. Refuses a source containing links or special files.
    pub fn move_tree(
        &self,
        path: &str,
        to: &str,
        max_entries: usize,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<(), FsError> {
        self.writable()?;
        let entries = self.collect_tree(path, true, true, max_entries, cancelled)?;
        for entry in entries {
            let _ = inside(&self.root, &join(to, &entry.path), false)?;
        }
        check_cancelled(cancelled)?;
        let (source, _, mut source_held) = self.checked(path, false)?;
        let (target, _target_held) =
            self.tree_target(&source.canonicalize().map_err(|e| io(&e))?, to)?;
        check_cancelled(cancelled)?;
        // The tree itself is renamed, so it cannot stay held; its parents do.
        source_held.release_last();
        fs::rename(source, target).map_err(|e| io(&e))
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
        let entries = self.collect_tree(path, true, true, max_entries, cancelled)?;
        for entry in entries.into_iter().rev() {
            check_cancelled(cancelled)?;
            let (target, current, mut held) = self.checked(&join(path, &entry.path), false)?;
            if current.is_dir() != entry.metadata.is_dir() {
                return Err(FsError::Failed);
            }
            if current.is_dir() {
                held.release_last();
                fs::remove_dir(target).map_err(|e| io(&e))?;
            } else {
                fs::remove_file(target).map_err(|e| io(&e))?;
            }
        }
        check_cancelled(cancelled)?;
        let (target, _, mut held) = self.checked(path, false)?;
        held.release_last();
        fs::remove_dir(target).map_err(|e| io(&e))
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

fn copy_exact(
    source: &Path,
    mut output: fs::File,
    mut remaining: u64,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), FsError> {
    use std::io::Read;
    let mut source = open_read(source)?;
    let mut buffer = [0; 8192];
    while remaining > 0 {
        check_cancelled(cancelled)?;
        let length = usize::try_from(remaining)
            .unwrap_or(usize::MAX)
            .min(buffer.len());
        let read = source.read(&mut buffer[..length]).map_err(|e| io(&e))?;
        if read == 0 {
            return Err(FsError::Failed);
        }
        output.write_all(&buffer[..read]).map_err(|e| io(&e))?;
        remaining -= read as u64;
    }
    if source.read(&mut buffer[..1]).map_err(|e| io(&e))? != 0 {
        return Err(FsError::Failed);
    }
    output.sync_all().map_err(|e| io(&e))
}
