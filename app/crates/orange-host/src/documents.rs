//! Documents on disk: saving without losing the previous version, autosaving beside
//! the file, recovering after a crash, and bundles that carry a document together with
//! the packages it needs.
//!
//! None of this opens, checks or runs a graph. Saving refuses text that is not a
//! document; everything else treats a document as text.
use crate::HostError;
use orange_model::from_json;
use std::path::{Path, PathBuf};

/// Largest document or bundle these functions read.
pub const MAX_DOCUMENT_BYTES: u64 = 64 * 1024 * 1024;

fn io(what: &str, error: &std::io::Error) -> HostError {
    // An I/O message can name paths; only the operation and error kind are kept.
    HostError::Document(format!("{what}: {:?}", error.kind()))
}

fn beside(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn write_atomic(target: &Path, bytes: &[u8]) -> Result<(), HostError> {
    let temporary = beside(target, ".tmp");
    std::fs::write(&temporary, bytes).map_err(|e| io("write the document", &e))?;
    std::fs::rename(&temporary, target).map_err(|e| io("replace the document", &e))
}

fn read(path: &Path) -> Result<Option<String>, HostError> {
    match std::fs::metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io("read the document", &error)),
        Ok(metadata) if metadata.len() > MAX_DOCUMENT_BYTES => {
            return Err(HostError::Document("the file is too large".into()));
        }
        Ok(_) => {}
    }
    std::fs::read_to_string(path)
        .map(Some)
        .map_err(|e| io("read the document", &e))
}

fn is_document(text: &str) -> bool {
    from_json(text).is_ok()
}

/// Saves a document. The text must be a document; the file it replaces is kept beside
/// it as `<file>.bak`, the new text is written to a temporary file and renamed into
/// place, and the autosave of the file, now older than it, is removed. A crash at any
/// point leaves the previous file or the new one.
pub fn save_document(path: &Path, json: &str) -> Result<(), HostError> {
    if !is_document(json) {
        return Err(HostError::Document("the text is not a document".into()));
    }
    if path.exists() {
        // The backup is a copy, so the file itself is never missing.
        let _ = std::fs::copy(path, beside(path, ".bak"))
            .map_err(|e| io("keep the previous version", &e))?;
    }
    write_atomic(path, json.as_bytes())?;
    discard_autosave(path)
}

/// Writes the current, unsaved state of a document beside its file as
/// `<file>.autosave`, without touching the file. An editor calls it on a timer.
pub fn autosave_document(path: &Path, json: &str) -> Result<(), HostError> {
    write_atomic(&beside(path, ".autosave"), json.as_bytes())
}

/// Removes a document's autosave, as after the user declines to recover it.
pub fn discard_autosave(path: &Path) -> Result<(), HostError> {
    match std::fs::remove_file(beside(path, ".autosave")) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io("remove the autosave", &error)),
    }
}

/// What is on disk for one document path.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Recovery {
    /// The saved file, when it exists and is a document.
    pub saved: Option<String>,
    /// Work that was autosaved and never saved: present only when it is a document
    /// and differs from the saved file. An editor offers it to the user.
    pub autosave: Option<String>,
    /// The version before the last save, offered when the saved file is missing or
    /// is not a document.
    pub backup: Option<String>,
}

/// Reads what can be opened for a document path after any kind of exit: the saved
/// file, a newer autosave, and the backup if the saved file is damaged. Nothing is
/// changed on disk.
pub fn recover_document(path: &Path) -> Result<Recovery, HostError> {
    let document = |text: Option<String>| text.filter(|text| is_document(text));
    let saved = document(read(path)?);
    let autosave =
        document(read(&beside(path, ".autosave"))?).filter(|text| Some(text) != saved.as_ref());
    let backup = if saved.is_none() {
        document(read(&beside(path, ".bak"))?)
    } else {
        None
    };
    Ok(Recovery {
        saved,
        autosave,
        backup,
    })
}

