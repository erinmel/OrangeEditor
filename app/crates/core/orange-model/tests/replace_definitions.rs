//! Nodes of replaced definitions rebuilt on open (04 "Definition evolution", 12 §6.4).

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::binding::{NewNode, new_node_edit};
use crate::catalog::{
    Determinism, NodeDefinition, Ports, ProviderDescriptor, ProviderKey, RegistryBuilder,
    RegistrySnapshot,
};
use crate::document::{DefinitionRef, Edge, NodeKind, PortRef};
use crate::id::{EdgeId, NodeId};
use crate::replace::{Replaced, Replacement, ReplacementError, Replacements, replace_definitions};
use crate::validation::{HostOrigin, Severity};
use crate::{DocCommand, GraphDocument, validate};
use serde_json::json;

const METER: &str = "10000000-0000-4000-8000-000000000001";
const SINK: &str = "10000000-0000-4000-8000-000000000002";
const WIRE: &str = "30000000-0000-4000-8000-000000000001";
const DIGEST: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const F64: &str = "core/f64@1";

fn dref(id: &str, version: u32) -> DefinitionRef {
    DefinitionRef {
        id: id.parse().unwrap(),
        version,
    }
}

fn id(text: &str) -> NodeId {
    text.parse().unwrap()
}

/// A pure definition with one `f64` input and one `f64` output.
fn definition(reference: DefinitionRef, input: &str, output: &str) -> NodeDefinition {
    NodeDefinition {
        definition: reference,
        config_version: 1,
        kind: NodeKind::Pure,
        volatility: None,
        determinism: Determinism::Deterministic,
        suspends: None,
        type_params: Vec::new(),
        ports: Ports::Declared(vec![
            serde_json::from_value(
                json!({"port_id": input, "direction": "input", "kind": "data", "type": F64}),
            )
            .unwrap(),
            serde_json::from_value(json!({"port_id": output, "direction": "output",
                "kind": "data", "type": F64, "source": "computed"}))
            .unwrap(),
        ]),
        references: Vec::new(),
        specializations: None,
        config_schema: None,
    }
}

fn key(version: &str) -> ProviderKey {
    package_key("example.audio", version)
}

fn package_key(package: &str, version: &str) -> ProviderKey {
    ProviderKey {
        package: package.into(),
        version: version.into(),
        content_digest: DIGEST.into(),
        id: "native".into(),
    }
}

/// `example/sink@1` from a package that does not change, beside the given meter
/// definitions, provided by `example.audio` version `version` and bound to it by default.
fn registry(version: &str, meters: Vec<NodeDefinition>) -> Arc<RegistrySnapshot> {
    let sink = definition(dref("example/sink", 1), "a", "out");
    let base = package_key("example.base", "1.0.0");
    let implements: BTreeSet<(DefinitionRef, u32)> = meters
        .iter()
        .map(|meter| (meter.definition.clone(), 1))
        .collect();
    let mut builder = RegistryBuilder::core(
        HostOrigin::new("0.1.0", format!("sha256:{}", "c".repeat(64))).unwrap(),
    )
    .definition(sink.clone())
    .provider(ProviderDescriptor {
        key: base.clone(),
        implements: [(sink.definition.clone(), 1)].into(),
        specializations: Default::default(),
    })
    .provider(ProviderDescriptor {
        key: key(version),
        implements,
        specializations: Default::default(),
    })
    .new_binding_default(sink.definition, base);
    for meter in meters {
        let reference = meter.definition.clone();
        builder = builder
            .definition(meter)
            .new_binding_default(reference, key(version));
    }
    builder.build().unwrap()
}

/// What package version 1 saved: a `meter@1` with a literal on `level` and config
/// `window`, wired from `reading` to a sink.
fn saved_with_meter_1() -> GraphDocument {
    let old = registry(
        "1.0.0",
        vec![definition(dref("example/meter", 1), "level", "reading")],
    );
    let mut document = GraphDocument::new("20000000-0000-4000-8000-000000000001".parse().unwrap());
    let mut meter = NewNode::new(id(METER), dref("example/meter", 1));
    meter.inputs.insert(
        "level".parse().unwrap(),
        serde_json::from_value(json!({"type": F64, "bits": format!("{:016x}", 0.5f64.to_bits())}))
            .unwrap(),
    );
    meter.config.insert("window".to_owned(), json!(5));
    meter.config.insert("kept".to_owned(), json!(true));
    let mut commands = new_node_edit(&document, &old, meter).unwrap().commands;
    commands.extend(
        new_node_edit(
            &document,
            &old,
            NewNode::new(id(SINK), dref("example/sink", 1)),
        )
        .unwrap()
        .commands,
    );
    commands.push(DocCommand::Connect {
        edge: Edge {
            id: WIRE.parse().unwrap(),
            from: PortRef {
                node: id(METER),
                port: "reading".parse().unwrap(),
            },
            to: PortRef {
                node: id(SINK),
                port: "a".parse().unwrap(),
            },
            extensions: Default::default(),
        },
    });
    let _ = document.apply(commands).unwrap();
    document
}

fn step(from: u32, to: u32, ports: &[(&str, &str)], config: &[(&str, &str)]) -> Replacement {
    Replacement {
        from: dref("example/meter", from),
        to: dref("example/meter", to),
        ports: ports
            .iter()
            .map(|(old, new)| (old.parse().unwrap(), new.parse().unwrap()))
            .collect(),
        config: config
            .iter()
            .map(|(old, new)| ((*old).to_owned(), (*new).to_owned()))
            .collect(),
    }
}

