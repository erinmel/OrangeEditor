//! Enum-typed variable and event declarations and enum map keys (11 §5.2, §5.4,
//! §10.2, §11.1): declarations resolve through the registry's schemas, initializers decode
//! against the current schema, and map keys need `hash` + `eq`.

use crate::common;

use std::sync::Arc;

use common::{minimal_document, node_id};
use orange_model::catalog::{RegistryBuilder, RegistrySnapshot};
use orange_model::type_term::TypeTerm;
use orange_model::types::{Constraint, TypeRegistryBuilder};
use orange_model::validation::{DiagnosticCode, HostOrigin};
use orange_model::value::literal_codec::{EnumSchema, VariantSchema};
use orange_model::{analyze, from_json};
use serde_json::{Value, json};

const LIGHT: &str = "org.example/light@1";
const MOOD: &str = "org.example/mood@1";

/// `light` (red, amber(u32), green, then `extra`) derives `eq` and `hash`; `mood`
/// (calm, busy) derives `eq` only.
fn registry(extra: &[&str]) -> Arc<RegistrySnapshot> {
    let mut types = TypeRegistryBuilder::core();
    let variant = |id: &str, payload: Option<&str>| VariantSchema {
        variant_id: id.into(),
        payload: payload.map(|ty| TypeTerm::parse(&ty.parse().unwrap()).unwrap()),
    };
    let mut lights = vec![
        variant("red", None),
        variant("amber", Some("core/u32@1")),
        variant("green", None),
    ];
    lights.extend(extra.iter().map(|id| variant(id, None)));
    let moods = vec![variant("calm", None), variant("busy", None)];
    for (ty, variants, constraints) in [
        (LIGHT, lights, &[Constraint::Eq, Constraint::Hash][..]),
        (MOOD, moods, &[Constraint::Eq][..]),
    ] {
        let ty = ty.parse().unwrap();
        types
            .register_enum("org.example", &ty, EnumSchema { variants })
            .unwrap();
        for constraint in constraints {
            types
                .derive_constraint("org.example", &ty, *constraint)
                .unwrap();
        }
    }
    let origin = HostOrigin::new("0.1.0", format!("sha256:{}", "b".repeat(64))).unwrap();
    RegistryBuilder::core(origin)
        .types(Arc::new(types.build().unwrap()))
        .build()
        .unwrap()
}

fn variable(n: u8, ty: &str, init: Option<Value>) -> Value {
    let mut v = json!({"id": node_id(n), "name": "v", "type": ty, "scope": "run"});
    if let Some(value) = init {
        v["init"] = json!({"type": ty, "value": value});
    }
    v
}

fn event(n: u8, payload: &str) -> Value {
    json!({"id": node_id(n), "name": "e", "payload": payload, "scope": "run"})
}

fn codes(
    variables: Vec<Value>,
    events: Vec<Value>,
    registry: &RegistrySnapshot,
) -> Vec<DiagnosticCode> {
    let doc = from_json(&minimal_document(
        json!({"variables": variables, "events": events}),
    ))
    .unwrap();
    analyze(&doc, registry)
        .report()
        .diagnostics
        .iter()
        .map(|d| d.code)
        .collect()
}

#[test]
fn enum_declarations_decode_their_initializers_against_the_schema() {
    let registry = registry(&[]);
    assert_eq!(
        codes(
            vec![
                variable(1, LIGHT, Some(json!({"variant": "amber", "payload": 3}))),
                variable(2, LIGHT, Some(json!({"variant": "red"}))),
                variable(3, LIGHT, None),
                variable(
                    4,
                    &format!("core/option@1<{LIGHT}>"),
                    Some(json!({"none": true}))
                ),
            ],
            vec![event(10, LIGHT)],
            &registry,
        ),
        []
    );
    for invalid in [
        json!({"variant": "blue"}),
        json!({"variant": "amber"}),
        json!({"variant": "red", "payload": 1}),
        json!({"variant": "amber", "payload": "3"}),
        json!("red"),
    ] {
        assert_eq!(
            codes(
                vec![variable(1, LIGHT, Some(invalid.clone()))],
                vec![],
                &registry
            ),
            [DiagnosticCode::LiteralType],
            "{invalid}"
        );
    }
}

/// A map key must implement `hash` and `eq` (11 §5.2); an enum deriving only `eq` is no
/// key, for variables and event payloads alike.
#[test]
fn enum_map_keys_need_hash_and_eq() {
    let registry = registry(&[]);
    let keyed = format!("core/map@1<{LIGHT},core/u32@1>");
    let init = json!([[{"variant": "amber", "payload": 1}, 5], [{"variant": "red"}, 6]]);
    assert_eq!(
        codes(
            vec![variable(1, &keyed, Some(init))],
            vec![event(10, &keyed)],
            &registry
        ),
        []
    );
    let duplicate = json!([[{"variant": "red"}, 5], [{"variant": "red"}, 6]]);
    assert_eq!(
        codes(
            vec![variable(1, &keyed, Some(duplicate))],
            vec![],
            &registry
        ),
        [DiagnosticCode::LiteralType]
    );
    // Equal tags with different payloads are different keys.
    let distinct =
        json!([[{"variant": "amber", "payload": 1}, 5], [{"variant": "amber", "payload": 2}, 6]]);
    assert_eq!(
        codes(vec![variable(1, &keyed, Some(distinct))], vec![], &registry),
        []
    );
    let unhashed = format!("core/map@1<{MOOD},core/u32@1>");
    assert_eq!(
        codes(
            vec![variable(1, &unhashed, None)],
            vec![event(10, &unhashed)],
            &registry
        ),
        [
            DiagnosticCode::DeclarationType,
            DiagnosticCode::DeclarationType
        ]
    );
}

/// A minor schema update may add variants (11 §5.3): stored initializers keep decoding
/// by variant ID, and one naming a variant the current schema lacks is invalid there.
#[test]
fn stored_initializers_follow_the_current_schema_across_a_variant_addition() {
    let (old, grown) = (registry(&[]), registry(&["blue"]));
    let green = || vec![variable(1, LIGHT, Some(json!({"variant": "green"})))];
    let blue = || vec![variable(1, LIGHT, Some(json!({"variant": "blue"})))];
    assert_eq!(codes(green(), vec![], &old), []);
    assert_eq!(codes(green(), vec![], &grown), []);
    assert_eq!(codes(blue(), vec![], &grown), []);
    assert_eq!(codes(blue(), vec![], &old), [DiagnosticCode::LiteralType]);
}