/// One package a document is locked to, as [`crate::Host::requirements`] lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requirement {
    pub package: String,
    pub version: String,
    /// The content digest the document recorded for the package.
    pub content_digest: String,
    /// Whether this host has exactly that package: the same ID, version and content.
    pub available: bool,
}

const MAGIC: &[u8; 8] = b"\x89OGRB\r\n\x1a";

/// A document with the package archives it needs, as one file to hand to someone
/// else. Reading a bundle installs nothing and runs nothing: the receiver sees the
/// document and decides which archives to install, with which grants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bundle {
    /// The document text.
    pub document: String,
    /// The `.orangeplug` archives carried with it.
    pub packages: Vec<Vec<u8>>,
}

impl Bundle {
    /// The bundle as bytes: a signature, a format version, then the document and each
    /// archive, each preceded by its length.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&1u32.to_le_bytes());
        let mut part = |data: &[u8]| {
            bytes.extend_from_slice(&(data.len() as u64).to_le_bytes());
            bytes.extend_from_slice(data);
        };
        part(self.document.as_bytes());
        part(&(self.packages.len() as u64).to_le_bytes());
        for package in &self.packages {
            part(package);
        }
        bytes
    }

    /// Reads a bundle. The document must be a document; the archives are returned as
    /// they are and are checked when they are installed.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, HostError> {
        let refused = || HostError::Document("the file is not a version 1 graph bundle".into());
        let mut rest = bytes
            .strip_prefix(MAGIC)
            .and_then(|rest| rest.strip_prefix(&1u32.to_le_bytes()))
            .ok_or_else(refused)?;
        let mut part = || -> Option<&[u8]> {
            let (length, after) = rest.split_first_chunk::<8>()?;
            let length = usize::try_from(u64::from_le_bytes(*length)).ok()?;
            let (data, after) = after.split_at_checked(length)?;
            rest = after;
            Some(data)
        };
        let document = part()
            .and_then(|text| std::str::from_utf8(text).ok())
            .filter(|text| is_document(text))
            .ok_or_else(refused)?
            .to_owned();
        let count = part()
            .and_then(|count| <[u8; 8]>::try_from(count).ok())
            .map(u64::from_le_bytes)
            .filter(|count| *count <= 4096)
            .ok_or_else(refused)?;
        let mut packages = Vec::new();
        for _ in 0..count {
            packages.push(part().ok_or_else(refused)?.to_vec());
        }
        if !rest.is_empty() {
            return Err(refused());
        }
        Ok(Self { document, packages })
    }
}

/// How a function of a document is published as a package node.
#[derive(Debug, Clone, Copy)]
pub struct Publish<'a> {
    /// ID of the package to make, such as `org.example.tools`.
    pub package: &'a str,
    /// Version of the package.
    pub version: &'a str,
    /// Local name of the node, lowercase with hyphens: the node's ID is
    /// `<package>/<node>`.
    pub node: &'a str,
    /// The node's display name; the function's own name when `None`.
    pub label: Option<&'a str>,
}

