//! Format v1 JSON codec (12 §6). Owns the `"format"` envelope, the version gate and
//! document-wide structural invariants. Semantic validation is not done here.

use std::borrow::Cow;
use std::collections::HashSet;
use std::fmt::Display;
use std::hash::Hash;

use serde::de::{self, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};

use crate::document::GraphDocument;
use crate::document::graph::DocumentWire;
use crate::document::nesting::MAX_NESTING;
use crate::error::{CodecError, ImportResource};

pub const FORMAT: &str = "orange.graph";
pub const FORMAT_VERSION: u32 = 1;

/// Configurable safety limits for loading one graph document.
///
/// The defaults are 64 MiB of JSON text, 100,000 nodes, and 500,000 edges. `max_bytes` is
/// checked before parsing. Node and edge limits are checked on counts taken by a streaming
/// pre-pass that keeps no values, so an over-quota document is rejected before any node or
/// edge is built. A document within its quotas is still built in memory proportional to
/// its size, which `max_bytes` bounds.
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

/// The first pass over the whole text: the envelope fields, the first repeated object key
/// at any depth (Serde maps would otherwise keep the last value silently), and the node and
/// edge counts of every body. It keeps no values, so quotas apply before anything is built.
struct Header {
    format: String,
    format_version: u32,
    duplicate: Option<String>,
    counts: Counts,
}

/// Array elements under the root's and each function's `nodes` and `edges`, and the
/// deepest container nesting seen, the root object being 1.
#[derive(Default)]
struct Counts {
    nodes: usize,
    edges: usize,
    nesting: usize,
}

/// Where a scanned value sits in the document, so only graph bodies are counted.
#[derive(Clone, Copy)]
enum Place {
    Root,
    Functions,
    Function,
    Nodes,
    Edges,
    Other,
}

impl Place {
    /// The place of the value under `key` in an object at this place.
    fn field(self, key: &str) -> Place {
        match (self, key) {
            (Place::Root | Place::Function, "nodes") => Place::Nodes,
            (Place::Root | Place::Function, "edges") => Place::Edges,
            (Place::Root, "functions") => Place::Functions,
            _ => Place::Other,
        }
    }

    /// The place of each element of an array at this place.
    fn element(self) -> Place {
        match self {
            Place::Functions => Place::Function,
            _ => Place::Other,
        }
    }
}

impl Counts {
    fn add_element(&mut self, array: Place) {
        match array {
            Place::Nodes => self.nodes = self.nodes.saturating_add(1),
            Place::Edges => self.edges = self.edges.saturating_add(1),
            _ => {}
        }
    }
}

impl<'de> Deserialize<'de> for Header {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(HeaderVisitor)
    }
}

struct HeaderVisitor;

impl<'de> Visitor<'de> for HeaderVisitor {
    type Value = Header;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a graph document object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Header, A::Error> {
        let (mut format, mut format_version) = (None, None);
        let mut seen = Seen::default();
        let mut duplicate = None;
        let mut counts = Counts {
            nesting: 1,
            ..Counts::default()
        };
        while let Some(Key(key)) = map.next_key()? {
            let found = match key.as_ref() {
                "format" => {
                    format = Some(map.next_value::<String>()?);
                    None
                }
                "format_version" => {
                    format_version = Some(map.next_value::<u32>()?);
                    None
                }
                other => map.next_value_seed(Scan {
                    place: Place::Root.field(other),
                    depth: 2,
                    counts: &mut counts,
                })?,
            };
            note_key(&mut seen, key, &mut duplicate);
            duplicate = duplicate.or(found);
        }
        Ok(Header {
            format: format.ok_or_else(|| de::Error::missing_field("format"))?,
            format_version: format_version
                .ok_or_else(|| de::Error::missing_field("format_version"))?,
            duplicate,
            counts,
        })
    }
}

/// Any JSON value at `place`, yielding only the first repeated object key found inside it
/// and adding the body items it holds to `counts`. Shape is not checked here, so a newer
/// document still reaches the version gate. `depth` is the nesting this value has if it is
/// a container.
struct Scan<'c> {
    place: Place,
    depth: usize,
    counts: &'c mut Counts,
}

impl<'de> DeserializeSeed<'de> for Scan<'_> {
    type Value = Option<String>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Scan<'_> {
    type Value = Option<String>;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("any JSON value")
    }
    fn visit_bool<E>(self, _: bool) -> Result<Self::Value, E> {
        Ok(None)
    }
    fn visit_i64<E>(self, _: i64) -> Result<Self::Value, E> {
        Ok(None)
    }
    fn visit_u64<E>(self, _: u64) -> Result<Self::Value, E> {
        Ok(None)
    }
    fn visit_f64<E>(self, _: f64) -> Result<Self::Value, E> {
        Ok(None)
    }
    fn visit_str<E>(self, _: &str) -> Result<Self::Value, E> {
        Ok(None)
    }
    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(None)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let Scan {
            place,
            depth,
            counts,
        } = self;
        counts.nesting = counts.nesting.max(depth);
        let mut duplicate = None;
        while let Some(found) = seq.next_element_seed(Scan {
            place: place.element(),
            depth: depth + 1,
            counts: &mut *counts,
        })? {
            counts.add_element(place);
            duplicate = duplicate.or(found);
        }
        Ok(duplicate)
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let Scan {
            place,
            depth,
            counts,
        } = self;
        counts.nesting = counts.nesting.max(depth);
        let mut seen = Seen::default();
        let mut duplicate = None;
        while let Some(Key(key)) = map.next_key()? {
            let child = place.field(&key);
            note_key(&mut seen, key, &mut duplicate);
            let found = map.next_value_seed(Scan {
                place: child,
                depth: depth + 1,
                counts: &mut *counts,
            })?;
            duplicate = duplicate.or(found);
        }
        Ok(duplicate)
    }
}

