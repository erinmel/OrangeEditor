//! Host-triggered `core/on-manual` entries through the shared bounded handler admission (C04b).
use super::*;
use orange_model::id::NodeId;
use orange_runtime::{EventOrigin, EventTarget, IngressError};

fn manual(n: u32, payload: Option<&str>, capacity: u32, overflow: &str) -> Json {
    let mut node = core(n, "on-manual");
    if let Some(payload) = payload {
        node["config"] = json!({"payload": payload});
    }
    node["activation"] = activation(0, capacity, overflow);
    node
}

fn node_id(n: u32) -> NodeId {
    id(n).parse().unwrap()
}

fn manual_graph(payload: Option<&str>, capacity: u32, overflow: &str) -> (GraphDocument, Run) {
    let mut set = variable_node(2, "set-variable", 100);
    let mut edges = vec![edge(1, 1, "then", 2, "in")];
    if payload.is_some() {
        edges.push(edge(2, 1, "payload", 2, "value"));
    } else {
        set["inputs"] = json!({"value": {"type": "core/u64@1", "value": "9"}});
    }
    let (doc, plan) = compile(
        vec![manual(1, payload, capacity, overflow), set],
        edges,
        vec![variable(100, "run", None)],
    );
    let run = run(&doc, plan);
    (doc, run)
}

fn value(run: &Run) -> Option<&Value> {
    run.variable(id(100).parse().unwrap())
}

#[test]
fn trigger_admits_typed_payload_and_root_starts_on_a_later_pump() {
    let (_doc, mut run) = manual_graph(Some("core/u64@1"), 4, "fault");
    pump_until_idle(&mut run);

    let first = run
        .trigger(node_id(1), Some(&Value::U64(7)), Some(55))
        .unwrap();
    assert_eq!(first.target, EventTarget::Manual(node_id(1)));
    assert_eq!(first.origin, EventOrigin::Manual);
    assert_eq!(
        (
            first.source_seq,
            first.cause,
            first.cause_depth,
            first.wall_ns
        ),
        (1, None, 0, Some(55))
    );
    assert_eq!(first.monotonic_ns, run.now());
    assert_eq!(run.last_event(), Some(first));
    // Admission is synchronous bookkeeping only: the handler root has not run.
    assert_eq!(value(&run), None);
    assert_eq!(run.event_stats().pending, 1);

    pump_until_idle(&mut run);
    assert_eq!(value(&run), Some(&Value::U64(7)));

    let second = run.trigger(node_id(1), Some(&Value::U64(8)), None).unwrap();
    assert_eq!((second.source_seq, second.wall_ns), (2, None));
    assert!(second.ingestion_seq > first.ingestion_seq);
    pump_until_idle(&mut run);
    assert_eq!(value(&run), Some(&Value::U64(8)));
    let stats = run.event_stats();
    assert_eq!((stats.accepted, stats.delivered, stats.pending), (2, 2, 0));
}

#[test]
fn refused_triggers_change_no_sequence_or_queue() {
    let (_doc, mut run) = manual_graph(Some("core/u64@1"), 4, "fault");
    pump_until_idle(&mut run);
    let refusals = [
        (node_id(2), Some(Value::U64(1)), IngressError::UnknownEntry),
        (node_id(99), Some(Value::U64(1)), IngressError::UnknownEntry),
        (node_id(1), None, IngressError::InvalidPayload),
        (
            node_id(1),
            Some(Value::I64(1)),
            IngressError::InvalidPayload,
        ),
    ];
    for (node, payload, expected) in refusals {
        assert_eq!(run.trigger(node, payload.as_ref(), None), Err(expected));
        assert_eq!(run.event_stats(), Default::default());
        assert_eq!(run.last_event(), None);
    }
    assert_eq!(
        IngressError::UnknownEntry.message_key(),
        "core/ingress-unknown-entry"
    );
    let accepted = run.trigger(node_id(1), Some(&Value::U64(3)), None).unwrap();
    assert_eq!((accepted.ingestion_seq, accepted.source_seq), (1, 1));

    run.stop().unwrap();
    assert_eq!(
        run.trigger(node_id(1), Some(&Value::U64(4)), None),
        Err(IngressError::NotRunning)
    );
}

#[test]
fn payloadless_entry_requires_no_payload() {
    let (_doc, mut run) = manual_graph(None, 4, "fault");
    pump_until_idle(&mut run);
    assert_eq!(
        run.trigger(node_id(1), Some(&Value::Unit), None),
        Err(IngressError::InvalidPayload)
    );
    run.trigger(node_id(1), None, None).unwrap();
    pump_until_idle(&mut run);
    assert_eq!(value(&run), Some(&Value::U64(9)));
}

#[test]
fn trigger_follows_the_entry_overflow_policy() {
    let (_doc, mut run) = manual_graph(Some("core/u64@1"), 1, "drop-newest");
    pump_until_idle(&mut run);
    // No root starts without a pump, so the first fills capacity and later ones drop.
    for n in 1..=3 {
        run.trigger(node_id(1), Some(&Value::U64(n)), None).unwrap();
    }
    let stats = run.event_stats();
    assert_eq!((stats.accepted, stats.dropped, stats.overflows), (1, 2, 2));
    pump_until_idle(&mut run);
    assert_eq!(value(&run), Some(&Value::U64(1)));

    let (_doc, mut run) = manual_graph(Some("core/u64@1"), 1, "fault");
    pump_until_idle(&mut run);
    run.trigger(node_id(1), Some(&Value::U64(1)), None).unwrap();
    assert_eq!(
        run.trigger(node_id(1), Some(&Value::U64(2)), None),
        Err(IngressError::DeliveryLimit)
    );
    // A refusal leaves the trigger sequence where the last accepted call put it.
    pump_until_idle(&mut run);
    let next = run.trigger(node_id(1), Some(&Value::U64(4)), None).unwrap();
    assert_eq!(next.source_seq, 2);
}

#[test]
fn manual_payload_config_is_validated() {
    let (package, registry) = catalog();
    for config in [
        json!({"payload": "core/nope@1"}),
        json!({"payload": "$T"}),
        json!({"payload": 3}),
        json!({"payload": "core/u64@1", "extra": true}),
    ] {
        let mut node = manual(1, None, 4, "fault");
        node["config"] = config.clone();
        let document = graph(&package, vec![node], vec![], vec![]);
        let analysis = analyze(&document, &registry);
        let codes: Vec<_> = analysis
            .report()
            .diagnostics
            .iter()
            .map(|d| d.code.as_str())
            .collect();
        assert!(codes.contains(&"E-MANUAL-PAYLOAD"), "{config}: {codes:?}");
    }
}
