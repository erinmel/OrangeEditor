//! Dispatch and Listen: one event channel, any number of listeners. A channel declared
//! without a payload is a pulse whose nodes have no payload port; a typed channel carries
//! its value to every Listen; an optional payload left unset sends None.
use orange_host::{
    Draft, Host, HostOptions, NodeId, StopMode, Value, VarScope, Variable, VariableId,
};
use serde_json::json;
use std::time::{Duration, Instant};

fn variable(host: &Host, draft: &mut Draft, name: &str, ty: &str) -> VariableId {
    host.declare_variable(
        draft,
        &Variable {
            id: None,
            name: name.into(),
            ty: ty.into(),
            scope: VarScope::Run,
            persistent: false,
            initial: None,
        },
    )
    .unwrap()
    .parse()
    .unwrap()
}

fn ports(host: &Host, draft: &Draft, node: NodeId) -> Vec<String> {
    host.draft_view(draft)
        .unwrap()
        .nodes
        .iter()
        .find(|shown| shown.id == node)
        .unwrap()
        .ports
        .iter()
        .filter(|port| port.id == "payload")
        .map(|port| port.ty.clone().unwrap_or_default())
        .collect()
}

/// A manual trigger wired to a Dispatch on a new channel with `payload`.
fn dispatch(host: &Host, draft: &mut Draft, payload: Option<&str>) -> (NodeId, NodeId, String) {
    let trigger = host.add_node(draft, "core/on-manual", [0.0, 0.0]).unwrap();
    let emit = host.add_node(draft, "core/emit", [200.0, 0.0]).unwrap();
    let event = host
        .declare_run_event_for(
            draft,
            emit,
            "Door bell",
            payload.map(|ty| ty.parse().unwrap()),
        )
        .unwrap()
        .to_string();
    host.connect(draft, (trigger, "then"), (emit, "in"))
        .unwrap();
    (trigger, emit, event)
}

/// A Listen on `event` that sets a new variable, from its payload or from `literal`.
fn listener(
    host: &Host,
    draft: &mut Draft,
    event: &str,
    name: &str,
    (ty, literal): (&str, Option<&str>),
) -> (NodeId, VariableId) {
    let listen = host.add_node(draft, "core/on-event", [400.0, 0.0]).unwrap();
    host.set_config(draft, listen, &json!({"event": event}))
        .unwrap();
    let target = variable(host, draft, name, ty);
    let set = host
        .add_node(draft, "core/set-variable", [600.0, 0.0])
        .unwrap();
    host.set_config(draft, set, &json!({"variable": target.to_string()}))
        .unwrap();
    match literal {
        Some(value) => {
            host.set_input(draft, (set, "value"), Some(value)).unwrap();
        }
        None => {
            host.connect(draft, (listen, "payload"), (set, "value"))
                .unwrap();
        }
    }
    host.connect(draft, (listen, "then"), (set, "in")).unwrap();
    (listen, target)
}

