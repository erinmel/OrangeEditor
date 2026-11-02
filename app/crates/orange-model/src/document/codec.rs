//! Format v1 JSON codec (12 §6). Owns the `"format"` envelope, the version gate and
//! document-wide structural invariants. Semantic validation is not done here.

use std::borrow::Cow;
use std::cell::Cell;
use std::collections::HashSet;
use std::fmt::Display;
use std::hash::Hash;
use std::rc::Rc;

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
/// pre-pass that keeps no values. [`ImportBudget`] adds limits for total JSON values, decoded
/// object-key bytes, and decoded string bytes. These checks run before any node or edge is
/// built; accepted documents are still materialized in memory, bounded by the byte and JSON
/// budgets.
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

/// Budgets for decoded JSON structure and text in one imported document.
///
/// Values include every container and primitive, including the root object. Key and string
/// budgets count decoded UTF-8 bytes, so JSON escapes count by their decoded character size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportBudget {
    pub max_values: usize,
    pub max_key_bytes: usize,
    pub max_string_bytes: usize,
}

impl ImportBudget {
    pub const fn new(max_values: usize, max_key_bytes: usize, max_string_bytes: usize) -> Self {
        Self {
            max_values,
            max_key_bytes,
            max_string_bytes,
        }
    }
}

impl Default for ImportBudget {
    fn default() -> Self {
        Self::new(1_000_000, 8 * 1024 * 1024, 32 * 1024 * 1024)
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
struct Counts {
    nodes: usize,
    edges: usize,
    nesting: usize,
    json_values: usize,
    json_key_bytes: usize,
    json_string_bytes: usize,
    budget: ImportBudget,
    violation: Rc<Cell<Option<(ImportResource, usize, usize)>>>,
}

impl Counts {
    fn new(
        budget: ImportBudget,
        violation: Rc<Cell<Option<(ImportResource, usize, usize)>>>,
    ) -> Self {
        Self {
            nodes: 0,
            edges: 0,
            nesting: 0,
            json_values: 0,
            json_key_bytes: 0,
            json_string_bytes: 0,
            budget,
            violation,
        }
    }

    fn charge<E: de::Error>(&mut self, resource: ImportResource, amount: usize) -> Result<(), E> {
        if self.violation.get().is_some() {
            return Err(E::custom("import budget exceeded"));
        }
        let (actual, max) = match resource {
            ImportResource::JsonValues => (&mut self.json_values, self.budget.max_values),
            ImportResource::JsonKeyBytes => (&mut self.json_key_bytes, self.budget.max_key_bytes),
            ImportResource::JsonStringBytes => {
                (&mut self.json_string_bytes, self.budget.max_string_bytes)
            }
            _ => return Ok(()),
        };
        *actual = actual.saturating_add(amount);
        if *actual > max {
            let actual = *actual;
            self.violation.set(Some((resource, max, actual)));
            return Err(E::custom("import budget exceeded"));
        }
        Ok(())
    }
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

struct HeaderSeed {
    budget: ImportBudget,
    violation: Rc<Cell<Option<(ImportResource, usize, usize)>>>,
}

impl<'de> DeserializeSeed<'de> for HeaderSeed {
    type Value = Header;
    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Header, D::Error> {
        deserializer.deserialize_map(HeaderVisitor {
            budget: self.budget,
            violation: self.violation,
        })
    }
}

struct HeaderVisitor {
    budget: ImportBudget,
    violation: Rc<Cell<Option<(ImportResource, usize, usize)>>>,
}

impl<'de> Visitor<'de> for HeaderVisitor {
    type Value = Header;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a graph document object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Header, A::Error> {
        let (mut format, mut format_version) = (None, None);
        let mut seen = Seen::default();
        let mut duplicate = None;
        let mut counts = Counts::new(self.budget, self.violation);
        counts.charge::<A::Error>(ImportResource::JsonValues, 1)?; // root object
        counts.nesting = 1;
        while let Some(Key(key)) = map.next_key()? {
            counts.charge::<A::Error>(ImportResource::JsonKeyBytes, key.len())?;
            let found = match key.as_ref() {
                "format" => {
                    counts.charge::<A::Error>(ImportResource::JsonValues, 1)?;
                    let value = map.next_value_seed(BudgetStringSeed {
                        counts: &mut counts,
                    })?;
                    format = Some(value);
                    None
                }
                "format_version" => {
                    counts.charge::<A::Error>(ImportResource::JsonValues, 1)?;
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

struct BudgetStringSeed<'c> {
    counts: &'c mut Counts,
}

impl<'de> DeserializeSeed<'de> for BudgetStringSeed<'_> {
    type Value = String;
    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<String, D::Error> {
        deserializer.deserialize_str(BudgetStringVisitor {
            counts: self.counts,
        })
    }
}

struct BudgetStringVisitor<'c> {
    counts: &'c mut Counts,
}

impl Visitor<'_> for BudgetStringVisitor<'_> {
    type Value = String;
    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a JSON string")
    }
    fn visit_str<E: de::Error>(self, value: &str) -> Result<String, E> {
        self.counts
            .charge::<E>(ImportResource::JsonStringBytes, value.len())?;
        Ok(value.to_owned())
    }
    fn visit_borrowed_str<E: de::Error>(self, value: &'_ str) -> Result<String, E> {
        self.counts
            .charge::<E>(ImportResource::JsonStringBytes, value.len())?;
        Ok(value.to_owned())
    }
    fn visit_string<E: de::Error>(self, value: String) -> Result<String, E> {
        self.counts
            .charge::<E>(ImportResource::JsonStringBytes, value.len())?;
        Ok(value)
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
        self.counts
            .charge::<D::Error>(ImportResource::JsonValues, 1)?;
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
    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        self.counts
            .charge::<E>(ImportResource::JsonStringBytes, value.len())?;
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
            counts.charge::<A::Error>(ImportResource::JsonKeyBytes, key.len())?;
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
    from_json_with_budget(text, ImportLimits::default(), ImportBudget::default())
}

/// Loads a format v1 document subject to caller-supplied import limits.
///
/// Use this function, or [`from_json`] with its default limits, for graph text from an
/// untrusted source. Hosts reading files or other streams should also bound the bytes
/// they read before constructing `text`; this function can enforce limits only after
/// the caller has allocated the input string. Direct Serde deserialization of
/// [`GraphDocument`] applies structural, identity and nesting checks, but cannot detect
/// duplicate JSON object keys or enforce the importer's byte, graph-item and decoded-JSON
/// quotas.
///
/// This compatibility entry point uses [`ImportBudget::default`] for decoded JSON values,
/// object-key bytes and string bytes. A JSON-resource violation found during the streaming
/// prepass takes precedence over format/version/duplicate diagnostics; without one, the
/// normal ordering below applies. Hosts may select a larger profile through
/// [`from_json_with_budget`]. The byte limit is enforced before parsing. One streaming pass
/// reads the envelope, finds repeated keys, counts body nodes/edges and enforces JSON budgets
/// before building a DOM. A JSON budget hit takes precedence over later format, version or
/// duplicate-key errors because scanning stops at the first resource violation. Otherwise,
/// errors come in this order: format, version, duplicate key, node quota, edge quota,
/// nesting, then shape and identity errors from building the document. Nesting deeper than
/// the JSON parser's own recursion bound is reported as a JSON error. Quotas therefore bound
/// what is built: an over-quota document fails before any node or edge is materialized.
pub fn from_json_with_limits(
    text: &str,
    limits: ImportLimits,
) -> Result<GraphDocument, CodecError> {
    from_json_with_budget(text, limits, ImportBudget::default())
}

/// Loads a format v1 document with explicit byte, graph-item and decoded-JSON budgets.
///
/// For untrusted streams, bound the host read before decoding UTF-8 or calling this importer:
///
/// ```
/// use std::io::{Cursor, Read};
/// use orange_model::{ImportBudget, ImportLimits, from_json_with_budget};
///
/// fn import<R: Read>(reader: R) -> Result<(), Box<dyn std::error::Error>> {
///     let max_bytes = ImportLimits::default().max_bytes;
///     let mut limited = reader.take((max_bytes + 1) as u64);
///     let mut bytes = Vec::new();
///     limited.read_to_end(&mut bytes)?;
///     if bytes.len() > max_bytes { return Err("document byte limit exceeded".into()); }
///     let text = std::str::from_utf8(&bytes)?;
///     let _document = from_json_with_budget(text, ImportLimits::default(), ImportBudget::default())?;
///     Ok(())
/// }
///
/// let source = r#"{"format":"orange.graph","format_version":1,"graph_id":"a0f3822f-9cf8-4ab6-a2bf-e21fd345c3c6","revision":0,"package_lock":[],"type_lock":[],"variables":[],"events":[],"signatures":{},"nodes":[],"edges":[],"functions":[],"exports":[],"editor":{}}"#;
/// import(Cursor::new(source.as_bytes()))?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn from_json_with_budget(
    text: &str,
    limits: ImportLimits,
    budget: ImportBudget,
) -> Result<GraphDocument, CodecError> {
    enforce_limit(ImportResource::Bytes, text.len(), limits.max_bytes)?;
    let violation = Rc::new(Cell::new(None));
    let mut deserializer = serde_json::Deserializer::from_str(text);
    let header = match (HeaderSeed {
        budget,
        violation: Rc::clone(&violation),
    })
    .deserialize(&mut deserializer)
    {
        Ok(header) => header,
        Err(error) => {
            if let Some((resource, max, actual)) = violation.get() {
                return Err(CodecError::ImportLimitExceeded {
                    resource,
                    max,
                    actual,
                });
            }
            return Err(error.into());
        }
    };
    deserializer.end()?;
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
