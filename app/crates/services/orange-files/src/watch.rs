//! Polling file and directory changes, with one clean baseline per arming.

use super::{BAD_INPUT, FOLDER_ADAPTER, Folder, MAX_ENTRIES, folder, refused};
use orange_sandbox::FsError;
use orange_sdk::{Arguments, EmitError, Emitter, Lifecycle, ServiceSource, Session, Value};
use serde_json::{Map, Value as Json};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::thread;
use std::time::SystemTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Identity(u64, u64, u64);

#[derive(Debug, PartialEq, Eq)]
struct Stamp {
    identity: Option<Identity>,
    directory: bool,
    bytes: u64,
    modified: Option<SystemTime>,
    readonly: bool,
}

impl From<orange_sandbox::Info> for Stamp {
    fn from(info: orange_sandbox::Info) -> Self {
        Self {
            identity: info.identity().map(|(a, b, c)| Identity(a, b, c)),
            directory: info.is_dir(),
            bytes: info.bytes(),
            modified: info.modified(),
            readonly: info.read_only(),
        }
    }
}

type Snapshot = BTreeMap<String, Stamp>;

fn snapshot(
    folder: &Folder,
    path: &str,
    life: &Lifecycle,
    limit: usize,
) -> Result<Snapshot, FsError> {
    folder
        .files
        .tree(path, true, limit, &|| life.stopped())
        .map(|entries| {
            entries
                .into_iter()
                .map(|entry| (entry.path, Stamp::from(entry.info)))
                .collect()
        })
}

#[derive(Debug, PartialEq, Eq)]
enum Change<'a> {
    Created,
    Modified,
    Deleted,
    Renamed(&'a str),
}

impl Change<'_> {
    fn name(&self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Modified => "changed",
            Self::Deleted => "deleted",
            Self::Renamed(_) => "renamed",
        }
    }
}

/// A duplicated fingerprint is ambiguous, for example two names of a hard link.
fn candidates<'a>(
    snapshot: &'a Snapshot,
    other: &Snapshot,
) -> BTreeMap<(Identity, bool), Option<&'a str>> {
    let mut unique = BTreeMap::new();
    for (path, stamp) in snapshot {
        if let Some(identity) = stamp.identity
            && !other.contains_key(path)
        {
            let _ = unique
                .entry((identity, stamp.directory))
                .and_modify(|previous| *previous = None)
                .or_insert(Some(path.as_str()));
        }
    }
    unique
}

fn differences<'a>(before: &'a Snapshot, after: &'a Snapshot) -> Vec<(&'a str, bool, Change<'a>)> {
    let missing = candidates(before, after);
    let added = candidates(after, before);
    let mut paired = std::collections::BTreeSet::new();
    let mut changes = Vec::new();
    for (path, stamp) in after {
        let change = match before.get(path) {
            Some(previous) if previous == stamp => continue,
            Some(_) => Change::Modified,
            None => {
                let old = stamp.identity.and_then(|identity| {
                    let key = (identity, stamp.directory);
                    (added.get(&key) == Some(&Some(path.as_str())))
                        .then(|| missing.get(&key).copied().flatten())
                        .flatten()
                });
                if let Some(old) = old {
                    let _ = paired.insert(old);
                    Change::Renamed(old)
                } else {
                    Change::Created
                }
            }
        };
        changes.push((path.as_str(), stamp.directory, change));
    }
    for (path, stamp) in before {
        if !after.contains_key(path) && !paired.contains(path.as_str()) {
            changes.push((path.as_str(), stamp.directory, Change::Deleted));
        }
    }
    changes
}

/// Signals its watcher to stop when the arm is dropped.
struct Watching(Arc<Lifecycle>);

impl Drop for Watching {
    fn drop(&mut self) {
        self.0.stop();
    }
}

/// Watches a folder and fires once for each change, with a [`crate::CHANGE_TYPE`]
/// record.
#[derive(Debug, Clone, Copy)]
pub(super) struct Watch;

impl Watch {
    fn start_thread(
        self,
        session: Option<Arc<dyn Session>>,
        path: String,
        wait_for_folder: bool,
        emitter: Emitter,
    ) -> Result<Box<dyn Send>, String> {
        let session = session.ok_or("the entry needs a folder connection")?;
        let Some(root) = folder(Some(session.as_ref())) else {
            return Err(format!("the entry needs a {FOLDER_ADAPTER} connection"));
        };
        let _ = orange_sandbox::inside(root.files.root(), &path, true)
            .map_err(|error| refused(error).message)?;
        let stop = Arc::new(Lifecycle::new());
        let life = Arc::clone(&stop);
        let _ = thread::Builder::new()
            .name("orange-files-watch".into())
            .spawn(move || {
                if let Some(folder) = folder(Some(session.as_ref())) {
                    self.watch(folder, &path, wait_for_folder, &emitter, &life, MAX_ENTRIES);
                }
            })
            .map_err(|_| "the file watcher could not start")?;
        Ok(Box::new(Watching(stop)))
    }

