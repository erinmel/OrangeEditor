//! The base file package: reading, writing, listing and watching files in a folder.
//!
//! It is an ordinary [`ServicePackage`], built on the plugin SDK alone. A document
//! declares an `org.orange.files/folder` connection naming a directory, and the nodes
//! below work on paths relative to it. A node can name nothing outside its folder: a
//! path is `/`-separated plain names, and `..`, absolute and drive forms, device names
//! and links that lead out of the folder are refused. An application that does not
//! want file access leaves the package out.
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
    Call, ConnectionAdapter, EmitError, Emitter, FaultText, KernelFault, ServiceNode,
    ServicePackage, ServiceSource, Session, Value,
};
use serde_json::{Map, Value as Json};
use std::any::Any;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, SystemTime};

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
/// Lists a directory. Input `path` (empty for the folder itself); output `names`,
/// sorted, a directory's name ending with `/`.
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
/// Removes an empty directory. Input `path`.
pub const REMOVE_DIRECTORY: &str = "org.orange.files/remove-directory";
/// Entry fired when a file in the folder is created, changed or deleted, with its
/// path as `payload`. Changes are found by comparing sizes and modification times
/// every `poll_ms`, so several changes to one file within that time fire once.
pub const ON_CHANGE: &str = "org.orange.files/on-change";

/// Largest file a node reads or writes.
pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
pub use orange_sandbox::MAX_PATH_BYTES;
/// Most entries one listing returns and one watched folder may hold.
pub const MAX_ENTRIES: usize = 10_000;
const DEFAULT_POLL_MS: u64 = 500;

const fn fault(code: &'static str, key: &'static str, message: &'static str) -> KernelFault {
    KernelFault {
        code: FaultText::Static(code),
        message_key: FaultText::Static(key),
        message,
    }
}
const NO_SESSION: KernelFault = fault(
    "org.orange.files/Connection",
    "org.orange.files/connection-kind",
    "The node needs a folder connection.",
);
const BAD_INPUT: KernelFault = fault(
    "org.orange.files/Input",
    "org.orange.files/input-invalid",
    "The node's inputs are not of the expected types.",
);
const BAD_PATH: KernelFault = fault(
    "org.orange.files/Path",
    "org.orange.files/path-invalid",
    "The path is not a relative path inside the folder.",
);
const NOT_FOUND: KernelFault = fault(
    "org.orange.files/NotFound",
    "org.orange.files/not-found",
    "The file or directory does not exist.",
);
const TOO_LARGE: KernelFault = fault(
    "org.orange.files/TooLarge",
    "org.orange.files/too-large",
    "The file or listing is over the size limit.",
);
const NOT_TEXT: KernelFault = fault(
    "org.orange.files/Encoding",
    "org.orange.files/not-utf8",
    "The file is not UTF-8 text.",
);
const READ_ONLY: KernelFault = fault(
    "org.orange.files/ReadOnly",
    "org.orange.files/read-only",
    "The folder connection is read-only.",
);
const FAILED: KernelFault = fault(
    "org.orange.files/Io",
    "org.orange.files/io-failed",
    "The operating system refused the file operation.",
);

