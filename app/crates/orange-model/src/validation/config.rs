//! Node configuration against the definition's stored `config_schema` (12 §3.2).
//!
//! The schema language is **not** JSON Schema. It is the small, explicitly versioned dialect
//! [`CONFIG_SCHEMA_DIALECT`]; any construct outside it makes the schema unsupported and is
//! reported (`E-CONFIG-SCHEMA-UNSUPPORTED`) instead of being ignored, so an unrecognized
//! dialect can never silently accept a configuration.

use std::cmp::Ordering;

use serde_json::{Map, Number, Value};

use crate::catalog::RegistrySnapshot;
use crate::document::{BodyRef, GraphDocument};

use super::diagnostic::{Diagnostic, DiagnosticCode, Subject, sort_diagnostics};

/// Identifier of the only supported config-schema dialect. A schema may state it in
/// `$schema`; any other `$schema` value is unsupported.
pub const CONFIG_SCHEMA_DIALECT: &str = "orange.config-schema/v1";

/// Nesting limit for schema documents; deeper schemas are unsupported.
const MAX_SCHEMA_DEPTH: usize = 16;

/// Dialect v1 keywords. Annotations are accepted and ignored; the rest are enforced.
///
/// - `type`: one of `object`, `array`, `string`, `integer`, `number`, `boolean`, `null`
///   (`integer` accepts only JSON integers, so `1.0` is a number but not an integer).
/// - `enum`: non-empty array; the value must equal a member.
/// - `minimum`, `maximum`: finite numbers, inclusive, applied to numbers only.
/// - `required`: array of property names, applied to objects only.
/// - `properties`: object of sub-schemas, applied to present properties of objects.
/// - `additionalProperties`: boolean only; `false` rejects properties not in `properties`.
/// - `default`, `title`, `description`, `$comment`, `$schema`: annotations.
const KEYWORDS: &[&str] = &[
    "type",
    "enum",
    "minimum",
    "maximum",
    "required",
    "properties",
    "additionalProperties",
    "default",
    "title",
    "description",
    "$comment",
    "$schema",
];

const TYPES: &[&str] = &[
    "object", "array", "string", "integer", "number", "boolean", "null",
];

/// Whether `schema` uses only the supported dialect, with well-formed keyword values.
pub fn config_schema_is_supported(schema: &Value) -> bool {
    supported(schema, 0)
}

fn supported(schema: &Value, depth: usize) -> bool {
    let Some(object) = schema.as_object() else {
        return false;
    };
    if depth > MAX_SCHEMA_DEPTH || object.keys().any(|key| !KEYWORDS.contains(&key.as_str())) {
        return false;
    }
    let finite = |key: &str| {
        object
            .get(key)
            .is_none_or(|value| value.as_f64().is_some_and(f64::is_finite))
    };
    object
        .get("$schema")
        .is_none_or(|value| value.as_str() == Some(CONFIG_SCHEMA_DIALECT))
        && object
            .get("type")
            .is_none_or(|value| value.as_str().is_some_and(|name| TYPES.contains(&name)))
        && object
            .get("enum")
            .is_none_or(|value| value.as_array().is_some_and(|list| !list.is_empty()))
        && finite("minimum")
        && finite("maximum")
        && object.get("required").is_none_or(|value| {
            value
                .as_array()
                .is_some_and(|list| list.iter().all(Value::is_string))
        })
        && object
            .get("additionalProperties")
            .is_none_or(Value::is_boolean)
        && object.get("properties").is_none_or(|value| {
            value
                .as_object()
                .is_some_and(|map| map.values().all(|child| supported(child, depth + 1)))
        })
}

/// Whether `config` satisfies a schema already accepted by [`config_schema_is_supported`].
pub fn config_satisfies_schema(config: &Value, schema: &Value) -> bool {
    let Some(schema) = schema.as_object() else {
        return false;
    };
    satisfies(config, schema)
}

