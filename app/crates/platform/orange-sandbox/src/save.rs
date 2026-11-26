//! Replacing a file the host owns, such as a saved document or a settings file.

use std::io::{self, Write as _};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNT: AtomicU64 = AtomicU64::new(0);

/// Atomically replaces `target` with `bytes` on filesystems that support atomic rename.
///
/// The bytes go to a new temporary file beside the target, created exclusively so a
/// file or link already at that name is never written through. It is flushed to disk
/// and renamed over the target. A failure before the rename leaves the previous file
/// in place and attempts to clean up the temporary file. On Unix a directory flush
/// is attempted after replacement. Success reports that the rename committed, with
/// crash durability dependent on the filesystem and its flush guarantees.
///
/// The parent directory must exist. The new file has default permissions; on
/// Windows a read-only target or one another program holds open without delete
/// sharing is refused, with the previous file kept.
pub fn replace_file(target: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::hash::{BuildHasher, RandomState};
    if target.file_name().is_none() {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let directory = target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let unpredictable = RandomState::new();
    for _ in 0..16 {
        let count = COUNT.fetch_add(1, Ordering::Relaxed);
        let temporary = directory.join(format!(
            ".orange-save-{}-{count}-{:016x}.tmp",
            std::process::id(),
            unpredictable.hash_one(count)
        ));
        let mut file = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        let written = file
            .write_all(bytes)
            .and_then(|()| file.sync_all())
            .and_then(|()| {
                drop(file);
                std::fs::rename(&temporary, target)
            });
        if written.is_err() {
            let _ = std::fs::remove_file(&temporary);
            return written;
        }
        sync_directory(directory);
        return Ok(());
    }
    Err(io::ErrorKind::AlreadyExists.into())
}

#[cfg(unix)]
fn sync_directory(directory: &Path) {
    if let Ok(handle) = std::fs::File::open(directory) {
        let _ = handle.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_directory(_directory: &Path) {}
