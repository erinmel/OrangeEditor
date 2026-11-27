use orange_host::{
    Call, Host, HostOptions, InputState, KernelFault, Literal, LiteralEditor, ServiceNode,
    ServicePackage, Value,
};
use serde_json::json;

struct Sink;

impl ServiceNode for Sink {
    fn run(&self, _: &Call<'_>) -> Result<Vec<Value>, KernelFault> {
        Ok(Vec::new())
    }
}

#[test]
fn port_caption_help_and_implicit_none_project_without_rewriting_the_graph() {
    let definition = orange_sdk::Definition::node("example.authoring", "sink", "Sink")
        .input("sample", "core/option@1<core/error@1>")
        .build();
    let mut definition: serde_json::Value = serde_json::from_str(&definition).unwrap();
    definition["ports"][0]["optional"] = json!(true);
    definition["ports"][0]["label"] =
        json!({"key":"example.authoring/sample-caption", "fallback":"Sample"});
    definition["ports"][0]["description"] =
        json!({"key":"example.authoring/sample-help", "fallback":"A sample"});
    let package = ServicePackage::new("example.authoring", "1.0.0")
        .node(&definition.to_string(), Sink)
        .unwrap();
    let (mut host, _) = Host::with_services(HostOptions::default(), vec![package]).unwrap();
    host.add_catalog(
        "es",
        "example-authoring-sample-caption = Muestra\nexample-authoring-sample-help = Una muestra\n",
    )
    .unwrap();
    let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
    let node = host
        .add_node(&mut draft, "example.authoring/sink", [0.0, 0.0])
        .unwrap();
    let text = draft.text().unwrap();
    for (language, caption, help) in [
        ("en", "Sample", "A sample"),
        ("es", "Muestra", "Una muestra"),
    ] {
        let view = host.draft_authoring_view_in(&draft, language).unwrap();
        let port = view.graph.nodes[0]
            .ports
            .iter()
            .find(|port| port.id == "sample")
            .unwrap();
        let detail = &view.port_details[&node][0];
        assert_eq!(port.label.as_deref(), Some(caption));
        assert_eq!(detail.description.as_deref(), Some(help));
        assert_eq!(port.input_state(Some(detail)), InputState::ImplicitNone);
        assert_eq!(
            port.literal_editor(Some(detail)),
            LiteralEditor::Option { some: false }
        );
    }
    assert_eq!(draft.text().unwrap(), text);
}

#[test]
fn input_states_preserve_shadowed_literals() {
    let (host, _) = Host::new(HostOptions::default()).unwrap();
    let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
    let number = host
        .add_node(&mut draft, "core/whole-number", [0.0, 0.0])
        .unwrap();
    let wait = host
        .add_node(&mut draft, "core/wait", [300.0, 0.0])
        .unwrap();
    let state = |draft: &orange_host::Draft, id, name: &str| {
        let view = host.draft_authoring_view(draft).unwrap();
        let node = view.graph.nodes.iter().find(|node| node.id == id).unwrap();
        let port = node
            .ports
            .iter()
            .find(|port| port.input && port.id == name)
            .unwrap();
        let details = view
            .port_details
            .get(&id)
            .and_then(|ports| ports.iter().find(|port| port.id == name));
        (port.input_state(details), port.literal_editor(details))
    };
    assert_eq!(
        state(&draft, number, "value"),
        (InputState::Default, LiteralEditor::Plain)
    );
    assert_eq!(state(&draft, wait, "duration").0, InputState::Required);
    let value = Literal::value("core/i64@1".parse().unwrap(), json!("7"));
    assert!(
        host.set_input_literals(&mut draft, [((number, "value"), value.clone())])
            .unwrap()
    );
    assert_eq!(state(&draft, number, "value").0, InputState::Literal);
    let source = host
        .add_node(&mut draft, "core/whole-number", [0.0, 160.0])
        .unwrap();
    host.connect_through(&mut draft, (source, "result"), (number, "value"), None)
        .unwrap();
    assert_eq!(
        state(&draft, number, "value").0,
        InputState::Wired { shadowed: true }
    );
    assert!(
        host.set_input_literals(&mut draft, [((number, "value"), value)])
            .is_err()
    );
    assert!(draft.undo());
    assert_eq!(state(&draft, number, "value").0, InputState::Literal);
}

