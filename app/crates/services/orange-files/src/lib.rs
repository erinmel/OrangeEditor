//! The base file package: reading, writing, listing and watching files in a folder.
//!
//! It is an ordinary [`ServicePackage`], built on the plugin SDK alone. A document
//! declares an `org.orange.files/folder` connection naming a directory, and the nodes
//! below work on paths relative to it. A node can name nothing outside its folder: a
//! path is `/`-separated plain names, and `..`, absolute and drive forms, device names
//! and links that lead out of the folder are refused. An application that does not
//! want file access leaves the package out.
//!
//! A file larger than that is handled in pieces: `file-info`, `read-range` and
//! `append-bytes` let a graph walk through it with bounded memory, and `copy` and
//! `move` have no size limit because they never hold the content.
//!
//! A whole file is read or written at once, up to [`MAX_FILE_BYTES`]. Writing replaces
//! a file atomically: the content goes to a temporary file in the same directory, which
//! is then renamed over the target, so a reader or a crash sees the old file or the new
//! one. Files move, copy and get directories inside the folder. Streams for larger
//! files are not provided.
// Product code reports failures as typed errors; each remaining panic is a documented
// internal invariant (10, "Panic policy").
#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]

use orange_sandbox::FsError;
use orange_sdk::{
    Call, ConnectionAdapter, Definition, KernelFault, ServiceNode, ServicePackage, Session, Value,
};
use serde_json::{Map, Value as Json, json};
use std::any::Any;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

mod glob;
mod watch;

include!(concat!(env!("OUT_DIR"), "/locales.rs"));

/// Package namespace of the definitions and adapter kind here.
pub const PACKAGE: &str = "org.orange.files";
/// Adapter kind of a folder. Settings: `path` (required, an existing directory),
/// `read_only` (default `false`; writing nodes then fail) and `poll_ms` (50 to 60,000,
/// default 500), how often [`ON_CHANGE`] looks for changes.
pub const FOLDER_ADAPTER: &str = "org.orange.files/folder";
/// Reads a UTF-8 text file. Input `path`; output `text`.
pub const READ_TEXT: &str = "org.orange.files/read-text";
/// Creates or replaces a text file. Inputs `path` and `text`.
pub const WRITE_TEXT: &str = "org.orange.files/write-text";
/// Adds text to the end of a file, creating it if needed. Inputs `path` and `text`.
pub const APPEND_TEXT: &str = "org.orange.files/append-text";
/// Reads a file as bytes. Input `path`; output `data`.
pub const READ_BYTES: &str = "org.orange.files/read-bytes";
/// Creates or replaces a file from bytes. Inputs `path` and `data`.
pub const WRITE_BYTES: &str = "org.orange.files/write-bytes";
/// Lists a directory. Inputs `path` (empty for the folder itself), `pattern` (a glob,
/// empty for everything) and `recursive`; output `names`, sorted, a directory's name
/// ending with `/`.
pub const LIST: &str = "org.orange.files/list";
/// Says whether a file or directory exists. Input `path`; output `found`.
pub const EXISTS: &str = "org.orange.files/exists";
/// Deletes a file. Input `path`.
pub const DELETE: &str = "org.orange.files/delete";
/// Moves or renames a file inside the folder. Inputs `path` and `to`; a file already
/// at `to` is replaced.
pub const MOVE: &str = "org.orange.files/move";
/// Copies a file inside the folder. Inputs `path` and `to`; a file already at `to` is
/// replaced.
pub const COPY: &str = "org.orange.files/copy";
/// Creates a directory and the directories on its path. Input `path`.
pub const MAKE_DIRECTORY: &str = "org.orange.files/make-directory";
/// Copies a bounded directory tree to a new path. Inputs `path` and `to`.
pub const COPY_DIRECTORY: &str = "org.orange.files/copy-directory";
/// Moves a bounded directory tree to a new path. Inputs `path` and `to`.
pub const MOVE_DIRECTORY: &str = "org.orange.files/move-directory";
/// Deletes a bounded directory tree. Input `path`.
pub const DELETE_DIRECTORY: &str = "org.orange.files/delete-directory";
/// Reports metadata for a file or directory. Input `path`.
pub const FILE_INFO: &str = "org.orange.files/file-info";
/// Entry fired when a file or folder in the watched folder is created, changed,
/// deleted or renamed. Input `folder` narrows it to a subfolder (empty for the whole
/// connection); the payload is a [`CHANGE_TYPE`] record. Changes are found by
/// comparing sizes and modification times every `poll_ms`, so several changes to one
/// file within that time fire once.
pub const ON_CHANGE: &str = "org.orange.files/on-change";
/// The payload of [`ON_CHANGE`]: `kind` (`created`, `changed`, `deleted` or
/// `renamed`), `path`, `previous_path` (empty unless renamed) and `is_directory`.
pub const CHANGE_TYPE: &str = "org.orange.files/change@1";