/// An object key, borrowed from the text when it has no escapes.
struct Key<'de>(Cow<'de, str>);

impl<'de> Deserialize<'de> for Key<'de> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_str(KeyVisitor)
    }
}

struct KeyVisitor;

impl<'de> Visitor<'de> for KeyVisitor {
    type Value = Key<'de>;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("an object key")
    }
    fn visit_borrowed_str<E>(self, key: &'de str) -> Result<Key<'de>, E> {
        Ok(Key(Cow::Borrowed(key)))
    }
    fn visit_str<E>(self, key: &str) -> Result<Key<'de>, E> {
        Ok(Key(Cow::Owned(key.to_owned())))
    }
}

/// Keys of one object. Most document objects are small, so a short list is searched
/// before a hash set is built.
#[derive(Default)]
struct Seen<'de> {
    few: Vec<Cow<'de, str>>,
    many: HashSet<Cow<'de, str>>,
}

impl<'de> Seen<'de> {
    const FEW: usize = 16;

    /// Hands `key` back if it was already present.
    fn insert(&mut self, key: Cow<'de, str>) -> Option<Cow<'de, str>> {
        if self.many.is_empty() {
            if self.few.contains(&key) {
                return Some(key);
            }
            if self.few.len() < Self::FEW {
                self.few.push(key);
                return None;
            }
            self.many.extend(self.few.drain(..));
        }
        if self.many.contains(&key) {
            return Some(key);
        }
        self.many.insert(key);
        None
    }
}

/// Records `key`, keeping the first repeat in document order, shortened for messages.
fn note_key<'de>(seen: &mut Seen<'de>, key: Cow<'de, str>, duplicate: &mut Option<String>) {
    if let Some(repeat) = seen.insert(key)
        && duplicate.is_none()
    {
        *duplicate = Some(repeat.chars().take(64).collect());
    }
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
/// The byte limit is enforced before parsing. One streaming pass then reads the envelope,
/// finds repeated keys and counts the nodes and edges of the root and every function body.
/// Errors come in this order: format, version, duplicate key, node quota, edge quota,
/// nesting, then shape and identity errors from building the document. Nesting deeper than
/// the JSON parser's own recursion bound is reported as a JSON error. Quotas therefore bound what is
/// built: an over-quota document fails before any node or edge is materialized.
pub fn from_json_with_limits(
    text: &str,
    limits: ImportLimits,
) -> Result<GraphDocument, CodecError> {
    enforce_limit(ImportResource::Bytes, text.len(), limits.max_bytes)?;
    let header: Header = serde_json::from_str(text)?;
    if header.format != FORMAT {
        return Err(CodecError::UnsupportedFormat {
            found: header.format.into(),
        });
    }
    if header.format_version != FORMAT_VERSION {
        return Err(CodecError::UnsupportedVersion {
            found: header.format_version,
        });
    }
    // After the version gate, so a newer document still reports its version.
    if let Some(key) = header.duplicate {
        return Err(CodecError::DuplicateKey { key: key.into() });
    }
    enforce_limit(ImportResource::Nodes, header.counts.nodes, limits.max_nodes)?;
    enforce_limit(ImportResource::Edges, header.counts.edges, limits.max_edges)?;
    check_nesting(header.counts.nesting)?;
    // Deserialize the body directly: wrapping the entire graph in another flattened
    // struct buffers it through Serde's intermediate representation a second time. The
    // unchecked wire form is read so `admit` reports typed codec errors.
    let mut json = serde_json::Deserializer::from_str(text);
    let document = DocumentWire::deserialize(&mut json)?;
    json.end()?;
    admit(document)
}

/// The structural checks shared by every construction from serialized data, through
/// [`from_json`] or the Serde `Deserialize` impl: the envelope tag when present, the
/// version, node and edge identities across bodies, and lock identities. `format` is
/// envelope-owned, not an extension, so it is removed; other extension order is kept.
pub(crate) fn admit(mut document: GraphDocument) -> Result<GraphDocument, CodecError> {
    if let Some(format) = document.extensions.shift_remove("format")
        && format.as_str() != Some(FORMAT)
    {
        return Err(CodecError::UnsupportedFormat {
            found: format
                .as_str()
                .map_or_else(|| format.to_string(), str::to_owned)
                .into(),
        });
    }
    if document.format_version != FORMAT_VERSION {
        return Err(CodecError::UnsupportedVersion {
            found: document.format_version,
        });
    }
    check_unique_across_bodies(&document)?;
    check_unique_locks(&document)?;
    Ok(document)
}

/// The shared budget of [`MAX_NESTING`], so a loaded document stays editable and savable.
pub(crate) fn check_nesting(nesting: usize) -> Result<(), CodecError> {
    if nesting > MAX_NESTING {
        return Err(CodecError::NestingLimit { max: MAX_NESTING });
    }
    Ok(())
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
                identity: format!("{}@{}", lock.id, lock.version).into(),
            });
        }
    }

    let mut types = HashSet::new();
    for lock in document.type_lock() {
        let identity = lock.ty.as_str();
        if !types.insert(identity) {
            return Err(CodecError::DuplicateLock {
                kind: "type",
                identity: identity.into(),
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