    fn watch(
        self,
        folder: &Folder,
        path: &str,
        wait_for_folder: bool,
        emitter: &Emitter,
        life: &Lifecycle,
        limit: usize,
    ) {
        let mut before = None;
        let mut last_error = None;
        loop {
            match snapshot(folder, path, life, limit) {
                Ok(now) => {
                    last_error = None;
                    if let Some(before) = &before {
                        for (relative, directory, change) in differences(before, &now) {
                            if life.stopped()
                                || self.report(emitter, path, relative, directory, &change)
                                    == Err(EmitError::Closed)
                            {
                                return;
                            }
                        }
                    }
                    before = Some(now);
                }
                Err(FsError::Cancelled) => return,
                Err(FsError::Changed) => {}
                Err(FsError::NotFound)
                    if wait_for_folder
                        && before.is_none()
                        && last_error.is_none()
                        && !path.is_empty() => {}
                Err(error) => {
                    before = None;
                    if last_error != Some(error) {
                        match emitter.fault(refused(error)) {
                            Ok(()) => last_error = Some(error),
                            Err(EmitError::Closed) => return,
                            Err(EmitError::Busy) => {}
                        }
                    }
                }
            }
            if !life.rest(folder.poll) {
                return;
            }
        }
    }

    fn report(
        self,
        emitter: &Emitter,
        folder: &str,
        path: &str,
        directory: bool,
        change: &Change<'_>,
    ) -> Result<(), EmitError> {
        let join = |relative: &str| {
            if folder.is_empty() {
                relative.to_owned()
            } else {
                format!("{folder}/{relative}")
            }
        };
        let previous = match change {
            Change::Renamed(path) => join(path),
            _ => String::new(),
        };
        let Ok(ty) = crate::CHANGE_TYPE.parse() else {
            return Ok(());
        };
        let text = |text: String| Some(Value::Str(Arc::new(text)));
        emitter.emit(Some(Value::Record(Arc::new(orange_sdk::RecordRepr {
            ty,
            fields: Box::new([
                text(change.name().to_owned()),
                text(join(path)),
                text(previous),
                Some(Value::Bool(directory)),
            ]),
        }))))
    }
}

impl ServiceSource for Watch {
    fn start(
        &self,
        session: Option<Arc<dyn Session>>,
        emitter: Emitter,
    ) -> Result<Box<dyn Send>, String> {
        self.start_thread(session, String::new(), false, emitter)
    }

    fn start_with_arguments(
        &self,
        session: Option<Arc<dyn Session>>,
        _settings: &Map<String, Json>,
        arguments: &Arguments,
        emitter: Emitter,
    ) -> Result<Box<dyn Send>, String> {
        let Some(Value::Str(path)) = arguments.get("folder") else {
            return Err(BAD_INPUT.message.into());
        };
        let wait_for_folder = matches!(arguments.get("wait_for_folder"), Some(Value::Bool(true)));
        self.start_thread(session, path.to_string(), wait_for_folder, emitter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TOO_LARGE;
    use std::fs;
    use std::sync::mpsc::{self, RecvTimeoutError};
    use std::time::Duration;

    #[test]
    fn an_overflow_faults_and_recovery_rebaselines_without_firing() {
        let root = std::env::temp_dir().join(format!("orange-watch-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("chosen/old")).unwrap();
        fs::write(root.join("chosen/second"), "x").unwrap();
        fs::write(root.join("chosen/third"), "x").unwrap();
        let folder = Folder {
            files: orange_sandbox::Folder::existing(root.canonicalize().unwrap(), false),
            poll: Duration::from_millis(10),
        };
        let (events, observed) = mpsc::channel();
        let faults = events.clone();
        let emitter = Emitter::with_faults(
            move |firing| events.send(Ok(firing)).map_err(|_| EmitError::Closed),
            move |fault| faults.send(Err(fault)).map_err(|_| EmitError::Closed),
        );
        let life = Arc::new(Lifecycle::new());
        let guard = Watching(life.clone());
        let worker =
            thread::spawn(move || Watch.watch(&folder, "chosen", false, &emitter, &life, 2));
        assert_eq!(
            observed.recv_timeout(Duration::from_secs(5)),
            Ok(Err(TOO_LARGE))
        );
        assert_eq!(
            observed.recv_timeout(Duration::from_millis(50)),
            Err(RecvTimeoutError::Timeout)
        );
        fs::remove_file(root.join("chosen/third")).unwrap();
        assert_eq!(
            observed.recv_timeout(Duration::from_millis(150)),
            Err(RecvTimeoutError::Timeout)
        );
        fs::remove_file(root.join("chosen/second")).unwrap();
        let firing = observed.recv_timeout(Duration::from_secs(5)).unwrap();
        let Ok(Some(Value::Record(change))) = firing else {
            panic!("expected one deletion: {firing:?}");
        };
        let text = |text: &str| Some(Value::Str(Arc::new(text.to_owned())));
        assert_eq!(
            &*change.fields,
            &[
                text("deleted"),
                text("chosen/second"),
                text(""),
                Some(Value::Bool(false))
            ]
        );
        drop(guard);
        worker.join().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn change_kinds_use_the_names_the_record_documents() {
        let names: Vec<_> = [
            Change::Created,
            Change::Modified,
            Change::Deleted,
            Change::Renamed("a"),
        ]
        .iter()
        .map(Change::name)
        .collect();
        assert_eq!(names, ["created", "changed", "deleted", "renamed"]);
    }

    #[test]
    fn a_real_directory_rename_reports_the_old_and_new_paths() {
        let root = std::env::temp_dir().join(format!("orange-watch-rename-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("old")).unwrap();
        let folder = Folder {
            files: orange_sandbox::Folder::existing(root.canonicalize().unwrap(), false),
            poll: Duration::from_millis(10),
        };
        let life = Lifecycle::new();
        let before = snapshot(&folder, "", &life, 10).unwrap();
        fs::rename(root.join("old"), root.join("new")).unwrap();
        let after = snapshot(&folder, "", &life, 10).unwrap();
        assert_eq!(
            differences(&before, &after),
            [("new", true, Change::Renamed("old"))]
        );
        fs::remove_dir_all(root).unwrap();
    }
}
