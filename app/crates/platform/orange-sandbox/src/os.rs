//! Directory handles: the one place a checked directory turns into a system call.
//!
//! Every operation of a [`super::Folder`] walks from the root to the directory it works
//! in, one plain name at a time, and then acts on a single name inside that directory.
//! A [`Dir`] is the directory reached, held so that what was checked is what is used:
//!
//! - On Linux and macOS it is an open directory descriptor. Each step opens the next
//!   name relative to the previous descriptor with `O_NOFOLLOW | O_DIRECTORY`, and the
//!   final call is the `*at` form relative to the last one (`openat`, `mkdirat`,
//!   `renameat`, `unlinkat`, `fstatat`). Renaming or replacing a directory on the path
//!   after it was opened does not change which directory the descriptor names, so an
//!   operation cannot be redirected outside the folder.
//! - On Windows every directory from the root down is held open without delete
//!   sharing and with `FILE_FLAG_OPEN_REPARSE_POINT`, so none of them can be renamed,
//!   removed or replaced by a junction while the operation runs, and the final call is
//!   made on the path those held directories pin.
//!
//! On both, the final file is opened without following a link at its name and the
//! opened handle is checked again.

use super::{FsError, Info};
use std::path::Path;
#[cfg(windows)]
use std::{ffi::OsString, fs::File, path::PathBuf};

/// Maps an operating-system error to the typed refusal a caller can act on. No text
/// from the system is kept, so nothing about paths outside the folder leaks.
pub(crate) fn io(error: &std::io::Error) -> FsError {
    use std::io::ErrorKind as Kind;
    #[cfg(windows)]
    {
        // ERROR_SHARING_VIOLATION and ERROR_LOCK_VIOLATION: another program has the
        // file open without sharing it, or holds a lock on a range of it.
        // ERROR_WRITE_PROTECT: removable media with its write protection on.
        match error.raw_os_error() {
            Some(32 | 33) => return FsError::Busy,
            Some(19) => return FsError::ReadOnlyLocation,
            _ => {}
        }
    }
    #[cfg(unix)]
    if error.raw_os_error() == Some(rustix::io::Errno::LOOP.raw_os_error()) {
        return FsError::InvalidPath;
    }
    match error.kind() {
        Kind::NotFound => FsError::NotFound,
        Kind::PermissionDenied => FsError::PermissionDenied,
        Kind::ReadOnlyFilesystem => FsError::ReadOnlyLocation,
        Kind::StorageFull | Kind::QuotaExceeded => FsError::NoSpace,
        Kind::CrossesDevices => FsError::CrossDevice,
        Kind::ResourceBusy | Kind::ExecutableFileBusy => FsError::Busy,
        Kind::Unsupported => FsError::Unsupported,
        Kind::NotADirectory => FsError::NotFound,
        _ => FsError::Failed,
    }
}

#[cfg(unix)]
fn errno(error: rustix::io::Errno) -> FsError {
    io(&std::io::Error::from(error))
}

/// A directory inside the folder, held for as long as the value lives.
#[derive(Debug)]
pub(crate) struct Dir {
    #[cfg(unix)]
    fd: std::os::fd::OwnedFd,
    #[cfg(windows)]
    path: PathBuf,
    #[cfg(windows)]
    held: Vec<File>,
}

/// How [`Dir::open`] opens an existing or new regular file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Access {
    /// An existing file, for reading.
    Read,
    /// For appending, created if missing.
    Append,
}

#[cfg(windows)]
mod flags {
    pub const FILE_LIST_DIRECTORY: u32 = 0x0001;
    pub const FILE_SHARE_READ: u32 = 0x0001;
    pub const FILE_SHARE_WRITE: u32 = 0x0002;
    pub const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    pub const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
}

#[cfg(windows)]
pub(crate) fn is_link(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & REPARSE_POINT != 0
}

