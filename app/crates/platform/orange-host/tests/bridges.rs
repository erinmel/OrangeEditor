//! Bridge Out and Bridge In: a value read across the canvas without a wire or a flow.
//! A Bridge In is bound to its Bridge Out by an ordinary data edge, so it is pulled like
//! any pure value and typed through it; one Bridge Out serves any number of Bridge Ins.
use orange_host::{Host, HostOptions, StopMode, Value, VarScope, Variable, VariableId};
use serde_json::json;
use std::time::{Duration, Instant};

fn variable(host: &Host, draft: &mut orange_host::Draft, name: &str) -> String {
    host.declare_variable(
        draft,
        &Variable {
            id: None,
            name: name.into(),
            ty: "core/f64@1".into(),
            scope: VarScope::Run,
            persistent: false,
            initial: None,
        },
    )
    .unwrap()
}

#[test]
fn one_bridge_out_feeds_every_bridge_in_when_they_are_pulled() {
    let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
    let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
    let number = host
        .add_node(&mut draft, "core/number", [0.0, 0.0])
        .unwrap();
    host.set_input(&mut draft, (number, "value"), Some("7.5"))
        .unwrap();
    let out = host
        .add_node(&mut draft, "core/bridge-out", [200.0, 0.0])
        .unwrap();
    host.set_config(&mut draft, out, &json!({"channel": "Level"}))
        .unwrap();
    host.connect(&mut draft, (number, "result"), (out, "value"))
        .unwrap();
    let trigger = host
        .add_node(&mut draft, "core/on-manual", [0.0, 300.0])
        .unwrap();
    let mut previous = (trigger, "then");
    let mut targets = Vec::new();
    for name in ["first", "second"] {
        let reading = host
            .add_node(&mut draft, "core/bridge-in", [400.0, 300.0])
            .unwrap();
        host.set_config(&mut draft, reading, &json!({"channel": "Level"}))
            .unwrap();
        host.connect(&mut draft, (out, "result"), (reading, "value"))
            .unwrap();
        let target = variable(&host, &mut draft, name);
        let set = host
            .add_node(&mut draft, "core/set-variable", [600.0, 300.0])
            .unwrap();
        host.set_config(&mut draft, set, &json!({"variable": target}))
            .unwrap();
        host.connect(&mut draft, (reading, "result"), (set, "value"))
            .unwrap();
        host.connect(&mut draft, previous, (set, "in")).unwrap();
        previous = (set, "then");
        targets.push(target.parse::<VariableId>().unwrap());
    }
    assert!(host.problems(&mut draft).unwrap().is_empty());
    let flow = host.open(&draft.text().unwrap()).unwrap();
    host.trigger(flow, trigger, None).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    for target in targets {
        loop {
            let read = host
                .inspect(flow, move |run| run.variable(target).cloned())
                .unwrap();
            if read == Some(Value::F64(7.5)) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the bridge was not read: {read:?}"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    host.stop(flow, StopMode::Graceful).unwrap();
    host.close(flow).unwrap();
}

#[test]
fn a_bridge_in_whose_bridge_out_is_gone_keeps_its_channel_and_reports_it_unbound() {
    let (host, _events) = Host::new(HostOptions::default()).unwrap();
    let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
    let out = host
        .add_node(&mut draft, "core/bridge-out", [0.0, 0.0])
        .unwrap();
    host.set_config(&mut draft, out, &json!({"channel": "Level"}))
        .unwrap();
    let reading = host
        .add_node(&mut draft, "core/bridge-in", [200.0, 0.0])
        .unwrap();
    host.set_config(&mut draft, reading, &json!({"channel": "Level"}))
        .unwrap();
    host.connect(&mut draft, (out, "result"), (reading, "value"))
        .unwrap();
    // Deleting the emitter keeps its subscriber, with the channel it was bound to.
    let _ = host.remove_nodes(&mut draft, &[out]).unwrap();
    let graph = host.draft_view(&draft).unwrap();
    let kept = graph.nodes.iter().find(|node| node.id == reading).unwrap();
    assert_eq!(kept.settings["channel"], "Level");
    assert!(!kept.ports.iter().any(|port| port.input && port.connected));
    assert!(
        !host.problems(&mut draft).unwrap().is_empty(),
        "an unbound bridge is reported"
    );
}