fn run_until(host: &mut Host, draft: &Draft, trigger: NodeId, expected: &[(VariableId, Value)]) {
    let flow = host.open(&draft.text().unwrap()).unwrap();
    host.trigger(flow, trigger, None).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    for (target, value) in expected {
        let target = *target;
        loop {
            let read = host
                .inspect(flow, move |run| run.variable(target).cloned())
                .unwrap();
            if read.as_ref() == Some(value) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "{target}: {read:?}, not {value:?}"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    host.stop(flow, StopMode::Graceful).unwrap();
    host.close(flow).unwrap();
}

#[test]
fn a_pulse_channel_has_no_payload_port_and_reaches_every_listen() {
    let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
    let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
    let (trigger, emit, event) = dispatch(&host, &mut draft, None);
    let (first, a) = listener(
        &host,
        &mut draft,
        &event,
        "a",
        ("core/bool@1", Some("true")),
    );
    let (_, b) = listener(
        &host,
        &mut draft,
        &event,
        "b",
        ("core/bool@1", Some("true")),
    );
    assert!(ports(&host, &draft, emit).is_empty());
    assert!(ports(&host, &draft, first).is_empty());
    assert!(host.problems(&mut draft).unwrap().is_empty());
    // Saved without a payload field, and read back the same.
    let text = draft.text().unwrap();
    let saved: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert!(
        saved["events"][0].get("payload").is_none(),
        "{}",
        saved["events"]
    );
    let reopened = host.draft(&text).unwrap();
    assert!(ports(&host, &reopened, emit).is_empty());
    run_until(
        &mut host,
        &draft,
        trigger,
        &[(a, Value::Bool(true)), (b, Value::Bool(true))],
    );
}

#[test]
fn a_typed_channel_carries_its_value_to_every_listen() {
    let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
    let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
    let (trigger, emit, event) = dispatch(&host, &mut draft, Some("core/i64@1"));
    host.set_input(&mut draft, (emit, "payload"), Some("42"))
        .unwrap();
    let (_, a) = listener(&host, &mut draft, &event, "a", ("core/i64@1", None));
    let (_, b) = listener(&host, &mut draft, &event, "b", ("core/i64@1", None));
    assert_eq!(ports(&host, &draft, emit), ["core/i64@1"]);
    assert!(host.problems(&mut draft).unwrap().is_empty());
    run_until(
        &mut host,
        &draft,
        trigger,
        &[(a, Value::I64(42)), (b, Value::I64(42))],
    );
}

#[test]
fn a_cleared_typed_payload_is_required_and_a_cleared_optional_one_sends_none() {
    let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
    let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
    let (_, emit, _) = dispatch(&host, &mut draft, Some("core/i64@1"));
    host.set_input(&mut draft, (emit, "payload"), Some("42"))
        .unwrap();
    host.set_input(&mut draft, (emit, "payload"), None).unwrap();
    let problems = host.problems(&mut draft).unwrap();
    assert!(
        problems
            .iter()
            .any(|problem| problem.code.as_str() == "E-INPUT-REQUIRED"),
        "{problems:?}"
    );
    let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
    let ty = "core/option@1<core/i64@1>";
    let (trigger, emit, event) = dispatch(&host, &mut draft, Some(ty));
    let (_, a) = listener(&host, &mut draft, &event, "a", (ty, None));
    assert_eq!(ports(&host, &draft, emit), [ty]);
    assert!(host.problems(&mut draft).unwrap().is_empty());
    run_until(&mut host, &draft, trigger, &[(a, Value::None)]);
}

#[test]
fn an_older_pulse_channel_keeps_its_optional_payload_ports() {
    let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
    let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
    // Channels made before event-only ones carried an optional bool set to None.
    let ty = "core/option@1<core/bool@1>";
    let (trigger, emit, event) = dispatch(&host, &mut draft, Some(ty));
    let none = json!({"type": ty, "value": {"none": true}}).to_string();
    host.set_input(&mut draft, (emit, "payload"), Some(&none))
        .unwrap();
    let (listen, a) = listener(
        &host,
        &mut draft,
        &event,
        "a",
        ("core/bool@1", Some("true")),
    );
    let reopened = host.draft(&draft.text().unwrap()).unwrap();
    assert_eq!(ports(&host, &reopened, emit), [ty]);
    assert_eq!(ports(&host, &reopened, listen), [ty]);
    assert!(host.problems(&mut draft).unwrap().is_empty());
    run_until(&mut host, &reopened, trigger, &[(a, Value::Bool(true))]);
}

#[test]
fn create_emitter_on_a_fresh_listen_makes_one_channel_in_one_undo_step() {
    let (host, _events) = Host::new(HostOptions::default()).unwrap();
    let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
    let listen = host
        .add_node(&mut draft, "core/on-event", [400.0, 0.0])
        .unwrap();
    let emit = host
        .add_signal_sender(&mut draft, listen, [0.0, 0.0])
        .unwrap();
    let document = draft.document();
    assert_eq!(document.events().len(), 1);
    let event = document.events().values().next().unwrap();
    assert_eq!(event.payload, None);
    for node in [listen, emit] {
        assert_eq!(
            document.root().nodes[&node].config["event"],
            json!(event.id.to_string())
        );
    }
    assert!(ports(&host, &draft, listen).is_empty());
    // One undo takes back the channel, the binding and the new Dispatch together.
    assert!(draft.undo());
    let document = draft.document();
    assert!(document.events().is_empty());
    assert_eq!(document.root().nodes.len(), 1);
    assert!(document.root().nodes[&listen].config.get("event").is_none());
    assert!(draft.redo());
    assert_eq!(draft.document().root().nodes.len(), 2);
}

#[test]
fn deleting_a_dispatch_keeps_its_listens_and_a_renamed_channel_undoes_cleanly() {
    let (host, _events) = Host::new(HostOptions::default()).unwrap();
    let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
    let (_, emit, event) = dispatch(&host, &mut draft, None);
    let first = host
        .add_signal_receiver(&mut draft, emit, [400.0, 0.0])
        .unwrap();
    let second = host
        .add_signal_receiver(&mut draft, emit, [400.0, 200.0])
        .unwrap();
    let id = event.parse().unwrap();
    assert!(host.rename_event(&mut draft, id, " Front door ").unwrap());
    assert!(host.rename_event(&mut draft, id, "").is_err());
    host.remove_nodes(&mut draft, &[emit]).unwrap();
    let name = |draft: &Draft| draft.document().events()[&id].name.clone();
    let bound = |draft: &Draft, node: NodeId| {
        draft.document().root().nodes[&node].config["event"] == json!(event)
    };
    // The subscribers stay, bound to the channel by its last name.
    assert_eq!(name(&draft), "Front door");
    assert!(bound(&draft, first) && bound(&draft, second));
    assert!(!draft.document().root().nodes.contains_key(&emit));
    // Undo brings the Dispatch back, then the old name; redo replays both.
    assert!(draft.undo());
    assert!(draft.document().root().nodes.contains_key(&emit));
    assert!(draft.undo());
    assert_eq!(name(&draft), "Door bell");
    assert!(draft.redo());
    assert_eq!(name(&draft), "Front door");
    assert!(draft.redo());
    assert!(!draft.document().root().nodes.contains_key(&emit));
    assert!(bound(&draft, first) && bound(&draft, second));
}