/// Largest file a node reads or writes.
pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
pub use orange_sandbox::MAX_PATH_BYTES;
/// Most entries one listing returns and one watched folder may hold.
pub const MAX_ENTRIES: usize = 10_000;
/// Largest aggregate file size copied by one Copy folder operation.
pub const MAX_COPY_TREE_BYTES: u64 = 1024 * 1024 * 1024;
const DEFAULT_POLL_MS: u64 = 500;

const NO_SESSION: KernelFault = KernelFault::from_static(
    "org.orange.files/Connection",
    "org.orange.files/connection-kind",
    "The node needs a folder connection.",
);
const BAD_INPUT: KernelFault = KernelFault::from_static(
    "org.orange.files/Input",
    "org.orange.files/input-invalid",
    "The node's inputs are not of the expected types.",
);
const BAD_PATH: KernelFault = KernelFault::from_static(
    "org.orange.files/Path",
    "org.orange.files/path-invalid",
    "The path is not a relative path inside the folder.",
);
const NOT_FOUND: KernelFault = KernelFault::from_static(
    "org.orange.files/NotFound",
    "org.orange.files/not-found",
    "The file or directory does not exist.",
);
const TOO_LARGE: KernelFault = KernelFault::from_static(
    "org.orange.files/TooLarge",
    "org.orange.files/too-large",
    "The file or listing is over the size limit.",
);
const NOT_TEXT: KernelFault = KernelFault::from_static(
    "org.orange.files/Encoding",
    "org.orange.files/not-utf8",
    "The file is not UTF-8 text.",
);
const READ_ONLY: KernelFault = KernelFault::from_static(
    "org.orange.files/ReadOnly",
    "org.orange.files/read-only",
    "The folder connection is read-only.",
);
const FAILED: KernelFault = KernelFault::from_static(
    "org.orange.files/Io",
    "org.orange.files/io-failed",
    "The operating system refused the file operation.",
);
const INVALID_GLOB: KernelFault = KernelFault::from_static(
    "org.orange.files/Glob",
    "org.orange.files/glob-invalid",
    "The glob must contain plain path components and be at most 128 bytes.",
);
const CANCELLED: KernelFault = KernelFault::from_static(
    "org.orange.files/Cancelled",
    "org.orange.files/operation-cancelled",
    "The folder operation was cancelled.",
);
const TREE_CHANGED: KernelFault = KernelFault::from_static(
    "org.orange.files/TreeChanged",
    "org.orange.files/tree-changed",
    "The folder changed while it was being read.",
);

fn refused(error: FsError) -> KernelFault {
    match error {
        FsError::NotFound => NOT_FOUND,
        FsError::InvalidPath => BAD_PATH,
        FsError::TooLarge => TOO_LARGE,
        FsError::ReadOnly => READ_ONLY,
        FsError::Cancelled => CANCELLED,
        FsError::Changed => TREE_CHANGED,
        _ => FAILED,
    }
}

/// A folder connection: the shared sandbox with this package's limits and faults.
#[derive(Debug)]
struct Folder {
    files: orange_sandbox::Folder,
    poll: Duration,
}
impl Session for Folder {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Folder {
    fn read(&self, path: &str) -> Result<Vec<u8>, KernelFault> {
        self.files.read(path, MAX_FILE_BYTES).map_err(refused)
    }
    fn write(&self, path: &str, data: &[u8]) -> Result<(), KernelFault> {
        if data.len() as u64 > MAX_FILE_BYTES {
            return Err(TOO_LARGE);
        }
        self.files.write(path, data).map_err(refused)
    }
    fn append(&self, path: &str, data: &[u8]) -> Result<(), KernelFault> {
        self.files
            .append(path, data, MAX_FILE_BYTES)
            .map_err(refused)
    }
    fn read_range(&self, path: &str, offset: u64, length: u64) -> Result<Vec<u8>, KernelFault> {
        self.files
            .read_range(path, offset, length, MAX_FILE_BYTES)
            .map_err(refused)
    }
    /// Adds one piece of at most [`MAX_FILE_BYTES`] to a file of any size.
    fn append_piece(&self, path: &str, data: &[u8]) -> Result<(), KernelFault> {
        if data.len() as u64 > MAX_FILE_BYTES {
            return Err(TOO_LARGE);
        }
        self.files.append(path, data, u64::MAX).map_err(refused)
    }
    fn exists(&self, path: &str) -> Result<bool, KernelFault> {
        self.files.exists(path).map_err(refused)
    }
    fn delete(&self, path: &str) -> Result<(), KernelFault> {
        self.files.remove(path).map_err(refused)
    }
    fn rename(&self, path: &str, to: &str) -> Result<(), KernelFault> {
        self.files.rename(path, to).map_err(refused)
    }
    fn copy(&self, path: &str, to: &str) -> Result<(), KernelFault> {
        self.files.copy(path, to, u64::MAX).map_err(refused)
    }
    fn make_directory(&self, path: &str) -> Result<(), KernelFault> {
        self.files.make_directory(path).map_err(refused)
    }