/// Turns a function of a document into a package holding one node made of that
/// function: logic built once in a graph becomes a typed node that any document can
/// use, locked and versioned like a coded one. `function` is the function's ID or its
/// name. The result is a `.orangeplug` archive; installing it checks the body.
///
/// The function must be an action or async function without type parameters. Its
/// body may use only its own local variables, and at most one connection, which each
/// use of the node then names. Package nodes inside the body keep their exact
/// providers, so whoever installs the result needs those packages too.
pub fn publish_function(
    json: &str,
    function: &str,
    publish: &Publish<'_>,
) -> Result<Vec<u8>, HostError> {
    use serde_json::{Value as Json, json};
    let refused = |reason: &str| HostError::Document(reason.to_owned());
    if !is_document(json) {
        return Err(refused("the text is not a document"));
    }
    let document: Json = serde_json::from_str(json).map_err(|_| refused("not JSON"))?;
    let list = |value: &Json| value.as_array().cloned().unwrap_or_default();
    let found = list(&document["functions"])
        .into_iter()
        .find(|candidate| candidate["id"] == function || candidate["name"] == function)
        .ok_or_else(|| refused("the document has no such function"))?;
    let kind = found["kind"].as_str().unwrap_or_default().to_owned();
    let suspends = match kind.as_str() {
        "action" => "never",
        "async" => "maybe",
        _ => return Err(refused("only an action or async function can be published")),
    };
    if !list(&found["type_params"]).is_empty() {
        return Err(refused(
            "a function with type parameters cannot be published",
        ));
    }
    let slug = !publish.node.is_empty()
        && publish.node.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        });
    if !slug {
        return Err(refused(
            "the node name must be lowercase words joined by hyphens",
        ));
    }

    // What the body reaches outside itself: variables, a connection, package nodes.
    let nodes = list(&found["nodes"]);
    let mut variables = Vec::new();
    let mut connection: Option<String> = None;
    let (mut locks, mut signatures) = (Vec::new(), serde_json::Map::new());
    for node in &nodes {
        if let Some(id) = node["config"]["variable"].as_str() {
            let declared = list(&document["variables"])
                .into_iter()
                .find(|variable| variable["id"] == id)
                .filter(|variable| variable["scope"] == "local")
                .ok_or_else(|| {
                    refused("the function uses a variable that is not one of its local variables")
                })?;
            if !variables.contains(&declared) {
                variables.push(declared);
            }
        }
        if let Some(id) = node["config"]["connection"].as_str() {
            if connection.as_deref().is_some_and(|other| other != id) {
                return Err(refused("the function uses more than one connection"));
            }
            connection = Some(id.to_owned());
        }
        if let Some(provider) = node.get("provider") {
            let lock = list(&document["package_lock"]).into_iter().find(|entry| {
                entry["id"] == provider["package"] && entry["version"] == provider["version"]
            });
            if let Some(lock) = lock
                && !locks.contains(&lock)
            {
                locks.push(lock);
            }
        }
        if let Some(signature) = node["signature"].as_str()
            && let Some(snapshot) = document["signatures"].get(signature)
        {
            let _ = signatures.insert(signature.to_owned(), snapshot.clone());
        }
    }

    let id = format!("{}/{}", publish.package, publish.node);
    let label = publish
        .label
        .or_else(|| found["name"].as_str())
        .unwrap_or(publish.node);
    let mut ports = list(&found["inputs"]);
    for mut output in list(&found["outputs"]) {
        if output["kind"] == "data" {
            output["source"] = json!("execution");
            output["available_on"] = json!(["then"]);
        }
        ports.push(output);
    }
    let mut definition = json!({
        "definition_id": id,
        "label": {"key": format!("{id}-label"), "fallback": label},
        "definition_version": 1, "config_version": 1,
        "kind": kind, "suspends": suspends, "determinism": "external",
        "ports": ports
    });
    let mut graph = json!({
        "format": crate::GRAPH_NODE_FORMAT, "format_version": 1,
        "function": found, "variables": variables,
        "signatures": signatures, "package_lock": locks
    });
    if let Some(connection) = connection {
        definition["connection"] = json!(true);
        graph["connection"] = json!(connection);
    }
    let publisher: Vec<&str> = publish.package.split('.').take(2).collect();
    let manifest = json!({
        "format": "orange.plugin", "format_version": 1,
        "id": publish.package, "version": publish.version,
        "publisher": publisher.join("."),
        "about": {"name": label},
        "host_api": {"major": 1, "min_minor": 0, "max_minor": 0},
        "dependencies": [],
        "contributions": {
            "nodes": [id],
            "providers": [{"slot": format!("node:{id}@1"), "id": crate::GRAPH_PROVIDER}]
        },
        "permissions": []
    });
    crate::pack(
        manifest.to_string().as_bytes(),
        &[
            (
                format!("nodes/{}.json", publish.node),
                definition.to_string().into_bytes(),
            ),
            (
                format!("graphs/{}.json", publish.node),
                graph.to_string().into_bytes(),
            ),
        ],
    )
}