#[cfg(unix)]
mod unix {
    use super::{Access, Dir, FsError, Info, errno};
    use rustix::fs::{AtFlags, FileType, Mode, OFlags};
    use std::ffi::OsString;
    use std::fs::File;
    use std::os::fd::OwnedFd;
    use std::os::unix::ffi::OsStrExt as _;
    use std::path::Path;

    const FILE_MODE: u32 = 0o666;
    const DIRECTORY_MODE: u32 = 0o777;

    fn directory_flags() -> OFlags {
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC
    }

    #[expect(
        clippy::unnecessary_cast,
        clippy::useless_conversion,
        reason = "stat field types differ between Linux and macOS"
    )]
    fn info(stat: &rustix::fs::Stat) -> Result<Info, FsError> {
        let kind = FileType::from_raw_mode(stat.st_mode);
        let directory = match kind {
            FileType::Directory => true,
            FileType::RegularFile => false,
            // A link or a special file is never addressable.
            _ => return Err(FsError::InvalidPath),
        };
        let modified = i64::try_from(stat.st_mtime)
            .ok()
            .zip(u32::try_from(stat.st_mtime_nsec).ok())
            .and_then(|(seconds, nanos)| {
                let after = std::time::Duration::new(seconds.unsigned_abs(), nanos);
                if seconds >= 0 {
                    std::time::UNIX_EPOCH.checked_add(after)
                } else {
                    std::time::UNIX_EPOCH.checked_sub(after)
                }
            });
        Ok(Info {
            directory,
            bytes: if directory {
                0
            } else {
                u64::try_from(stat.st_size).unwrap_or(0)
            },
            modified,
            read_only: (stat.st_mode as u32) & 0o222 == 0,
            identity: Some((stat.st_dev as u64, stat.st_ino as u64, 0)),
        })
    }

    impl Dir {
        pub(crate) fn root(root: &Path) -> Result<Self, FsError> {
            // The root is the capability the host supplied, so links in its own path
            // are followed; nothing below it is.
            let fd = rustix::fs::open(
                root,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(errno)?;
            Ok(Self { fd })
        }

        pub(crate) fn child(&self, name: &str) -> Result<Self, FsError> {
            match rustix::fs::openat(&self.fd, name, directory_flags(), Mode::empty()) {
                Ok(fd) => Ok(Self { fd }),
                Err(error)
                    if error == rustix::io::Errno::LOOP || error == rustix::io::Errno::NOTDIR =>
                {
                    // A regular file is not a directory to enter; a link is refused.
                    match self.stat(name) {
                        Ok(info) if !info.directory => Err(FsError::NotFound),
                        Ok(_) => Err(FsError::Changed),
                        Err(error) => Err(error),
                    }
                }
                Err(error) => Err(errno(error)),
            }
        }

        pub(crate) fn create_dir(&self, name: &str) -> Result<(), FsError> {
            rustix::fs::mkdirat(&self.fd, name, Mode::from_raw_mode(DIRECTORY_MODE as _))
                .map_err(errno)
        }

        pub(crate) fn stat(&self, name: &str) -> Result<Info, FsError> {
            let stat =
                rustix::fs::statat(&self.fd, name, AtFlags::SYMLINK_NOFOLLOW).map_err(errno)?;
            info(&stat)
        }

        pub(crate) fn info(&self) -> Result<Info, FsError> {
            info(&rustix::fs::fstat(&self.fd).map_err(errno)?)
        }

        pub(crate) fn open(&self, name: &str, access: Access) -> Result<File, FsError> {
            let flags = match access {
                Access::Read => OFlags::RDONLY,
                Access::Append => OFlags::WRONLY | OFlags::APPEND | OFlags::CREATE,
            };
            // Non-blocking, so a FIFO put at the name cannot hang the open; the type
            // check below refuses it.
            let fd = rustix::fs::openat(
                &self.fd,
                name,
                flags | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
                Mode::from_raw_mode(FILE_MODE as _),
            )
            .map_err(errno)?;
            regular(fd)
        }

        pub(crate) fn create_new(&self, name: &str) -> Result<File, FsError> {
            let fd = rustix::fs::openat(
                &self.fd,
                name,
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::from_raw_mode(FILE_MODE as _),
            )
            .map_err(errno)?;
            regular(fd)
        }

        pub(crate) fn rename(&self, from: &str, to: &Self, name: &str) -> Result<(), FsError> {
            rustix::fs::renameat(&self.fd, from, &to.fd, name).map_err(errno)
        }

        pub(crate) fn remove_file(&self, name: &str) -> Result<(), FsError> {
            rustix::fs::unlinkat(&self.fd, name, AtFlags::empty()).map_err(errno)
        }

        pub(crate) fn remove_dir(&self, name: &str) -> Result<(), FsError> {
            rustix::fs::unlinkat(&self.fd, name, AtFlags::REMOVEDIR).map_err(errno)
        }

        pub(crate) fn names(&self) -> Result<Vec<OsString>, FsError> {
            let mut names = Vec::new();
            for entry in rustix::fs::Dir::read_from(&self.fd).map_err(errno)? {
                let entry = entry.map_err(errno)?;
                let name = entry.file_name().to_bytes();
                if name != b"." && name != b".." {
                    names.push(std::ffi::OsStr::from_bytes(name).to_owned());
                }
            }
            Ok(names)
        }

        /// Flushes the directory's entries, so a rename in it survives a crash.
        /// Some filesystems do not support it; the rename is still atomic there.
        pub(crate) fn sync(&self) {
            let _ = rustix::fs::fsync(&self.fd);
        }

        pub(crate) fn same(&self, other: &Self) -> Result<bool, FsError> {
            Ok(self.info()?.identity == other.info()?.identity)
        }

        /// Another handle to the same directory.
        pub(crate) fn duplicate(&self) -> Result<Self, FsError> {
            Ok(Self {
                fd: self.fd.try_clone().map_err(|error| super::io(&error))?,
            })
        }
    }

    fn regular(fd: OwnedFd) -> Result<File, FsError> {
        let file = File::from(fd);
        let stat = rustix::fs::fstat(&file).map_err(errno)?;
        if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
            return Err(FsError::InvalidPath);
        }
        Ok(file)
    }
}