    fn list_filtered(
        &self,
        path: &str,
        pattern: &str,
        recursive: bool,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Vec<Arc<String>>, KernelFault> {
        let pattern = glob::Glob::parse(pattern).ok_or(INVALID_GLOB)?;
        let entries = self
            .files
            .tree(path, recursive, MAX_ENTRIES, cancelled)
            .map_err(refused)?;
        let mut names: Vec<_> = entries
            .into_iter()
            .filter(|entry| pattern.matches(&entry.path))
            .map(|entry| {
                let mut path = entry.path;
                if entry.metadata.is_dir() {
                    path.push('/');
                }
                Arc::new(path)
            })
            .collect();
        names.sort_unstable();
        Ok(names)
    }
}

fn unix_millis(time: SystemTime) -> Option<i64> {
    match time.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_millis()).ok(),
        Err(before) => i64::try_from(before.duration().as_millis())
            .ok()
            .map(|ms| -ms),
    }
}

#[derive(Debug)]
struct FolderAdapter;
impl ConnectionAdapter for FolderAdapter {
    fn open(&self, settings: &Map<String, Json>) -> Result<Arc<dyn Session>, String> {
        let path = settings
            .get("path")
            .and_then(Json::as_str)
            .filter(|path| !path.is_empty())
            .ok_or("`path` is required")?;
        let root = Path::new(path)
            .canonicalize()
            .ok()
            .filter(|root| root.is_dir())
            .ok_or("`path` is not an existing directory")?;
        let read_only = match settings.get("read_only") {
            None => false,
            Some(Json::Bool(read_only)) => *read_only,
            Some(_) => return Err("`read_only` must be true or false".into()),
        };
        let poll = match settings.get("poll_ms") {
            None => DEFAULT_POLL_MS,
            Some(value) => value
                .as_u64()
                .filter(|ms| (50..=60_000).contains(ms))
                .ok_or("`poll_ms` must be an integer from 50 to 60000")?,
        };
        Ok(Arc::new(Folder {
            files: orange_sandbox::Folder::new(root, read_only),
            poll: Duration::from_millis(poll),
        }))
    }
    fn about(&self) -> orange_sdk::AdapterAbout {
        orange_sdk::AdapterAbout {
            label: "Folder".into(),
            purpose: "Reads, writes and watches the files of one folder, and nothing outside it."
                .into(),
        }
    }
    fn texts(&self) -> Vec<orange_sdk::SettingText> {
        vec![orange_sdk::SettingText {
            setting: "path".into(),
            label: "Folder".into(),
            help: "The folder's path on this machine.".into(),
            required: true,
            ..orange_sdk::SettingText::default()
        }]
    }
    fn toggles(&self) -> Vec<orange_sdk::SettingToggle> {
        vec![orange_sdk::SettingToggle {
            setting: "read_only".into(),
            label: "Read only".into(),
            help: "Nodes can read the folder but not change it.".into(),
            default: false,
        }]
    }
    #[expect(clippy::cast_precision_loss, reason = "a small number of milliseconds")]
    fn fields(&self) -> Vec<orange_sdk::SettingField> {
        vec![orange_sdk::SettingField {
            setting: "poll_ms".into(),
            label: "Check every".into(),
            help: "How often the folder is looked at for changes.".into(),
            unit: "ms".into(),
            min: 50.0,
            max: 60_000.0,
            default: Some(DEFAULT_POLL_MS as f64),
            whole: true,
        }]
    }
}

fn folder(session: Option<&dyn Session>) -> Option<&Folder> {
    session?.as_any().downcast_ref::<Folder>()
}