fn table(steps: impl IntoIterator<Item = Replacement>) -> Replacements {
    let mut table = Replacements::new();
    for step in steps {
        table.add(step).unwrap();
    }
    table
}

fn errors(document: &GraphDocument, registry: &RegistrySnapshot) -> Vec<String> {
    validate(document, registry)
        .diagnostics
        .into_iter()
        .filter(|item| item.severity == Severity::Error)
        .map(|item| format!("{:?}: {}", item.code, item.message))
        .collect()
}

#[test]
fn a_replaced_node_keeps_its_identity_values_and_wires_under_the_new_names() {
    let mut document = saved_with_meter_1();
    let current = registry(
        "2.0.0",
        vec![definition(dref("example/meter", 2), "signal", "value")],
    );
    assert!(!errors(&document, &current).is_empty(), "meter@1 is gone");
    let table = table([step(
        1,
        2,
        &[("level", "signal"), ("reading", "value")],
        &[("window", "window_ms")],
    )]);
    let revision = document.revision();

    let replaced = replace_definitions(&mut document, &current, &table).unwrap();
    assert_eq!(
        replaced,
        [Replaced {
            node: id(METER),
            from: dref("example/meter", 1),
            to: dref("example/meter", 2),
        }]
    );
    assert_eq!(document.revision(), revision + 1, "one transaction");
    let meter = document.node(id(METER)).unwrap();
    assert_eq!(meter.definition, dref("example/meter", 2));
    assert_eq!(
        meter
            .inputs
            .keys()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["signal"]
    );
    assert_eq!(meter.config["window_ms"], json!(5));
    assert_eq!(meter.config["kept"], json!(true));
    assert!(!meter.config.contains_key("window"));
    assert_eq!(
        ProviderKey::of(&document, meter.provider.as_ref().unwrap()),
        Some(key("2.0.0"))
    );
    let wire = &document.root().edges[&WIRE.parse::<EdgeId>().unwrap()];
    assert_eq!(wire.from.port.to_string(), "value");
    assert_eq!(
        (wire.to.node, wire.to.port.to_string()),
        (id(SINK), "a".into())
    );
    assert_eq!(errors(&document, &current), Vec::<String>::new());

    // Already replaced: nothing left to do, and the document is not touched.
    assert_eq!(
        replace_definitions(&mut document, &current, &table).unwrap(),
        []
    );
    assert_eq!(document.revision(), revision + 1);
}

#[test]
fn a_registered_definition_is_never_moved() {
    let mut document = saved_with_meter_1();
    let both = registry(
        "2.0.0",
        vec![
            definition(dref("example/meter", 1), "level", "reading"),
            definition(dref("example/meter", 2), "signal", "value"),
        ],
    );
    let table = table([step(1, 2, &[("level", "signal")], &[])]);
    let before = document.clone();
    assert_eq!(
        replace_definitions(&mut document, &both, &table).unwrap(),
        []
    );
    assert_eq!(document, before);
}

#[test]
fn a_chain_of_replacements_composes_its_renames() {
    let mut document = saved_with_meter_1();
    let current = registry(
        "3.0.0",
        vec![definition(dref("example/meter", 3), "input", "value")],
    );
    let table = table([
        step(
            1,
            2,
            &[("level", "signal"), ("reading", "value")],
            &[("window", "span")],
        ),
        step(2, 3, &[("signal", "input")], &[("span", "window_ms")]),
    ]);
    let replaced = replace_definitions(&mut document, &current, &table).unwrap();
    assert_eq!(replaced[0].to, dref("example/meter", 3));
    let meter = document.node(id(METER)).unwrap();
    assert!(meter.inputs.contains_key(&"input".parse().unwrap()));
    assert_eq!(meter.config["window_ms"], json!(5));
    assert_eq!(errors(&document, &current), Vec::<String>::new());
}

#[test]
fn a_loop_or_a_missing_target_leaves_the_node_for_validation_to_report() {
    let current = registry(
        "2.0.0",
        vec![definition(dref("example/meter", 2), "signal", "value")],
    );
    for table in [
        // meter@1 → meter@7, which nothing registers.
        table([step(1, 7, &[], &[])]),
        // meter@1 → meter@5 → meter@1.
        table([step(1, 5, &[], &[]), step(5, 1, &[], &[])]),
    ] {
        let mut document = saved_with_meter_1();
        let before = document.clone();
        assert_eq!(
            replace_definitions(&mut document, &current, &table).unwrap(),
            []
        );
        assert_eq!(document, before);
    }
}

#[test]
fn a_table_refuses_self_and_conflicting_replacements() {
    let mut table = Replacements::new();
    assert_eq!(
        table.add(step(1, 1, &[], &[])),
        Err(ReplacementError::SelfReplacement(dref("example/meter", 1)))
    );
    table.add(step(1, 2, &[("level", "signal")], &[])).unwrap();
    // The same declaration from a second file is accepted once.
    table.add(step(1, 2, &[("level", "signal")], &[])).unwrap();
    assert_eq!(
        table.add(step(1, 3, &[], &[])),
        Err(ReplacementError::Conflict(dref("example/meter", 1)))
    );
}
