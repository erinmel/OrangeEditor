//! Format v1 JSON codec (12 §6). Owns the `"format"` envelope, the version gate and
//! document-wide structural invariants. Semantic validation is not done here.

use std::collections::HashSet;
use std::fmt::Display;
use std::hash::Hash;

use serde::{Deserialize, Serialize};

use crate::document::GraphDocument;
use crate::error::CodecError;

pub const FORMAT: &str = "orange.graph";
pub const FORMAT_VERSION: u32 = 1;

#[derive(Deserialize)]
struct Header {
    format: String,
    format_version: u32,
}

#[derive(Deserialize)]
struct EnvelopeIn {
    #[allow(dead_code)] // checked through `Header` before the full parse
    format: String,
    #[serde(flatten)]
    document: GraphDocument,
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
    let document = serde_json::from_str::<EnvelopeIn>(text)?.document;
    check_unique_across_bodies(&document)?;
    Ok(document)
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

fn first_duplicate<'a, K: Eq + Hash + Display + 'a>(
    keys: impl Iterator<Item = &'a K>,
) -> Option<String> {
    let mut seen = HashSet::new();
    keys.into_iter()
        .find(|key| !seen.insert(*key))
        .map(ToString::to_string)
}
