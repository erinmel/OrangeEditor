//! Closing the gap between checking a path and using it.
//!
//! On Windows every directory on a checked path is held open without delete sharing for
//! as long as the operation runs, so none of them can be renamed, removed or replaced by
//! a junction in between. On every system the final file is opened without following a
//! link, and the opened handle is checked again.
//!
//! Elsewhere than Windows a directory can still be renamed while it is open, so a process
//! that can write to the folder can race an intermediate directory (05, "Files").

use super::{FsError, io, tree};
use std::fs::{File, OpenOptions};
use std::path::Path;

#[cfg(windows)]
mod flags {
    pub const FILE_LIST_DIRECTORY: u32 = 0x0001;
    pub const FILE_SHARE_READ: u32 = 0x0001;
    pub const FILE_SHARE_WRITE: u32 = 0x0002;
    pub const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    pub const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
}

/// Directories held open while an operation runs. Empty where holding a directory open
/// does not keep it in place.
#[derive(Debug, Default)]
pub(crate) struct Pins(Vec<File>);

impl Pins {
    /// Holds `directory` in place, refusing it if it is a link or not a directory.
    pub(crate) fn hold(&mut self, directory: &Path) -> Result<(), FsError> {
        #[cfg(windows)]
        {
            use flags::*;
            use std::os::windows::fs::OpenOptionsExt;
            // Listing access takes part in share checks; attribute access alone would not,
            // so a rename of the directory would not be refused.
            let handle = OpenOptions::new()
                .access_mode(FILE_LIST_DIRECTORY)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
                .open(directory)
                .map_err(|error| io(&error))?;
            let info = handle.metadata().map_err(|error| io(&error))?;
            if !info.is_dir() || tree::is_link(&info) {
                return Err(FsError::InvalidPath);
            }
            self.0.push(handle);
        }
        #[cfg(not(windows))]
        {
            let _ = directory;
        }
        Ok(())
    }

    /// Lets go of the last directory held, so the operation can remove or rename it.
    /// Removing or renaming a link that replaced it affects only the link.
    pub(crate) fn release_last(&mut self) {
        let _ = self.0.pop();
    }
}

/// Opens an existing regular file without following a link at its own name, and checks
/// the handle: whatever replaced the file after it was checked is refused.
pub(crate) fn open_file(options: &mut OpenOptions, path: &Path) -> Result<File, FsError> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let _ = options.custom_flags(flags::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let _ = options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path).map_err(|error| {
        #[cfg(unix)]
        if error.raw_os_error() == Some(libc::ELOOP) {
            return FsError::InvalidPath;
        }
        io(&error)
    })?;
    let info = file.metadata().map_err(|error| io(&error))?;
    if tree::is_link(&info) || !info.is_file() {
        return Err(FsError::InvalidPath);
    }
    Ok(file)
}

/// Opens an existing regular file for reading; see [`open_file`].
pub(crate) fn open_read(path: &Path) -> Result<File, FsError> {
    open_file(OpenOptions::new().read(true), path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("orange-pin-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("inner")).unwrap();
        root
    }

    /// A held directory cannot be renamed away and replaced while an operation runs.
    #[cfg(windows)]
    #[test]
    fn a_held_directory_cannot_be_renamed_until_released() {
        let root = scratch("hold");
        let inner = root.join("inner");
        let moved = root.join("moved");
        let mut held = Pins::default();
        held.hold(&root).unwrap();
        held.hold(&inner).unwrap();
        assert!(std::fs::rename(&inner, &moved).is_err());
        // Files inside a held directory are still created, replaced and removed.
        std::fs::write(inner.join("a.txt"), b"a").unwrap();
        std::fs::rename(inner.join("a.txt"), inner.join("b.txt")).unwrap();
        std::fs::remove_file(inner.join("b.txt")).unwrap();
        held.release_last();
        std::fs::rename(&inner, &moved).unwrap();
        drop(held);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_file_opens_and_a_directory_is_refused() {
        let root = scratch("open");
        std::fs::write(root.join("a.txt"), b"a").unwrap();
        assert!(open_read(&root.join("a.txt")).is_ok());
        assert!(open_read(&root.join("inner")).is_err());
        assert_eq!(
            open_read(&root.join("missing")).unwrap_err(),
            FsError::NotFound
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A link put at a checked file's name is not followed.
    #[cfg(unix)]
    #[test]
    fn a_link_at_the_file_name_is_refused() {
        let root = scratch("link");
        let outside = root.join("outside.txt");
        std::fs::write(&outside, b"secret").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("inner/a.txt")).unwrap();
        assert_eq!(
            open_read(&root.join("inner/a.txt")).unwrap_err(),
            FsError::InvalidPath
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
