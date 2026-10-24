//! Format v1 JSON codec (12 §6). Owns the `"format"` envelope, the version gate and
//! document-wide structural invariants. Semantic validation is not done here.

use std::collections::HashSet;
use std::fmt::Display;
use std::hash::Hash;

use serde::{Deserialize, Serialize};

use crate::document::GraphDocument;
use crate::error::{CodecError, ImportResource};

pub const FORMAT: &str = "orange.graph";
pub const FORMAT_VERSION: u32 = 1;

/// Configurable safety limits for loading one graph document.
///
/// The defaults are 64 MiB of JSON text, 100,000 nodes, and 500,000 edges. Node and edge
/// limits are checked after structural deserialization; only `max_bytes` prevents the
/// parser from allocating in proportion to an oversized input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportLimits {
    pub max_bytes: usize,
    pub max_nodes: usize,
    pub max_edges: usize,
}

impl ImportLimits {
    pub const fn new(max_bytes: usize, max_nodes: usize, max_edges: usize) -> Self {
        Self {
            max_bytes,
            max_nodes,
            max_edges,
        }
    }
}

impl Default for ImportLimits {
    fn default() -> Self {
        Self::new(64 * 1024 * 1024, 100_000, 500_000)
    }
}

#[derive(Deserialize)]
struct Header {
    format: String,
    format_version: u32,
}

#[derive(Serialize)]
struct EnvelopeOut<'a> {
    format: &'static str,
    #[serde(flatten)]
    document: &'a GraphDocument,
}

/// Loads a format v1 document. The format and version are checked before the body is
/// parsed, so a newer document reports its version instead of a shape error.
pub fn from_json(text: &str) -> Result<GraphDocument, CodecError> {
    from_json_with_limits(text, ImportLimits::default())
}

/// Loads a format v1 document subject to caller-supplied import limits.
///
/// The byte limit is enforced before parsing. Node and edge limits are structural checks
/// after deserialization and therefore do not bound the allocations made while parsing.
pub fn from_json_with_limits(
    text: &str,
    limits: ImportLimits,
) -> Result<GraphDocument, CodecError> {
    enforce_limit(ImportResource::Bytes, text.len(), limits.max_bytes)?;
    let header: Header = serde_json::from_str(text)?;
    if header.format != FORMAT {
        return Err(CodecError::UnsupportedFormat {
            found: header.format,
        });
    }
    if header.format_version != FORMAT_VERSION {
        return Err(CodecError::UnsupportedVersion {
            found: header.format_version,
        });
    }
    // Deserialize the body directly: wrapping the entire graph in another flattened
    // struct buffers it through Serde's intermediate representation a second time.
    // `format` is envelope-owned, not an extension; keep other extension key order.
    let mut document: GraphDocument = serde_json::from_str(text)?;
    document.extensions.shift_remove("format");
    check_unique_across_bodies(&document)?;
    let (nodes, edges) = document
        .bodies()
        .fold((0usize, 0usize), |(nodes, edges), body| {
            (
                nodes.saturating_add(body.nodes.len()),
                edges.saturating_add(body.edges.len()),
            )
        });
    enforce_limit(ImportResource::Nodes, nodes, limits.max_nodes)?;
    enforce_limit(ImportResource::Edges, edges, limits.max_edges)?;
    check_unique_locks(&document)?;
    Ok(document)
}

fn enforce_limit(resource: ImportResource, actual: usize, max: usize) -> Result<(), CodecError> {
    if actual > max {
        return Err(CodecError::ImportLimitExceeded {
            resource,
            max,
            actual,
        });
    }
    Ok(())
}

/// Saves a document as compact JSON with a trailing newline. The output depends only on
/// the document value, so saving the same document twice yields identical bytes.
pub fn to_json(document: &GraphDocument) -> Result<String, CodecError> {
    let mut text = serde_json::to_string(&EnvelopeOut {
        format: FORMAT,
        document,
    })?;
    text.push('\n');
    Ok(text)
}

/// Node and edge IDs are unique across the root and all function bodies (12 §6). Within
/// one body the keyed maps already reject duplicates while parsing.
fn check_unique_across_bodies(document: &GraphDocument) -> Result<(), CodecError> {
    let bodies = || document.bodies();
    first_duplicate(bodies().flat_map(|body| body.nodes.keys())).map_or(Ok(()), |id| {
        Err(CodecError::DuplicateId { kind: "node", id })
    })?;
    first_duplicate(bodies().flat_map(|body| body.edges.keys())).map_or(Ok(()), |id| {
        Err(CodecError::DuplicateId { kind: "edge", id })
    })
}

/// Lock identity is independent of the lock payload: duplicate entries are ambiguous even
/// when their values happen to agree.
fn check_unique_locks(document: &GraphDocument) -> Result<(), CodecError> {
    let mut packages = HashSet::new();
    for lock in document.package_lock() {
        let identity = (lock.id.as_str(), lock.version.as_str());
        if !packages.insert(identity) {
            return Err(CodecError::DuplicateLock {
                kind: "package",
                identity: format!("{}@{}", lock.id, lock.version),
            });
        }
    }

    let mut types = HashSet::new();
    for lock in document.type_lock() {
        let identity = lock.ty.as_str();
        if !types.insert(identity) {
            return Err(CodecError::DuplicateLock {
                kind: "type",
                identity: identity.to_owned(),
            });
        }
    }
    Ok(())
}

fn first_duplicate<'a, K: Eq + Hash + Display + 'a>(
    keys: impl Iterator<Item = &'a K>,
) -> Option<String> {
    let mut seen = HashSet::new();
    keys.into_iter()
        .find(|key| !seen.insert(*key))
        .map(ToString::to_string)
}