/// What a file node does.
#[derive(Debug, Clone, Copy)]
enum Operation {
    ReadText,
    WriteText,
    AppendText,
    ReadBytes,
    WriteBytes,
    List,
    Exists,
    Delete,
    Move,
    Copy,
    MakeDirectory,
    ReadRange,
    AppendBytes,
    CopyDirectory,
    MoveDirectory,
    DeleteDirectory,
    FileInfo,
}
impl ServiceNode for Operation {
    fn run(&self, call: &Call<'_>) -> Result<Vec<Value>, KernelFault> {
        let folder = folder(call.session).ok_or(NO_SESSION)?;
        let cancelled = || call.lease.is_cancelled();
        match (self, call.inputs) {
            (Self::ReadText, [Value::Str(path)]) => {
                let text = String::from_utf8(folder.read(path)?).map_err(|_| NOT_TEXT)?;
                Ok(vec![Value::Str(Arc::new(text))])
            }
            (Self::ReadBytes, [Value::Str(path)]) => {
                Ok(vec![Value::Bytes(Arc::new(folder.read(path)?))])
            }
            (Self::WriteText, [Value::Str(path), Value::Str(text)]) => {
                folder.write(path, text.as_bytes()).map(|()| Vec::new())
            }
            (Self::AppendText, [Value::Str(path), Value::Str(text)]) => {
                folder.append(path, text.as_bytes()).map(|()| Vec::new())
            }
            (Self::WriteBytes, [Value::Str(path), Value::Bytes(data)]) => {
                folder.write(path, data).map(|()| Vec::new())
            }
            (Self::Exists, [Value::Str(path)]) => Ok(vec![Value::Bool(folder.exists(path)?)]),
            (Self::Delete, [Value::Str(path)]) => folder.delete(path).map(|()| Vec::new()),
            (Self::Move, [Value::Str(path), Value::Str(to)]) => {
                folder.rename(path, to).map(|()| Vec::new())
            }
            (Self::Copy, [Value::Str(path), Value::Str(to)]) => {
                folder.copy(path, to).map(|()| Vec::new())
            }
            (Self::MakeDirectory, [Value::Str(path)]) => {
                folder.make_directory(path).map(|()| Vec::new())
            }
            (Self::ReadRange, [Value::Str(path), Value::U64(offset), Value::U64(length)]) => {
                let data = folder.read_range(path, *offset, *length)?;
                Ok(vec![Value::Bytes(Arc::new(data))])
            }
            (Self::AppendBytes, [Value::Str(path), Value::Bytes(data)]) => {
                folder.append_piece(path, data).map(|()| Vec::new())
            }
            (Self::CopyDirectory, [Value::Str(path), Value::Str(to)]) => folder
                .files
                .copy_tree(path, to, MAX_ENTRIES, MAX_COPY_TREE_BYTES, &cancelled)
                .map(|()| Vec::new())
                .map_err(refused),
            (Self::MoveDirectory, [Value::Str(path), Value::Str(to)]) => folder
                .files
                .move_tree(path, to, MAX_ENTRIES, &cancelled)
                .map(|()| Vec::new())
                .map_err(refused),
            (Self::DeleteDirectory, [Value::Str(path)]) => folder
                .files
                .remove_tree(path, MAX_ENTRIES, &cancelled)
                .map(|()| Vec::new())
                .map_err(refused),
            (
                Self::List,
                [
                    Value::Str(path),
                    Value::Str(pattern),
                    Value::Bool(recursive),
                ],
            ) => {
                let names = folder.list_filtered(path, pattern, *recursive, &cancelled)?;
                Ok(vec![Value::List(Arc::new(orange_sdk::ListRepr::Str(
                    names.into_boxed_slice(),
                )))])
            }
            (Self::FileInfo, [Value::Str(path)]) => {
                let metadata = folder.files.metadata(path).map_err(refused)?;
                let directory = metadata.is_dir();
                let modified = metadata
                    .modified()
                    .ok()
                    .and_then(unix_millis)
                    .map_or(Value::None, Value::I64);
                Ok(vec![
                    Value::Bool(directory),
                    Value::U64(if directory { 0 } else { metadata.len() }),
                    modified,
                    Value::Bool(metadata.permissions().readonly()),
                ])
            }
            _ => Err(BAD_INPUT),
        }
    }
}

/// A data port of a node in the table below: whether it is an output, its name and type.
type Port = (bool, &'static str, &'static str);
fn input(name: &'static str, ty: &'static str) -> Port {
    (false, name, ty)
}
fn output(name: &'static str, ty: &'static str) -> Port {
    (true, name, ty)
}
/// The part every definition of this package shares.
fn described(definition: Definition, description: &str) -> Definition {
    definition
        .description(description)
        .category("files", "Files")
        .uses(FOLDER_ADAPTER)
}

