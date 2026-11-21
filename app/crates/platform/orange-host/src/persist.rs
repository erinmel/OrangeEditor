//! Installed packages on disk, so a host started on the same directory has them again.
//!
//! The directory holds each installed archive under the name of its SHA-256 and an
//! index, `installed.json`, listing them in installation order with the capabilities
//! the user granted. Starting a host installs the listed archives again, in order,
//! through the same checks as a first installation. Both kinds of file are written to
//! a temporary name and renamed, so a crash leaves the previous index or the new one.
//! `disabled.json` lists the IDs of packages a host started on the directory leaves out.
use orange_guest::Grants;
use orange_model::format::History;
use orange_packages::PackageKey;
use serde_json::{Map, Value as Json, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

const INDEX: &str = "installed.json";
/// The versions of `installed.json`; a change to its layout adds an upgrade step here.
const INSTALLED: History = History::new("orange.installed-packages", 1, &[]);

/// One archive the index lists.
pub(crate) struct Saved {
    pub file: String,
    pub grants: Grants,
    /// The package ID and version the index recorded for the archive.
    pub key: Option<(String, String)>,
    /// The archive bytes, or why they could not be read.
    pub archive: Result<Vec<u8>, String>,
}

fn io(what: &str, error: &std::io::Error) -> String {
    // An I/O message can name paths; only the operation and error kind are kept.
    format!("{what}: {:?}", error.kind())
}

fn entries(directory: &Path) -> Result<Vec<Json>, String> {
    let text = match fs::read_to_string(directory.join(INDEX)) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(io("read the package index", &error)),
    };
    let (mut index, _) = INSTALLED
        .read(&text)
        .map_err(|error| format!("the package index: {error}"))?;
    match index.remove("packages") {
        Some(Json::Array(packages)) => Ok(packages),
        _ => Err("the package index lists no `packages`".to_owned()),
    }
}

/// The index file listing `packages`, at the current version.
fn index(packages: Vec<Json>) -> Json {
    let mut body = Map::new();
    let _ = body.insert("packages".to_owned(), Json::Array(packages));
    INSTALLED.wrap(body)
}

/// A file name the index may refer to: one this module wrote, never a path.
fn archive_name(entry: &Json) -> Option<&str> {
    let name = entry["archive"].as_str()?;
    let stem = name.strip_suffix(".orangeplug")?;
    (stem.len() == 64 && stem.bytes().all(|byte| byte.is_ascii_hexdigit())).then_some(name)
}

/// Creates the directory if needed and reads what was installed into it.
pub(crate) fn load(directory: &Path) -> Result<Vec<Saved>, String> {
    fs::create_dir_all(directory).map_err(|error| io("create the package directory", &error))?;
    let mut saved = Vec::new();
    for entry in entries(directory)? {
        let file = archive_name(&entry)
            .ok_or("the package index names a file this host did not write")?
            .to_owned();
        let archive =
            fs::read(directory.join(&file)).map_err(|error| io("read the archive", &error));
        saved.push(Saved {
            file,
            grants: Grants {
                log: entry["log"] == true,
                files: entry["files"] == true,
                http: entry["http"] == true,
            },
            key: entry["id"]
                .as_str()
                .zip(entry["version"].as_str())
                .map(|(id, version)| (id.to_owned(), version.to_owned())),
            archive,
        });
    }
    Ok(saved)
}

const DISABLED: &str = "disabled.json";
/// The versions of `disabled.json`.
const DISABLED_LIST: History = History::new("orange.disabled-packages", 1, &[]);

/// The package IDs the directory says to leave out; none when it says nothing.
pub(crate) fn disabled(directory: &Path) -> Result<BTreeSet<String>, String> {
    let text = match fs::read_to_string(directory.join(DISABLED)) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(BTreeSet::new());
        }
        Err(error) => return Err(io("read the disabled packages", &error)),
    };
    let (list, _) = DISABLED_LIST
        .read(&text)
        .map_err(|error| format!("the disabled package list: {error}"))?;
    let ids = list
        .get("packages")
        .and_then(Json::as_array)
        .ok_or("the disabled package list has no `packages`")?;
    Ok(ids
        .iter()
        .filter_map(|id| id.as_str().map(str::to_owned))
        .collect())
}

/// Replaces the list of package IDs to leave out.
pub(crate) fn set_disabled(directory: &Path, ids: &BTreeSet<String>) -> Result<(), String> {
    fs::create_dir_all(directory).map_err(|error| io("create the package directory", &error))?;
    let mut body = Map::new();
    let _ = body.insert("packages".to_owned(), json!(ids));
    let list = DISABLED_LIST.wrap(body);
    write(directory, DISABLED, list.to_string().as_bytes())
}

fn write(directory: &Path, name: &str, bytes: &[u8]) -> Result<(), String> {
    let temporary = directory.join(format!("{name}.tmp"));
    fs::write(&temporary, bytes).map_err(|error| io("write a package file", &error))?;
    fs::rename(&temporary, directory.join(name))
        .map_err(|error| io("replace a package file", &error))
}

/// Adds an installed archive to the directory and to the end of its index.
pub(crate) fn save(
    directory: &Path,
    archive: &[u8],
    grants: Grants,
    key: &PackageKey,
) -> Result<(), String> {
    let name = format!("{}.orangeplug", crate::hex(&Sha256::digest(archive)));
    let mut packages = entries(directory)?;
    packages.retain(|entry| entry["archive"] != name.as_str());
    packages.push(json!({"archive": name, "log": grants.log,
        "files": grants.files, "http": grants.http,
        "id": key.id.as_str(), "version": key.version.as_str()}));
    write(directory, &name, archive)?;
    write(directory, INDEX, index(packages).to_string().as_bytes())
}

/// Takes a package out of the index, then deletes its archive. A crash between the two
/// leaves an unlisted file, which is never read.
pub(crate) fn forget(directory: &Path, key: &PackageKey) -> Result<(), String> {
    let (gone, kept): (Vec<Json>, Vec<Json>) = entries(directory)?.into_iter().partition(|entry| {
        entry["id"] == key.id.as_str() && entry["version"] == key.version.as_str()
    });
    if gone.is_empty() {
        return Ok(());
    }
    write(directory, INDEX, index(kept).to_string().as_bytes())?;
    for entry in &gone {
        if let Some(name) = archive_name(entry) {
            match fs::remove_file(directory.join(name)) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(io("delete the archive", &error)),
            }
        }
    }
    Ok(())
}
