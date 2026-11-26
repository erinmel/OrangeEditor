//! Which of a node's settings its card shows, and what they are called. Their controls
//! are in `fields`; tuning settings are only in Properties (22 §3.1).
use super::*;
use crate::properties::watch_field_entries;
use orange_host::intrinsic;

/// The settings of `node` that name one of the document's variables, as JSON pointers.
pub(super) fn variable_pointers(node: &NodeView) -> impl Iterator<Item = &str> {
    node.references
        .iter()
        .filter(|(_, kind)| *kind == orange_host::ReferenceKind::Variable)
        .map(|(pointer, _)| pointer.as_str())
}

/// The settings a person can edit in a field: not read-only, not a reference to a
/// declaration, and of a kind a field holds.
fn all_fields(node: &NodeView) -> impl Iterator<Item = (&String, &serde_json::Value)> {
    node.schema
        .as_ref()
        .and_then(|schema| schema["properties"].as_object())
        .into_iter()
        .flat_map(|properties| properties.iter())
        .filter(|(name, schema)| {
            !schema["readOnly"].as_bool().unwrap_or(false)
                && !node
                    .references
                    .iter()
                    .any(|(pointer, _)| pointer == &format!("/{name}"))
        })
        .filter(|(_, schema)| {
            schema["enum"].is_array()
                || matches!(
                    schema["type"].as_str(),
                    Some("string" | "boolean" | "number" | "integer")
                )
        })
}

/// Tuning settings: in Properties under Advanced, never on the card (22 §3.1). A
/// definition puts more there with its layout's `advanced` region (ADR-33).
pub(crate) fn advanced(node: &NodeView, name: &str) -> bool {
    matches!(
        name,
        "int_overflow" | "float_policy" | "window_ms" | "max_iterations"
    ) || node
        .controls
        .region_of(name)
        .is_some_and(|(region, _)| region == orange_host::controls::Region::Advanced)
}

/// The settings a card shows, in order: When's in the order its mode needs, others as
/// the schema lists them.
pub(super) fn fields(node: &NodeView) -> impl Iterator<Item = (&String, &serde_json::Value)> {
    watch_field_entries(node).chain(
        all_fields(node)
            .filter(|(name, _)| node.definition != intrinsic::ON_WATCH && !advanced(node, name)),
    )
}

/// A setting's name as a person reads it: its title, or its key in words without a
/// unit suffix (the field shows the unit).
pub(super) fn label(name: &str, schema: &serde_json::Value) -> String {
    schema["title"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| {
            let name = name
                .strip_suffix("_hz")
                .or_else(|| name.strip_suffix("_db"))
                .or_else(|| name.strip_suffix("_ms"))
                .unwrap_or(name);
            let mut label = name.replace('_', " ");
            if let Some(first) = label.get_mut(..1) {
                first.make_ascii_uppercase();
            }
            label
        })
}