#[cfg(windows)]
impl Dir {
    fn hold(path: &Path) -> Result<File, FsError> {
        use flags::*;
        use std::os::windows::fs::OpenOptionsExt;
        // Listing access takes part in share checks; attribute access alone would not,
        // so a rename of the directory would not be refused.
        let handle = std::fs::OpenOptions::new()
            .access_mode(FILE_LIST_DIRECTORY)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
            .map_err(|error| io(&error))?;
        let info = handle.metadata().map_err(|error| io(&error))?;
        if is_link(&info) {
            return Err(FsError::InvalidPath);
        }
        if !info.is_dir() {
            return Err(if info.is_file() {
                FsError::NotFound
            } else {
                FsError::InvalidPath
            });
        }
        Ok(handle)
    }

    pub(crate) fn root(root: &Path) -> Result<Self, FsError> {
        Ok(Self {
            path: root.to_path_buf(),
            held: vec![Self::hold(root)?],
        })
    }

    pub(crate) fn child(&self, name: &str) -> Result<Self, FsError> {
        let path = self.path.join(name);
        let handle = Self::hold(&path)?;
        let mut child = self.duplicate()?;
        child.held.push(handle);
        child.path = path;
        Ok(child)
    }

    /// Another handle to the same directory, holding the same ones above it.
    pub(crate) fn duplicate(&self) -> Result<Self, FsError> {
        let mut held = Vec::with_capacity(self.held.len() + 1);
        for handle in &self.held {
            held.push(handle.try_clone().map_err(|error| io(&error))?);
        }
        Ok(Self {
            path: self.path.clone(),
            held,
        })
    }

    pub(crate) fn create_dir(&self, name: &str) -> Result<(), FsError> {
        std::fs::create_dir(self.path.join(name)).map_err(|error| io(&error))
    }