fn satisfies(value: &Value, schema: &Map<String, Value>) -> bool {
    if let Some(name) = schema.get("type").and_then(Value::as_str) {
        let matches = match name {
            "object" => value.is_object(),
            "array" => value.is_array(),
            "string" => value.is_string(),
            "integer" => value.is_i64() || value.is_u64(),
            "number" => value.is_number(),
            "boolean" => value.is_boolean(),
            _ => value.is_null(),
        };
        if !matches {
            return false;
        }
    }
    if let Some(members) = schema.get("enum").and_then(Value::as_array)
        && !members.contains(value)
    {
        return false;
    }
    if let Some(number) = value.as_number() {
        let below = schema
            .get("minimum")
            .and_then(Value::as_number)
            .is_some_and(|min| compare_numbers(number, min) == Ordering::Less);
        let above = schema
            .get("maximum")
            .and_then(Value::as_number)
            .is_some_and(|max| compare_numbers(number, max) == Ordering::Greater);
        if below || above {
            return false;
        }
    }
    let Some(object) = value.as_object() else {
        return true;
    };
    let required = schema
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str);
    if required.into_iter().any(|name| !object.contains_key(name)) {
        return false;
    }
    let properties = schema.get("properties").and_then(Value::as_object);
    if schema.get("additionalProperties") == Some(&Value::Bool(false))
        && object
            .keys()
            .any(|key| properties.is_none_or(|known| !known.contains_key(key)))
    {
        return false;
    }
    properties.is_none_or(|known| {
        known.iter().all(|(name, child)| {
            object
                .get(name)
                .is_none_or(|member| child.as_object().is_some_and(|c| satisfies(member, c)))
        })
    })
}

/// JSON retains signed/unsigned 64-bit integers separately from binary64 numbers. Never
/// round an integer to f64 before comparing a bound (adjacent integers above 2^53 merge).
fn compare_numbers(left: &Number, right: &Number) -> Ordering {
    let integer = |number: &Number| {
        number
            .as_i64()
            .map(i128::from)
            .or_else(|| number.as_u64().map(i128::from))
    };
    match (integer(left), integer(right)) {
        (Some(left), Some(right)) => left.cmp(&right),
        (Some(left), None) => compare_integer_float(left, right.as_f64().expect("JSON number")),
        (None, Some(right)) => {
            compare_integer_float(right, left.as_f64().expect("JSON number")).reverse()
        }
        (None, None) => left
            .as_f64()
            .expect("JSON number")
            .partial_cmp(&right.as_f64().expect("JSON number"))
            .expect("JSON numbers are finite"),
    }
}

fn compare_integer_float(integer: i128, float: f64) -> Ordering {
    // All JSON integers fit strictly inside i128. A saturating cast of a float outside
    // i128's range therefore orders correctly too. Within range, truncation is exact;
    // only a fractional part can break a tie (including negative fractions and -0.0).
    integer
        .cmp(&(float as i128))
        .then_with(|| 0.0f64.partial_cmp(&float.fract()).expect("finite number"))
}

/// Validates every node's `config` against its resolved definition's `config_schema`.
///
/// A definition without a schema constrains nothing here. A schema outside the supported
/// dialect reports `E-CONFIG-SCHEMA-UNSUPPORTED` on each node that uses it and its config is
/// not judged; otherwise a failing config reports `E-CONFIG-INVALID`. Diagnostics carry no
/// config values. Nodes whose definition is not in the registry are left to node resolution.
pub fn validate_config(document: &GraphDocument, registry: &RegistrySnapshot) -> Vec<Diagnostic> {
    let origin = registry.origin();
    let mut diagnostics = Vec::new();
    for (body_ref, body) in document.bodies_with_refs() {
        for node in body.nodes.values() {
            // A schema describes exactly its own config version; other versions are a
            // resolution matter and are not judged against it.
            let Some(schema) = registry
                .definition(&node.definition)
                .filter(|definition| definition.config_version == node.config_version)
                .and_then(|definition| definition.config_schema.as_ref())
            else {
                continue;
            };
            let code = if !config_schema_is_supported(schema) {
                DiagnosticCode::ConfigSchemaUnsupported
            } else if config_satisfies_schema(&Value::Object(node.config.clone()), schema) {
                continue;
            } else {
                DiagnosticCode::ConfigInvalid
            };
            let subject = Subject::new(Some(node.id), None, None);
            let subject = match body_ref {
                BodyRef::Root => subject,
                BodyRef::Function(function) => subject.with_function(function),
            };
            diagnostics.push(Diagnostic::new(code, subject, origin));
        }
    }
    sort_diagnostics(&mut diagnostics);
    diagnostics
}