#[test]
fn nested_options_distinguish_present_none_from_omission() {
    let (host, _) = Host::new(HostOptions::default()).unwrap();
    let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
    let node = host.add_node(&mut draft, "core/value", [0.0, 0.0]).unwrap();
    let ty = "core/option@1<core/option@1<core/i64@1>>";
    host.choose_type(&mut draft, node, "T", Some(ty)).unwrap();
    let view = host.draft_authoring_view(&draft).unwrap();
    assert_eq!(
        view.port_details[&node][0].editor,
        LiteralEditor::Option { some: true }
    );
    let none = Literal::value(ty.parse().unwrap(), json!({"none": true}));
    let some_none = Literal::value(ty.parse().unwrap(), json!({"some": {"none": true}}));
    host.set_input_literals(&mut draft, [((node, "value"), none.clone())])
        .unwrap();
    host.set_input_literals(&mut draft, [((node, "value"), some_none.clone())])
        .unwrap();
    let stored = |draft: &orange_host::Draft| {
        draft
            .document()
            .node(node)
            .unwrap()
            .inputs
            .values()
            .next()
            .unwrap()
            .clone()
    };
    assert_eq!(stored(&draft), some_none);
    assert!(draft.undo());
    assert_eq!(stored(&draft), none);
    assert!(draft.redo());
    assert_eq!(stored(&draft), some_none);
}

#[test]
fn paired_list_literals_validate_and_undo_as_one_edit() {
    let (host, _) = Host::new(HostOptions::default()).unwrap();
    let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
    let map = host
        .add_node(&mut draft, "org.orange.collections/build-map", [0.0, 0.0])
        .unwrap();
    host.choose_type(&mut draft, map, "K", Some("core/string@1"))
        .unwrap();
    host.choose_type(&mut draft, map, "V", Some("core/i64@1"))
        .unwrap();
    let keys = Literal::value("core/list@1<core/string@1>".parse().unwrap(), json!(["a"]));
    let values = |payload| Literal::value("core/list@1<core/i64@1>".parse().unwrap(), payload);
    let before = draft.text().unwrap();
    assert!(
        host.set_input_literals(
            &mut draft,
            [
                ((map, "keys"), keys.clone()),
                ((map, "values"), values(json!([7])))
            ]
        )
        .is_err()
    );
    assert_eq!(draft.text().unwrap(), before);
    assert!(
        host.set_input_literals(
            &mut draft,
            [
                ((map, "keys"), keys),
                ((map, "values"), values(json!(["7"])))
            ]
        )
        .unwrap()
    );
    assert_eq!(draft.document().node(map).unwrap().inputs.len(), 2);
    assert!(draft.undo());
    assert!(draft.document().node(map).unwrap().inputs.is_empty());
    assert!(draft.redo());
    assert_eq!(draft.document().node(map).unwrap().inputs.len(), 2);
}

#[test]
fn record_omission_applies_only_to_schema_optional_fields() {
    let (host, _) = Host::new(HostOptions::default()).unwrap();
    let mut saved: serde_json::Value =
        serde_json::from_str(&orange_host::new_document().unwrap()).unwrap();
    saved["types"] = json!([{
        "id": "6c0e4a52-8b1f-4a5e-9c3d-1e2f3a4b5c6d", "name": "Setting",
        "kind": "record", "fields": [
            {"id":"optional", "name":"Optional", "type":"core/option@1<core/string@1>", "optional":true},
            {"id":"required", "name":"Required", "type":"core/option@1<core/string@1>"}
        ]
    }]);
    let mut draft = host.draft(&saved.to_string()).unwrap();
    let record = host
        .add_node(&mut draft, "org.orange.collections/make-record", [0.0, 0.0])
        .unwrap();
    host.set_config(
        &mut draft,
        record,
        &json!({"type":"local/t6c0e4a528b1f4a5e9c3d1e2f3a4b5c6d@1"}),
    )
    .unwrap();
    assert_eq!(
        host.record_omissions(&draft, record),
        [("optional".to_owned(), false)].into()
    );
    let outer = "core/option@1<core/option@1<core/string@1>>"
        .parse()
        .unwrap();
    host.set_input_literals(
        &mut draft,
        [(
            (record, "optional"),
            Literal::value(outer, json!({"none":true})),
        )],
    )
    .unwrap();
    assert!(host.record_omissions(&draft, record)["optional"]);
    assert!(draft.undo());
    assert!(!host.record_omissions(&draft, record)["optional"]);
}