    pub(crate) fn stat(&self, name: &str) -> Result<Info, FsError> {
        let metadata = std::fs::symlink_metadata(self.path.join(name)).map_err(|e| io(&e))?;
        Info::from_metadata(&metadata)
    }

    pub(crate) fn info(&self) -> Result<Info, FsError> {
        let handle = self.held.last().ok_or(FsError::Failed)?;
        Info::from_metadata(&handle.metadata().map_err(|error| io(&error))?)
    }

    fn checked(file: File) -> Result<File, FsError> {
        let info = file.metadata().map_err(|error| io(&error))?;
        if is_link(&info) || !info.is_file() {
            return Err(FsError::InvalidPath);
        }
        Ok(file)
    }

    pub(crate) fn open(&self, name: &str, access: Access) -> Result<File, FsError> {
        use std::os::windows::fs::OpenOptionsExt;
        let mut options = std::fs::OpenOptions::new();
        let _ = match access {
            Access::Read => options.read(true),
            Access::Append => options.read(true).append(true).create(true),
        };
        let _ = options.custom_flags(flags::FILE_FLAG_OPEN_REPARSE_POINT);
        Self::checked(
            options
                .open(self.path.join(name))
                .map_err(|error| io(&error))?,
        )
    }

    pub(crate) fn create_new(&self, name: &str) -> Result<File, FsError> {
        use std::os::windows::fs::OpenOptionsExt;
        Self::checked(
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .custom_flags(flags::FILE_FLAG_OPEN_REPARSE_POINT)
                .open(self.path.join(name))
                .map_err(|error| io(&error))?,
        )
    }

    pub(crate) fn rename(&self, from: &str, to: &Self, name: &str) -> Result<(), FsError> {
        std::fs::rename(self.path.join(from), to.path.join(name)).map_err(|e| io(&e))
    }

    pub(crate) fn remove_file(&self, name: &str) -> Result<(), FsError> {
        std::fs::remove_file(self.path.join(name)).map_err(|error| io(&error))
    }

    pub(crate) fn remove_dir(&self, name: &str) -> Result<(), FsError> {
        std::fs::remove_dir(self.path.join(name)).map_err(|error| io(&error))
    }

    pub(crate) fn names(&self) -> Result<Vec<OsString>, FsError> {
        let mut names = Vec::new();
        for entry in std::fs::read_dir(&self.path).map_err(|error| io(&error))? {
            names.push(entry.map_err(|error| io(&error))?.file_name());
        }
        Ok(names)
    }

    /// Windows offers no directory flush through std; NTFS journals the rename.
    pub(crate) fn sync(&self) {}

    /// Whether both name the same directory. Every directory on both paths is held, so
    /// their resolved paths cannot change underneath the comparison.
    pub(crate) fn same(&self, other: &Self) -> Result<bool, FsError> {
        let left = self.path.canonicalize().map_err(|error| io(&error))?;
        let right = other.path.canonicalize().map_err(|error| io(&error))?;
        Ok(left == right)
    }
}

#[cfg(windows)]
impl Info {
    pub(crate) fn from_metadata(metadata: &std::fs::Metadata) -> Result<Self, FsError> {
        use std::os::windows::fs::MetadataExt;
        if is_link(metadata) || !(metadata.is_file() || metadata.is_dir()) {
            return Err(FsError::InvalidPath);
        }
        let created = metadata.creation_time();
        Ok(Self {
            directory: metadata.is_dir(),
            bytes: if metadata.is_dir() { 0 } else { metadata.len() },
            modified: metadata.modified().ok(),
            read_only: metadata.permissions().readonly(),
            // Stable std does not expose the NTFS file index. Creation time, last
            // write time and size together stand in for it.
            identity: (created != 0).then_some((
                created,
                metadata.last_write_time(),
                metadata.len(),
            )),
        })
    }
}

#[cfg(not(any(unix, windows)))]
compile_error!("orange-sandbox supports Unix and Windows targets only");

/// Whether `path` names a file or directory; used where a [`Dir`] cannot be had.
pub(crate) fn exists(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}