fn refused(error: FsError) -> KernelFault {
    match error {
        FsError::NotFound => NOT_FOUND,
        FsError::InvalidPath => BAD_PATH,
        FsError::TooLarge => TOO_LARGE,
        FsError::ReadOnly => READ_ONLY,
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
    fn list(&self, path: &str) -> Result<Vec<Arc<String>>, KernelFault> {
        let names = self.files.list(path, MAX_ENTRIES).map_err(refused)?;
        Ok(names.into_iter().map(Arc::new).collect())
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
        self.files.copy(path, to, MAX_FILE_BYTES).map_err(refused)
    }
    fn make_directory(&self, path: &str) -> Result<(), KernelFault> {
        self.files.make_directory(path).map_err(refused)
    }
    fn remove_directory(&self, path: &str) -> Result<(), KernelFault> {
        self.files.remove_directory(path).map_err(refused)
    }

    /// Every file under the folder with its size and modification time, by relative
    /// path. Links are not followed. `None` when the folder holds too many entries.
    fn snapshot(&self) -> Option<BTreeMap<String, (u64, Option<SystemTime>)>> {
        let mut files = BTreeMap::new();
        let mut pending = vec![(self.files.root().to_path_buf(), String::new())];
        let mut seen = 0usize;
        while let Some((directory, prefix)) = pending.pop() {
            let Ok(entries) = std::fs::read_dir(&directory) else {
                continue;
            };
            for entry in entries.flatten() {
                seen += 1;
                if seen > MAX_ENTRIES {
                    return None;
                }
                let Ok(name) = entry.file_name().into_string() else {
                    continue;
                };
                // The temporary files of this package's own writes are not changes.
                if orange_sandbox::Folder::is_temporary(&name) {
                    continue;
                }
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                let path = format!("{prefix}{name}");
                if kind.is_dir() {
                    pending.push((entry.path(), format!("{path}/")));
                } else if kind.is_file()
                    && let Ok(metadata) = entry.metadata()
                {
                    let _ = files.insert(path, (metadata.len(), metadata.modified().ok()));
                }
            }
        }
        Some(files)
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
    RemoveDirectory,
}
impl ServiceNode for Operation {
    fn run(&self, call: &Call<'_>) -> Result<Vec<Value>, KernelFault> {
        let folder = folder(call.session).ok_or(NO_SESSION)?;
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
            (Self::List, [Value::Str(path)]) => {
                let names = folder.list(path)?.into_boxed_slice();
                Ok(vec![Value::List(Arc::new(orange_sdk::ListRepr::Str(
                    names,
                )))])
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
            (Self::RemoveDirectory, [Value::Str(path)]) => {
                folder.remove_directory(path).map(|()| Vec::new())
            }
            _ => Err(BAD_INPUT),
        }
    }
}

/// Stops its watcher when the flow that owns the entry closes.
struct Watching(Arc<AtomicBool>);
impl Drop for Watching {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

#[derive(Debug)]
struct OnChange;
impl ServiceSource for OnChange {
    fn start(
        &self,
        session: Option<Arc<dyn Session>>,
        emitter: Emitter,
    ) -> Result<Box<dyn Send>, String> {
        let session = session.ok_or("the entry needs a folder connection")?;
        if folder(Some(session.as_ref())).is_none() {
            return Err("the entry needs a folder connection".into());
        }
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let _ = thread::Builder::new()
            .name("orange-files-watch".into())
            .spawn(move || {
                let Some(folder) = folder(Some(session.as_ref())) else {
                    return;
                };
                let mut before = folder.snapshot().unwrap_or_default();
                let step = Duration::from_millis(25).min(folder.poll);
                'watch: loop {
                    let mut waited = Duration::ZERO;
                    while waited < folder.poll {
                        if stopped.load(Ordering::Acquire) {
                            break 'watch;
                        }
                        thread::sleep(step);
                        waited += step;
                    }
                    // A folder over the limit is not watched until it is small again.
                    let Some(now) = folder.snapshot() else {
                        continue;
                    };
                    let changed = now
                        .iter()
                        .filter(|(path, state)| before.get(*path) != Some(state))
                        .map(|(path, _)| path)
                        .chain(before.keys().filter(|path| !now.contains_key(*path)));
                    for path in changed {
                        let payload = Value::Str(Arc::new(path.clone()));
                        if emitter.emit(Some(payload)) == Err(EmitError::Closed) {
                            break 'watch;
                        }
                    }
                    before = now;
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Box::new(Watching(stop)))
    }
}

/// A node definition of this package. `ports` are its port objects.
fn definition(name: &str, label: &str, description: &str, entry: bool, ports: &[String]) -> String {
    let kind = if entry {
        r#""kind": "entry""#
    } else {
        r#""kind": "async", "suspends": "always""#
    };
    format!(
        r#"{{
  "definition_id": "{PACKAGE}/{name}",
  "label": {{"key": "{PACKAGE}/{name}-label", "fallback": "{label}"}},
  "description": {{"key": "{PACKAGE}/{name}-description", "fallback": "{description}"}},
  "editor": {{"category_id": "{PACKAGE}/category-files",
    "category": {{"key": "{PACKAGE}/category-files", "fallback": "Files"}}}},
  "definition_version": 1,
  "config_version": 1,
  {kind},
  "determinism": "external",
  "connection": true,
  "ports": [{}]
}}"#,
        ports.join(",")
    )
}
fn input(name: &str, ty: &str) -> String {
    format!(r#"{{"port_id": "{name}", "direction": "input", "kind": "data", "type": "{ty}"}}"#)
}
fn output(name: &str, ty: &str) -> String {
    format!(
        r#"{{"port_id": "{name}", "direction": "output", "kind": "data", "type": "{ty}",
      "source": "execution", "available_on": ["then"]}}"#
    )
}

const STRING: &str = "core/string@1";
const BYTES: &str = "core/bytes@1";

/// The package to pass to `Host::with_services`.
pub fn package() -> ServicePackage {
    let path = || input("path", STRING);
    let nodes: [(&str, &str, &str, Operation, Vec<String>); 12] = [
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
            "remove-directory",
            "Remove directory",
            "Removes an empty directory from a folder.",
            Operation::RemoveDirectory,
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
            "list",
            "List folder",
            "Lists the files and directories in a folder.",
            Operation::List,
            vec![path(), output("names", "core/list@1<core/string@1>")],
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
        .adapter(FOLDER_ADAPTER, FolderAdapter));
    for (name, label, description, operation, ports) in nodes {
        package = package.and_then(|package| {
            package.node(
                &definition(name, label, description, false, &ports),
                operation,
            )
        });
    }
    let on_change = definition(
        "on-change",
        "On file change",
        "Fires when a file in a folder is created, changed or deleted.",
        true,
        &[format!(
            r#"{{"port_id": "payload", "direction": "output", "kind": "data", "type": "{STRING}",
      "source": "payload"}}"#
        )],
    );
    match package.and_then(|package| package.source(&on_change, OnChange)) {
        Ok(package) => package,
        // The definitions are built from constants of this crate, checked by its tests.
        Err(error) => unreachable!("built-in definition is valid: {error}"),
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
        assert_eq!(package.nodes().len(), 12);
        assert_eq!(package.sources().len(), 1);
        assert_eq!(package.adapters().len(), 1);
    }

    #[test]
    fn files_are_written_replaced_appended_listed_and_deleted_inside_the_folder() {
        let root = scratch("ops");
        let session = open(&root, serde_json::json!({})).unwrap();
        let folder = folder(Some(session.as_ref())).unwrap();

        folder.write("notes/a.txt", b"one").unwrap();
        folder.write("notes/a.txt", b"two").unwrap();
        folder.append("notes/a.txt", b"+three").unwrap();
        folder.append("log.txt", b"line\n").unwrap();
        assert_eq!(folder.read("notes/a.txt").unwrap(), b"two+three");
        // Replacing leaves no temporary file behind.
        let names = |path: &str| -> Vec<String> {
            folder
                .list(path)
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
        assert_eq!(folder.remove_directory("deep"), Err(FAILED));
        folder.remove_directory("deep/er").unwrap();
        folder.remove_directory("deep").unwrap();
        assert_eq!(folder.remove_directory("deep"), Err(NOT_FOUND));
        assert_eq!(folder.make_directory("../out"), Err(BAD_PATH));

        for escape in ["../x", "/x", "notes/../../x", "C:/x", "a\\b", "nul", ""] {
            assert_eq!(folder.write(escape, b"x"), Err(BAD_PATH), "{escape:?}");
            assert_eq!(folder.read(escape), Err(BAD_PATH), "{escape:?}");
        }
        assert!(!root.parent().unwrap().join("x").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_read_only_folder_refuses_changes_and_settings_are_checked() {
        let root = scratch("ro");
        std::fs::write(root.join("kept.txt"), "kept").unwrap();
        let session = open(&root, serde_json::json!({"read_only": true})).unwrap();
        let folder = folder(Some(session.as_ref())).unwrap();
        assert_eq!(folder.read("kept.txt").unwrap(), b"kept");
        assert_eq!(folder.write("kept.txt", b"x"), Err(READ_ONLY));
        assert_eq!(folder.append("kept.txt", b"x"), Err(READ_ONLY));
        assert_eq!(folder.delete("kept.txt"), Err(READ_ONLY));

        assert!(open(&root.join("missing"), serde_json::json!({})).is_err());
        assert!(open(&root.join("kept.txt"), serde_json::json!({})).is_err());
        assert!(open(&root, serde_json::json!({"read_only": "yes"})).is_err());
        assert!(open(&root, serde_json::json!({"poll_ms": 1})).is_err());
        assert!(FolderAdapter.open(&Map::new()).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}
