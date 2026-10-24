//! Independent source demand across epochs; SHA stream primitive alone is shared.
#[path = "support/reference.rs"]
mod reference;
#[path = "support/seeded_provider.rs"]
mod seeded_provider;
#[path = "support/data_graph.rs"]
mod support;

use orange_bundled::Operation;
use orange_model::{Value, analyze, from_json, to_json};
use orange_runtime::{DataActivation, DataOptions};
use proptest::prelude::*;
use reference::{Fault, Outcome, Reference};
use seeded_provider::*;
use serde_json::json;
use std::sync::Arc;
use support::*;

fn pull(frame: &mut DataActivation, index: usize) -> Outcome {
    let export = frame.export(index).unwrap().unwrap();
    export.outcome.cloned().map_err(|fault| Fault {
        source: fault.source,
        code: fault.fault.code,
    })
}

fn compare(
    seed: u64,
    sequence: u64,
    choices: &[(u8, usize, usize, u64)],
    reverse: bool,
    masks: [u32; 3],
) -> Vec<Outcome> {
    let (package, snapshot) = catalog();
    let mut nodes = vec![random(1), random(2), random(3), math(4, "add", 20, 22)];
    let mut sources = vec![(1, "value"), (2, "value"), (3, "value"), (4, "sum")];
    let mut edges = Vec::new();
    for (step, &(choice, left, right, fallback)) in choices.iter().enumerate() {
        let operation = Operation::ALL[usize::from(choice) % 4];
        let n = u32::try_from(step).unwrap() + 5;
        let name = operation.definition_id().rsplit('/').next().unwrap();
        nodes.push(math(n, name, fallback, fallback));
        let (a, a_port) = sources[left % sources.len()];
        let (b, b_port) = sources[right % sources.len()];
        edges.push(edge(n * 2, a, a_port, n, "a"));
        // Some literal RHS values guarantee zero divisors and checked demand
        // differences without depending on improbable random words.
        if choice < 128 {
            edges.push(edge(n * 2 + 1, b, b_port, n, "b"));
        }
        sources.push((n, operation.output_port()));
    }
    if reverse {
        nodes.reverse();
        edges.reverse();
    }
    let exports = sources.iter().map(|&(n, port)| export(n, port)).collect();
    let original = graph(&package, nodes, edges, exports);
    let encoded = to_json(&original).unwrap();
    let doc = from_json(&encoded).unwrap();
    let analysis = analyze(&doc, &snapshot);
    let mut oracle = Reference::seeded(&doc, &snapshot, &analysis, seed, sequence);
    let resolver = Counting::new(package);
    let plan = Arc::new(compile(&doc, snapshot.clone(), &resolver));
    let mut frame = DataActivation::start(
        plan.clone(),
        &doc,
        &snapshot,
        seed,
        sequence,
        DataOptions::default(),
    )
    .unwrap();
    let mut replay = DataActivation::start(
        plan,
        &doc,
        &snapshot,
        seed,
        sequence,
        DataOptions::default(),
    )
    .unwrap();
    let mut results = Vec::new();
    for (epoch, mask) in masks.into_iter().enumerate() {
        frame.begin_epoch().unwrap();
        replay.begin_epoch().unwrap();
        oracle.begin_epoch();
        let mut order: Vec<_> = (0..sources.len()).collect();
        if reverse ^ (epoch % 2 == 1) {
            order.reverse();
        }
        for index in order {
            if mask & (1 << index) == 0 {
                continue;
            }
            let expected = oracle.export(index);
            let actual = pull(&mut frame, index);
            assert_eq!(actual, expected, "epoch {epoch}, export {index}");
            assert_eq!(pull(&mut frame, index), expected, "same epoch repeat");
            assert_eq!(
                pull(&mut replay, index),
                expected,
                "fresh activation replay"
            );
            results.push(actual);
        }
    }
    assert!(oracle.executions.values().all(|count| *count <= 3));
    assert_eq!(to_json(&doc).unwrap(), encoded);
    results
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]
    #[test]
    fn generated_seeded_dags_match_source_oracle_and_replay(
        seed in any::<u64>(),
        sequence in any::<u64>(),
        choices in prop::collection::vec((any::<u8>(), 0_usize..32, 0_usize..32, any::<u64>()), 1..13),
        reverse in any::<bool>(),
        masks in any::<[u32; 3]>(),
    ) {
        compare(seed, sequence, &choices, reverse, masks);
    }

    #[test]
    fn dynamic_lazy_faults_and_nested_options_match_source_epochs(
        seed in any::<u64>(), sequence in any::<u64>(),
        selector_fault in any::<bool>(), rhs_fault in any::<bool>(),
        reverse in any::<bool>(),
    ) {
        let select = json!({"id":id(4),"definition_id":"core/select","definition_version":1,"config_version":1,"config":{},"type_pins":{"T":"core/u64@1"}});
        let and = json!({"id":id(5),"definition_id":"core/and","definition_version":1,"config_version":1,"config":{}});
        let or = json!({"id":id(6),"definition_id":"core/or","definition_version":1,"config_version":1,"config":{}});
        let coalesce = json!({"id":id(7),"definition_id":"core/coalesce","definition_version":1,"config_version":1,"config":{},"type_pins":{"T":"core/u64@1"}});
        let nested = json!({"id":id(8),"definition_id":"core/coalesce","definition_version":1,"config_version":1,"config":{},"type_pins":{"T":MAYBE},
            "inputs":{"fallback":{"type":MAYBE,"value":{"none":true}}}});
        let capture = json!({"id":id(9),"definition_id":"core/try","definition_version":1,"config_version":1,"config":{},"type_pins":{"T":MAYBE}});
        let (doc, snapshot, resolver) = fixture(
            vec![random(1), random(2), select, and, or, coalesce, nested, capture,
                fixture_node(10, BOOL, selector_fault), fixture_node(11, BOOL, rhs_fault),
                fixture_node(12, MAYBE, rhs_fault), fixture_node(13, NESTED, rhs_fault)],
            vec![edge(1,10,"value",4,"condition"),edge(2,1,"value",4,"if_true"),edge(3,2,"value",4,"if_false"),
                edge(4,10,"value",5,"a"),edge(5,11,"value",5,"b"),
                edge(6,10,"value",6,"a"),edge(7,11,"value",6,"b"),
                edge(8,12,"value",7,"value"),edge(9,4,"result",7,"fallback"),
                edge(10,13,"value",8,"value"),edge(11,8,"result",9,"value")],
            vec![export(4,"result"),export(5,"result"),export(6,"result"),export(7,"result"),export(8,"result"),export(9,"result")],
        );
        let analysis = analyze(&doc, &snapshot);
        let mut oracle = Reference::seeded(&doc, &snapshot, &analysis, seed, sequence);
        let plan = fixture_plan(&doc, &snapshot, &resolver);
        let mut frame = DataActivation::start(plan, &doc, &snapshot, seed, sequence, DataOptions::default()).unwrap();
        for _ in 0..3 {
            frame.begin_epoch().unwrap();
            oracle.begin_epoch();
            let mut order: Vec<_> = (0..doc.exports().len()).collect();
            if reverse { order.reverse(); }
            for index in order {
                let expected = oracle.export(index);
                prop_assert_eq!(pull(&mut frame,index), expected.clone());
                prop_assert_eq!(pull(&mut frame,index), expected);
            }
        }
    }
}

