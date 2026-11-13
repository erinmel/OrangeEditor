//! Persistent variables on disk, so a flow started again has the values it ended with.
//!
//! A host given a state directory keeps one file per graph, `<graph ID>.variables.json`,
//! holding the run-scoped variables that graph declares `persistent`, and one file,
//! `shared.variables.json`, for the persistent shared variables. Each is
//! `{"format": "orange.variables", "format_version": 1, "variables": [{"id", "name",
//! "literal"}]}` with the value as a document literal, so a person can read and edit
//! it. A file is written to a temporary name and renamed, so a crash leaves the
//! previous values or the new ones.
use orange_model::Value;
use orange_model::document::literal::Literal;
use orange_model::document::{GraphDocument, VarScope};
use orange_model::id::{TypeExpr, VariableId};
use orange_model::types::TypeRegistry;
use orange_model::value::literal_codec;
use serde_json::{Value as Json, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const FORMAT: &str = "orange.variables";
/// Name of the shared variables' file, without the suffix.
pub(crate) const SHARED: &str = "shared";

/// One variable a host keeps: its ID, display name and declared type.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Kept {
    pub id: VariableId,
    pub name: String,
    pub ty: TypeExpr,
}

/// The variables of `scope` that `document` declares persistent.
pub(crate) fn kept(document: &GraphDocument, scope: VarScope) -> Vec<Kept> {
    document
        .variables()
        .values()
        .filter(|variable| variable.persistent && variable.scope == scope)
        .map(|variable| Kept {
            id: variable.id,
            name: variable.name.clone(),
            ty: variable.ty.clone(),
        })
        .collect()
}

fn file(directory: &Path, name: &str) -> PathBuf {
    directory.join(format!("{name}.variables.json"))
}

fn io(what: &str, error: &std::io::Error) -> String {
    // An I/O message can name paths; only the operation and error kind are kept.
    format!("{what}: {:?}", error.kind())
}

/// One stored variable, as [`crate::Host::stored_variables`] returns it.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredVariable {
    pub id: VariableId,
    /// The display name the variable had when it was stored.
    pub name: String,
    /// The value as a document literal: its type and its JSON form.
    pub literal: Literal,
}

/// Everything stored under `name`; empty when nothing was stored yet.
pub(crate) fn load(directory: &Path, name: &str) -> Result<Vec<StoredVariable>, String> {
    let text = match std::fs::read_to_string(file(directory, name)) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(io("read the stored variables", &error)),
    };
    let stored: Json =
        serde_json::from_str(&text).map_err(|_| "the stored variables are not JSON".to_owned())?;
    if stored["format"] != FORMAT || stored["format_version"] != 1 {
        return Err(format!("expected a `{FORMAT}` version 1 file"));
    }
    let entries = stored["variables"]
        .as_array()
        .ok_or("the stored variables list no `variables`")?;
    let mut variables = Vec::with_capacity(entries.len());
    for entry in entries {
        let parsed = entry["id"]
            .as_str()
            .and_then(|id| id.parse().ok())
            .zip(serde_json::from_value::<Literal>(entry["literal"].clone()).ok());
        let (id, literal) = parsed.ok_or("a stored variable has no valid `id` and `literal`")?;
        variables.push(StoredVariable {
            id,
            name: entry["name"].as_str().unwrap_or_default().to_owned(),
            literal,
        });
    }
    Ok(variables)
}

/// Replaces what is stored under `name`.
pub(crate) fn save(
    directory: &Path,
    name: &str,
    variables: &[StoredVariable],
) -> Result<(), String> {
    std::fs::create_dir_all(directory).map_err(|e| io("create the state directory", &e))?;
    let entries: Vec<Json> = variables
        .iter()
        .map(|variable| {
            json!({"id": variable.id.to_string(), "name": variable.name,
                "literal": variable.literal})
        })
        .collect();
    let text = json!({"format": FORMAT, "format_version": 1, "variables": entries});
    let target = file(directory, name);
    let temporary = directory.join(format!("{name}.variables.json.tmp"));
    std::fs::write(&temporary, format!("{text:#}\n"))
        .map_err(|e| io("write the stored variables", &e))?;
    std::fs::rename(&temporary, &target).map_err(|e| io("replace the stored variables", &e))
}

/// The stored values that still fit `kept`: same ID and same declared type, decoding
/// to a value. A variable that was removed or changed type is left out, so it starts
/// from its initial value.
pub(crate) fn restore(
    stored: &[StoredVariable],
    kept: &[Kept],
    types: &TypeRegistry,
) -> BTreeMap<VariableId, Value> {
    let mut values = BTreeMap::new();
    for variable in stored {
        let fits = kept
            .iter()
            .any(|kept| kept.id == variable.id && kept.ty == variable.literal.ty);
        if fits && let Ok(value) = literal_codec::decode(&variable.literal, types) {
            let _ = values.insert(variable.id, value);
        }
    }
    values
}

/// What to store for `kept` given each variable's value now. An unset variable, or
/// one whose value cannot be written as a literal, is left out.
pub(crate) fn snapshot(
    kept: &[Kept],
    types: &TypeRegistry,
    mut value: impl FnMut(VariableId) -> Option<Value>,
) -> Vec<StoredVariable> {
    kept.iter()
        .filter_map(|kept| {
            let literal = literal_codec::encode(&kept.ty, &value(kept.id)?, types).ok()?;
            Some(StoredVariable {
                id: kept.id,
                name: kept.name.clone(),
                literal,
            })
        })
        .collect()
}
