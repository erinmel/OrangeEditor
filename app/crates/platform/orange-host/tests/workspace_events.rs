//! Events one flow dispatches to the other flows of its host (docs/03, C12c).
use orange_host::{FlowId, Host, HostError, HostOptions, RunState, StopMode, Value};
use orange_model::id::VariableId;
use orange_model::{GraphDocument, from_json, to_json};
use orange_testkit::{core, edge, id, node, until};
use serde_json::{Value as Json, json};
use std::time::Duration;

const I64: &str = "core/i64@1";
/// The workspace event every graph here dispatches and listens to.
const SCORE: u32 = 800;
/// The run variable a graph keeps the last score it heard in.
const HEARD: u32 = 100;

/// A graph that dispatches the payload of its manual entry (node 1) as `score`, and
/// keeps every `score` it hears from another flow in `heard`.
fn scoring_graph(graph: u32) -> String {
    let document = GraphDocument::new(
        format!("20000000-0000-4000-8000-{graph:012x}")
            .parse()
            .unwrap(),
    );
    let mut saved: Json = serde_json::from_str(&to_json(&document).unwrap()).unwrap();
    let mut send = core(1, "on-manual");
    send["config"] = json!({"payload": I64});
    let mut dispatch = core(2, "emit");
    dispatch["config"] = json!({"event": id(SCORE)});
    let mut listen = core(3, "on-event");
    listen["config"] = json!({"event": id(SCORE)});
    let mut keep = core(4, "set-variable");
    keep["config"] = json!({"variable": id(HEARD)});
    saved["nodes"] = json!([send, dispatch, listen, keep]);
    saved["edges"] = json!([
        edge(1, 1, "then", 2, "in"),
        edge(2, 1, "payload", 2, "payload"),
        edge(3, 3, "then", 4, "in"),
        edge(4, 3, "payload", 4, "value"),
    ]);
    saved["variables"] = json!([{"id": id(HEARD), "name": "heard", "type": I64, "scope": "run"}]);
    saved["events"] =
        json!([{"id": id(SCORE), "name": "score", "payload": I64, "scope": "workspace"}]);
    to_json(&from_json(&saved.to_string()).unwrap()).unwrap()
}

fn heard(host: &Host, flow: FlowId) -> Option<Value> {
    let variable: VariableId = id(HEARD).parse().unwrap();
    host.inspect(flow, move |run| run.variable(variable).cloned())
        .unwrap()
}

#[test]
fn a_dispatched_workspace_event_reaches_every_other_flow_and_not_its_sender() {
    let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
    // Without a declaration a document that uses workspace events does not open.
    assert!(matches!(
        host.open(&scoring_graph(1)),
        Err(HostError::Workspace(_))
    ));
    host.share_variables(&scoring_graph(9)).unwrap();
    let flows = [1, 2, 3].map(|graph| host.open(&scoring_graph(graph)).unwrap());

    host.trigger(flows[0], node(1), Some(Value::I64(5)))
        .unwrap();
    for flow in &flows[1..] {
        until("the listeners hear 5", || {
            heard(&host, *flow) == Some(Value::I64(5))
        });
    }
    // A broadcast reaches the other flows, never its sender.
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(heard(&host, flows[0]), None);

    host.trigger(flows[2], node(1), Some(Value::I64(9)))
        .unwrap();
    for flow in &flows[..2] {
        until("the listeners hear 9", || {
            heard(&host, *flow) == Some(Value::I64(9))
        });
    }
    assert_eq!(heard(&host, flows[2]), Some(Value::I64(5)));
}

#[test]
fn stopping_one_flow_leaves_the_others_exchanging_events() {
    let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
    host.share_variables(&scoring_graph(9)).unwrap();
    let flows = [1, 2, 3].map(|graph| host.open(&scoring_graph(graph)).unwrap());
    host.stop(flows[1], StopMode::Immediate).unwrap();
    until("the stopped flow ends", || {
        host.state(flows[1])
            .is_none_or(|state| matches!(state, RunState::Stopped | RunState::Disposed))
    });

    host.trigger(flows[0], node(1), Some(Value::I64(3)))
        .unwrap();
    until("the running listener hears 3", || {
        heard(&host, flows[2]) == Some(Value::I64(3))
    });
    host.trigger(flows[2], node(1), Some(Value::I64(4)))
        .unwrap();
    until("the sender hears the reply", || {
        heard(&host, flows[0]) == Some(Value::I64(4))
    });
}

#[test]
fn a_document_whose_workspace_events_do_not_match_the_declaration_does_not_open() {
    let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
    host.share_variables(&scoring_graph(9)).unwrap();
    let mut other: Json = serde_json::from_str(&scoring_graph(2)).unwrap();
    other["events"][0]["payload"] = json!("core/string@1");
    other["variables"][0]["type"] = json!("core/string@1");
    other["nodes"][0]["config"]["payload"] = json!("core/string@1");
    let text = to_json(&from_json(&other.to_string()).unwrap()).unwrap();
    let opened = host.open(&text);
    assert!(matches!(opened, Err(HostError::Workspace(_))), "{opened:?}");
}

#[test]
fn an_edited_flow_keeps_hearing_and_sending_workspace_events() {
    let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
    host.share_variables(&scoring_graph(9)).unwrap();
    let [first, second] = [1, 2].map(|graph| host.open(&scoring_graph(graph)).unwrap());
    // A change to what the listener queues replaces the flow's run.
    let mut edited: Json = serde_json::from_str(&scoring_graph(2)).unwrap();
    edited["nodes"][2]["activation"]["capacity"] = json!(4);
    let edited = to_json(&from_json(&edited.to_string()).unwrap()).unwrap();
    assert!(host.update(second, &edited).unwrap().changed);

    host.trigger(first, node(1), Some(Value::I64(6))).unwrap();
    until("the edited flow hears 6", || {
        heard(&host, second) == Some(Value::I64(6))
    });
    host.trigger(second, node(1), Some(Value::I64(8))).unwrap();
    until("the first flow hears 8", || {
        heard(&host, first) == Some(Value::I64(8))
    });
}