#[test]
fn typed_manual_samples_match_the_running_type_and_expire_on_swap() {
    let (mut host, _) = Host::new(HostOptions::default()).unwrap();
    let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
    let entry = host
        .add_node(&mut draft, "core/on-manual", [0.0, 0.0])
        .unwrap();
    host.set_config(&mut draft, entry, &json!({"payload": "core/i64@1"}))
        .unwrap();
    let flow = host.open(&draft.text().unwrap()).unwrap();
    let request = host.manual_payload(flow, entry).unwrap();
    assert_eq!(request.ty.as_str(), "core/i64@1");
    assert_eq!(request.editor, LiteralEditor::Plain);
    let number = |value| Literal::value("core/i64@1".parse().unwrap(), value);
    assert!(
        host.fire_manual_literal(&request, &number(json!(7)))
            .is_err()
    );
    host.fire_manual_literal(&request, &number(json!("7")))
        .unwrap();

    host.set_config(
        &mut draft,
        entry,
        &json!({"payload": "core/option@1<core/i64@1>"}),
    )
    .unwrap();
    assert!(host.update(flow, &draft.text().unwrap()).unwrap().changed);
    assert!(
        host.fire_manual_literal(&request, &number(json!("7")))
            .is_err()
    );
    let request = host.manual_payload(flow, entry).unwrap();
    assert_eq!(request.editor, LiteralEditor::Option { some: true });
    let none = Literal::value(request.ty.clone(), json!({"none": true}));
    host.fire_manual_literal(&request, &none).unwrap();
    host.close(flow).unwrap();
}

#[test]
fn core_and_package_ports_read_their_help_from_the_catalogues() {
    let definition = orange_sdk::Definition::node("example.authoring", "fetch", "Fetch")
        .input("target_path", "core/string@1")
        .port_description("target_path", "Where to fetch from.")
        .build();
    let package = ServicePackage::new("example.authoring", "1.0.0")
        .node(&definition, Sink)
        .unwrap();
    let (mut host, _) = Host::with_services(HostOptions::default(), vec![package]).unwrap();
    host.add_catalog(
        "es",
        "example-authoring-fetch-port-target-path = De dónde traerlo.\n",
    )
    .unwrap();
    let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
    let branch = host
        .add_node(&mut draft, orange_host::intrinsic::BRANCH, [0.0, 0.0])
        .unwrap();
    let fetch = host
        .add_node(&mut draft, "example.authoring/fetch", [0.0, 200.0])
        .unwrap();
    for (language, condition, target) in [
        (
            "en",
            "The Boolean that chooses the branch.",
            "Where to fetch from.",
        ),
        ("es", "El booleano que elige la rama.", "De dónde traerlo."),
        ("ja", "分岐を選ぶ真偽値。", "Where to fetch from."),
    ] {
        let view = host.draft_authoring_view_in(&draft, language).unwrap();
        let help = |node, port: &str| {
            view.port_details[&node]
                .iter()
                .find(|detail| detail.id == port)
                .and_then(|detail| detail.description.clone())
        };
        assert_eq!(
            help(branch, "condition").as_deref(),
            Some(condition),
            "{language}"
        );
        assert_eq!(
            help(fetch, "target_path").as_deref(),
            Some(target),
            "{language}"
        );
    }
    let all = host.port_help_all_in(orange_host::intrinsic::BRANCH, "en");
    assert!(all.iter().any(|(port, _)| port == "condition"));
}