#[test]
fn first_fault_skips_entropy_until_actual_late_demand_and_matches_provenance() {
    let (package, snapshot) = catalog();
    let doc = graph(
        &package,
        vec![
            random(1),
            random(2),
            math(3, "divide", 1, 0),
            math(4, "add", 0, 0),
        ],
        vec![edge(1, 3, "quotient", 4, "a"), edge(2, 2, "value", 4, "b")],
        vec![export(4, "sum"), export(2, "value"), export(1, "value")],
    );
    let analysis = analyze(&doc, &snapshot);
    let mut oracle = Reference::new(&doc, &snapshot, &analysis);
    let resolver = Counting::new(package);
    let plan = Arc::new(compile(&doc, snapshot.clone(), &resolver));
    let mut frame =
        DataActivation::start(plan, &doc, &snapshot, 0, 0, DataOptions::default()).unwrap();
    for epoch in 0..3 {
        frame.begin_epoch().unwrap();
        oracle.begin_epoch();
        assert_eq!(pull(&mut frame, 0), oracle.export(0));
        assert!(!oracle.executions.contains_key(&id(2).parse().unwrap()));
        if epoch == 2 {
            assert_eq!(pull(&mut frame, 1), oracle.export(1));
            assert_eq!(pull(&mut frame, 2), oracle.export(2));
            // Node 1's first invocation stays the external-vector known answer,
            // despite two elapsed epochs with no actual demand.
            assert_eq!(pull(&mut frame, 2), Ok(Value::U64(0xfba761e9efd54e44)));
            assert_eq!(
                oracle.exports(),
                (0..3).map(|i| pull(&mut frame, i)).collect::<Vec<_>>()
            );
        }
    }
    assert!(
        compare(17, 9, &[(0, 0, 0, 0)], false, [u32::MAX; 3])
            != compare(17, 10, &[(0, 0, 0, 0)], false, [u32::MAX; 3])
    );
}