const STRING: &str = "core/string@1";
const BYTES: &str = "core/bytes@1";

/// The package to pass to `Host::with_services`.
pub fn package() -> ServicePackage {
    let path = || input("path", STRING);
    let nodes: [(&str, &str, &str, Operation, Vec<Port>); 16] = [
        (
            "copy-directory",
            "Copy folder",
            "Copies a folder and its contents to a new path.",
            Operation::CopyDirectory,
            vec![path(), input("to", STRING)],
        ),
        (
            "move-directory",
            "Move folder",
            "Moves a folder and its contents to a new path.",
            Operation::MoveDirectory,
            vec![path(), input("to", STRING)],
        ),
        (
            "delete-directory",
            "Delete folder",
            "Deletes a folder and all its contents.",
            Operation::DeleteDirectory,
            vec![path()],
        ),
        (
            "file-info",
            "File info",
            "Gives a file or folder's size, modification time and read-only status.",
            Operation::FileInfo,
            vec![
                path(),
                output("is_directory", "core/bool@1"),
                output("bytes", "core/u64@1"),
                output("modified_unix_ms", "core/option@1<core/i64@1>"),
                output("read_only", "core/bool@1"),
            ],
        ),
        (
            "read-range",
            "Read part of file",
            "Reads up to a number of bytes of a file from a position: fewer at its end.",
            Operation::ReadRange,
            vec![
                path(),
                input("offset", "core/u64@1"),
                input("length", "core/u64@1"),
                output("data", BYTES),
            ],
        ),
        (
            "append-bytes",
            "Append to file",
            "Adds bytes to the end of a file in a folder.",
            Operation::AppendBytes,
            vec![path(), input("data", BYTES)],
        ),
        (
            "move",
            "Move file",
            "Moves or renames a file inside a folder.",
            Operation::Move,
            vec![path(), input("to", STRING)],
        ),
        (
            "copy",
            "Copy file",
            "Copies a file inside a folder.",
            Operation::Copy,
            vec![path(), input("to", STRING)],
        ),
        (
            "make-directory",
            "Make directory",
            "Creates a directory inside a folder.",
            Operation::MakeDirectory,
            vec![path()],
        ),
        (
            "read-text",
            "Read text file",
            "Reads a UTF-8 text file from a folder.",
            Operation::ReadText,
            vec![path(), output("text", STRING)],
        ),
        (
            "write-text",
            "Write text file",
            "Creates or replaces a text file in a folder.",
            Operation::WriteText,
            vec![path(), input("text", STRING)],
        ),
        (
            "append-text",
            "Append to text file",
            "Adds text to the end of a file in a folder.",
            Operation::AppendText,
            vec![path(), input("text", STRING)],
        ),
        (
            "read-bytes",
            "Read file",
            "Reads a file from a folder as bytes.",
            Operation::ReadBytes,
            vec![path(), output("data", BYTES)],
        ),
        (
            "write-bytes",
            "Write file",
            "Creates or replaces a file in a folder from bytes.",
            Operation::WriteBytes,
            vec![path(), input("data", BYTES)],
        ),
        (
            "exists",
            "File exists",
            "Says whether a file or directory exists in a folder.",
            Operation::Exists,
            vec![path(), output("found", "core/bool@1")],
        ),
        (
            "delete",
            "Delete file",
            "Deletes a file from a folder.",
            Operation::Delete,
            vec![path()],
        ),
    ];
    let mut package = Ok(ServicePackage::new(PACKAGE, env!("CARGO_PKG_VERSION"))
        .about(
            "Files",
            "Reads, writes, lists and watches files in folders the document names.",
        )
        .with_catalogs(CATALOGS)
        .adapter(FOLDER_ADAPTER, FolderAdapter)
        .record(
            "change",
            &[
                ("kind", STRING),
                ("path", STRING),
                ("previous_path", STRING),
                ("is_directory", "core/bool@1"),
            ],
        ));
    for (name, label, description, operation, ports) in nodes {
        package = package.and_then(|package| {
            let mut definition = described(Definition::node(PACKAGE, name, label), description);
            for (is_output, port, ty) in ports {
                definition = if is_output {
                    definition.output(port, ty)
                } else {
                    let definition = definition.input(port, ty);
                    match (operation, port) {
                        (
                            Operation::MakeDirectory
                            | Operation::CopyDirectory
                            | Operation::MoveDirectory,
                            "to",
                        ) => definition,
                        (
                            Operation::CopyDirectory
                            | Operation::MoveDirectory
                            | Operation::DeleteDirectory
                            | Operation::List,
                            "path",
                        ) => definition.path_input(port, true, &[]),
                        (
                            Operation::WriteText
                            | Operation::AppendText
                            | Operation::WriteBytes
                            | Operation::AppendBytes,
                            "path",
                        )
                        | (Operation::Move | Operation::Copy, "to") => {
                            definition.save_path_input(port, &[])
                        }
                        (Operation::MakeDirectory, "path") => definition,
                        (_, "path") => definition.path_input(port, false, &[]),
                        _ => definition,
                    }
                };
            }
            package.node(&definition.build(), operation)
        });
    }
    let on_change = described(
        Definition::entry(PACKAGE, "on-change", "On file change"),
        "Fires when a file or folder is created, changed, deleted or renamed, with what changed: its kind, its path, its previous path on a rename, and whether it is a folder. Folder narrows it to a subfolder. Enable Wait for folder if that subfolder will be created after the graph starts.",
    )
    .input_default("folder", STRING, json!({"type": STRING, "value": ""}))
    .input_default(
        "wait_for_folder",
        "core/bool@1",
        json!({"type": "core/bool@1", "value": false}),
    )
    .payload(CHANGE_TYPE)
    .build();
    let list = described(
        Definition::node(PACKAGE, "list", "List folder"),
        "Lists the files and folders in a folder, folders ending in /. A pattern such as *.txt keeps the matching ones; Recursive includes what the subfolders hold.",
    )
    .input_default("path", STRING, json!({"type": STRING, "value": ""}))
    .path_input("path", true, &[])
    .input_default("pattern", STRING, json!({"type": STRING, "value": ""}))
    .input_default(
        "recursive",
        "core/bool@1",
        json!({"type": "core/bool@1", "value": false}),
    )
    .output("names", "core/list@1<core/string@1>")
    .build();
    match package
        .and_then(|package| package.source(&on_change, watch::Watch))
        .and_then(|package| package.node(&list, Operation::List))
    {
        Ok(package) => package,
        // The definitions are built from constants of this crate, checked by its tests.
        Err(error) => unreachable!("built-in definition is valid: {}", error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(root: &Path, extra: Json) -> Result<Arc<dyn Session>, String> {
        let mut settings = extra.as_object().cloned().unwrap_or_default();
        let _ = settings.insert("path".into(), Json::String(root.to_string_lossy().into()));
        FolderAdapter.open(&settings)
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        // Nested, so the directory above the folder is this test's own too.
        let root = std::env::temp_dir()
            .join(format!("orange-files-{name}-{}", std::process::id()))
            .join("folder");
        let _ = std::fs::remove_dir_all(root.parent().unwrap());
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn the_package_builds_with_its_nodes() {
        let package = package();
        assert_eq!(package.id(), PACKAGE);
        assert_eq!(package.nodes().len(), 17);
        assert_eq!(package.sources().len(), 1);
        assert_eq!(package.adapters().len(), 1);
    }

    #[test]
    fn files_are_written_replaced_appended_listed_and_deleted_inside_the_folder() {
        let root = scratch("ops");
        let session = open(&root, json!({})).unwrap();
        let folder = folder(Some(session.as_ref())).unwrap();

        folder.write("notes/a.txt", b"one").unwrap();
        folder.write("notes/a.txt", b"two").unwrap();
        folder.append("notes/a.txt", b"+three").unwrap();
        folder.append("log.txt", b"line\n").unwrap();
        assert_eq!(folder.read("notes/a.txt").unwrap(), b"two+three");
        // Replacing leaves no temporary file behind.
        let names = |path: &str| -> Vec<String> {
            folder
                .list_filtered(path, "", false, &|| false)
                .unwrap()
                .iter()
                .map(|name| name.as_str().to_owned())
                .collect()
        };
        assert_eq!(names(""), ["log.txt", "notes/"]);
        assert_eq!(names("notes"), ["a.txt"]);
        assert_eq!(folder.exists("notes/a.txt"), Ok(true));
        assert_eq!(folder.exists("notes/b.txt"), Ok(false));
        folder.delete("notes/a.txt").unwrap();
        assert_eq!(folder.delete("notes/a.txt"), Err(NOT_FOUND));
        assert_eq!(folder.read("notes/a.txt"), Err(NOT_FOUND));

        // Moving, copying and directories stay inside the folder too.
        folder.write("a.txt", b"moved").unwrap();
        folder.rename("a.txt", "sub/b.txt").unwrap();
        folder.copy("sub/b.txt", "c.txt").unwrap();
        assert_eq!(folder.exists("a.txt"), Ok(false));
        assert_eq!(folder.read("sub/b.txt").unwrap(), b"moved");
        assert_eq!(folder.read("c.txt").unwrap(), b"moved");
        assert_eq!(folder.rename("missing.txt", "d.txt"), Err(NOT_FOUND));
        assert_eq!(folder.copy("c.txt", "../d.txt"), Err(BAD_PATH));
        assert_eq!(folder.rename("../c.txt", "d.txt"), Err(BAD_PATH));
        folder.make_directory("deep/er").unwrap();
        assert_eq!(folder.exists("deep/er"), Ok(true));
        assert_eq!(folder.make_directory("../out"), Err(BAD_PATH));

        for escape in ["../x", "/x", "notes/../../x", "C:/x", "a\\b", "nul", ""] {
            assert_eq!(folder.write(escape, b"x"), Err(BAD_PATH), "{escape:?}");
            assert_eq!(folder.read(escape), Err(BAD_PATH), "{escape:?}");
        }
        assert!(!root.parent().unwrap().join("x").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_file_is_walked_in_pieces_and_grown_piece_by_piece() {
        let root = scratch("pieces");
        let session = open(&root, json!({})).unwrap();
        let folder = folder(Some(session.as_ref())).unwrap();
        folder.write("big.bin", b"0123456789").unwrap();
        assert_eq!(folder.read_range("big.bin", 0, 4).unwrap(), b"0123");
        assert_eq!(folder.read_range("big.bin", 8, 4).unwrap(), b"89");
        // Past the end there is nothing to read, which is not a failure.
        assert_eq!(folder.read_range("big.bin", 10, 4).unwrap(), b"");
        assert_eq!(folder.read_range("big.bin", 99, 4).unwrap(), b"");
        assert_eq!(
            folder.read_range("big.bin", 0, MAX_FILE_BYTES + 1),
            Err(TOO_LARGE)
        );
        assert_eq!(folder.read_range("../big.bin", 0, 1), Err(BAD_PATH));
        // A copy made piece by piece equals the original.
        let mut offset = 0;
        loop {
            let piece = folder.read_range("big.bin", offset, 3).unwrap();
            if piece.is_empty() {
                break;
            }
            folder.append_piece("copy.bin", &piece).unwrap();
            offset += piece.len() as u64;
        }
        assert_eq!(folder.read("copy.bin").unwrap(), b"0123456789");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_read_only_folder_refuses_changes_and_settings_are_checked() {
        let root = scratch("ro");
        std::fs::write(root.join("kept.txt"), "kept").unwrap();
        let session = open(&root, json!({"read_only": true})).unwrap();
        let folder = folder(Some(session.as_ref())).unwrap();
        assert_eq!(folder.read("kept.txt").unwrap(), b"kept");
        assert_eq!(folder.write("kept.txt", b"x"), Err(READ_ONLY));
        assert_eq!(folder.append("kept.txt", b"x"), Err(READ_ONLY));
        assert_eq!(folder.delete("kept.txt"), Err(READ_ONLY));

        assert!(open(&root.join("missing"), json!({})).is_err());
        assert!(open(&root.join("kept.txt"), json!({})).is_err());
        assert!(open(&root, json!({"read_only": "yes"})).is_err());
        assert!(open(&root, json!({"poll_ms": 1})).is_err());
        assert!(FolderAdapter.open(&Map::new()).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn recursive_actions_are_bounded_and_filter_the_requested_tree() {
        let root = scratch("tree");
        let session = open(&root, json!({})).unwrap();
        let folder = folder(Some(session.as_ref())).unwrap();
        folder.write("from/a.txt", b"ab").unwrap();
        folder.write("from/sub/b.txt", b"cd").unwrap();
        folder.write("from/sub/c.md", b"ef").unwrap();
        let names = folder
            .list_filtered("from", "*.txt", true, &|| false)
            .unwrap();
        assert_eq!(
            names.iter().map(|name| name.as_str()).collect::<Vec<_>>(),
            ["a.txt", "sub/b.txt"]
        );
        let names = folder.list_filtered("from", "", false, &|| false).unwrap();
        assert_eq!(
            names.iter().map(|name| name.as_str()).collect::<Vec<_>>(),
            ["a.txt", "sub/"]
        );
        assert_eq!(
            folder.files.copy_tree("from", "copy", 2, 10, &|| false),
            Err(FsError::TooLarge)
        );
        assert_eq!(
            folder.files.copy_tree("from", "copy", 4, 5, &|| false),
            Err(FsError::TooLarge)
        );
        assert_eq!(
            folder
                .files
                .copy_tree("from", "from/sub/copy", 4, 6, &|| false),
            Err(FsError::InvalidPath)
        );
        assert_eq!(
            folder.files.copy_tree("from", "copy", 4, 6, &|| true),
            Err(FsError::Cancelled)
        );
        assert!(!root.join("copy").exists());
        folder
            .files
            .copy_tree("from", "copy", 4, 6, &|| false)
            .unwrap();
        assert_eq!(folder.read("copy/sub/b.txt").unwrap(), b"cd");
        assert_eq!(folder.files.metadata("copy/sub/b.txt").unwrap().len(), 2);
        assert!(folder.files.metadata("copy/sub").unwrap().is_dir());
        assert_eq!(
            folder.files.move_tree("copy", "from", 4, &|| false),
            Err(FsError::InvalidPath)
        );
        folder
            .files
            .move_tree("copy", "moved", 4, &|| false)
            .unwrap();
        assert!(!root.join("copy").exists());
        assert_eq!(
            folder.files.remove_tree("moved", 3, &|| false),
            Err(FsError::TooLarge)
        );
        assert_eq!(folder.read("moved/a.txt").unwrap(), b"ab");
        folder.files.remove_tree("moved", 4, &|| false).unwrap();
        assert!(!root.join("moved").exists());
        assert_eq!(
            folder.files.remove_tree("", 10, &|| false),
            Err(FsError::InvalidPath)
        );
        let locked = orange_sandbox::Folder::new(root.canonicalize().unwrap(), true);
        assert_eq!(
            locked.copy_tree("from", "copy", 4, 6, &|| false),
            Err(FsError::ReadOnly)
        );
        assert_eq!(
            locked.move_tree("from", "copy", 4, &|| false),
            Err(FsError::ReadOnly)
        );
        assert_eq!(
            locked.remove_tree("from", 4, &|| false),
            Err(FsError::ReadOnly)
        );
        std::fs::remove_dir_all(root.parent().unwrap()).unwrap();
    }

    #[cfg(any(unix, windows))]
    #[cfg_attr(
        windows,
        ignore = "requires a Windows directory that permits creating junctions"
    )]
    #[test]
    fn recursive_actions_and_enumeration_do_not_follow_a_real_escape_link() {
        use std::fs;
        let root = scratch("link");
        let outside = root.parent().unwrap().join("outside");
        fs::create_dir_all(root.join("from")).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("kept.txt"), "kept").unwrap();
        fs::write(root.join("from/kept.txt"), "kept").unwrap();
        let link = root.join("from").join("link");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        #[cfg(windows)]
        {
            // A directory junction exercises Windows reparse checks without requiring
            // the privilege needed to create a symbolic link.
            let output = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(&link)
                .arg(&outside)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "failed to create test junction: {output:?}"
            );
        }
        let session = open(&root, json!({})).unwrap();
        let folder = folder(Some(session.as_ref())).unwrap();
        let names = folder.list_filtered("from", "", true, &|| false).unwrap();
        assert_eq!(
            names.iter().map(|name| name.as_str()).collect::<Vec<_>>(),
            ["kept.txt"]
        );
        assert_eq!(
            folder.list_filtered("from/link", "", true, &|| false),
            Err(BAD_PATH)
        );
        assert_eq!(
            folder.files.metadata("from/link/kept.txt").err(),
            Some(FsError::InvalidPath)
        );
        assert_eq!(
            folder.files.copy_tree("from", "copy", 10, 100, &|| false),
            Err(FsError::InvalidPath)
        );
        assert_eq!(
            folder.files.move_tree("from", "moved", 10, &|| false),
            Err(FsError::InvalidPath)
        );
        assert_eq!(
            folder.files.remove_tree("from", 10, &|| false),
            Err(FsError::InvalidPath)
        );
        assert!(!root.join("copy").exists() && !root.join("moved").exists());
        assert_eq!(fs::read(outside.join("kept.txt")).unwrap(), b"kept");
        assert_eq!(fs::read(root.join("from/kept.txt")).unwrap(), b"kept");
        #[cfg(unix)]
        fs::remove_file(link).unwrap();
        #[cfg(windows)]
        fs::remove_dir(link).unwrap();
        fs::remove_dir_all(root.parent().unwrap()).unwrap();
    }
}
