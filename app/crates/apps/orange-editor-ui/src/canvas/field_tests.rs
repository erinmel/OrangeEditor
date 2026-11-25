//! Card fields driven through egui input, frame by frame, as a person uses them.
use super::*;

fn id(n: u32) -> NodeId {
    format!("20000000-0000-4000-8000-{n:012x}").parse().unwrap()
}

#[test]
fn for_each_shows_its_default_list_mode_and_the_map_choice() {
    let (host, _) = orange_host::Host::new(orange_host::HostOptions::default()).unwrap();
    let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
    host.add_node(&mut draft, "core/for-each", [0.0, 0.0])
        .unwrap();
    let graph = host.draft_view(&draft).unwrap();
    let node = &graph.nodes[0];
    let schema = &node.schema.as_ref().unwrap()["properties"]["mode"];
    assert_eq!(
        fields::shown(node, card::FieldOf::Setting("mode", schema, None)).0,
        t("for-each-list")
    );
    assert_eq!(
        crate::properties::setting_choice_label(node, "map"),
        t("for-each-map")
    );
    assert!(
        node.ports
            .iter()
            .any(|port| port.id == "element" && !port.input)
    );
}

fn port(name: &str, input: bool, exec: bool, ty: &str) -> PortView {
    PortView {
        id: name.into(),
        input,
        exec,
        ty: (!exec).then(|| ty.to_owned()),
        value: None,
        connected: false,
        default: None,
        label: None,
    }
}

#[test]
fn a_file_path_label_identifies_its_folder_connection() {
    let mut node = node();
    node.definition = "org.orange.files/write-text".into();
    let label = crate::properties::folder_path_label(&node, "path".into(), Some("Reports"));
    assert!(label.starts_with("path"));
    assert!(label.ends_with("Reports"));
    let mut input = port("path", true, false, "core/string@1");
    input.value = Some("monthly/today.csv".into());
    let path = orange_host::PathInput {
        port: "path".into(),
        folder: false,
        save: true,
        extensions: Vec::new(),
    };
    assert_eq!(
        fields::shown(&node, card::FieldOf::Path(&input, &path)),
        ("monthly/today.csv".into(), false)
    );
    node.definition = "org.orange.audio/play".into();
    assert_eq!(
        crate::properties::folder_path_label(&node, "path".into(), Some("Speakers")),
        "path"
    );
}

#[test]
fn diagnostic_marks_use_the_requested_connector_when_port_names_match() {
    use orange_host::PortDirection::{Input, Output};
    let mut session = Session::new();
    let node = std::sync::Arc::make_mut(&mut session.graph.nodes[0]);
    node.ports.push(port("shared", true, false, "core/i64@1"));
    node.ports
        .push(port("shared", false, false, "core/stream@1<core/i64@1>"));
    let geometry = Geometry::new(&session.graph);
    let input = geometry
        .diagnostic_port(id(1), "shared", Some(Input))
        .unwrap();
    let output = geometry
        .diagnostic_port(id(1), "shared", Some(Output))
        .unwrap();
    assert!(geometry.diagnostic_port(id(1), "shared", None).is_none());
    assert!(input.offset.x < output.offset.x);
    for zoom in [0.35, 1.0, 2.0] {
        assert_eq!(
            input.offset * zoom,
            geometry.port(id(1), "shared", true).unwrap().offset * zoom
        );
        assert_eq!(
            output.offset * zoom,
            geometry.port(id(1), "shared", false).unwrap().offset * zoom
        );
    }
    assert!(stream_type(output.ty));
    assert!(!stream_type(input.ty));
}

/// A branch-like node: a way in, two ways out, a true-or-false input, a number and
/// a choice setting.
fn node() -> NodeView {
    NodeView {
        id: id(1),
        name: "Check".into(),
        kind: "Check".into(),
        definition: "org.example/check".into(),
        detail: None,
        entry: false,
        ports: vec![
            port("in", true, true, ""),
            port("yes", false, true, ""),
            port("no", false, true, ""),
            port("condition", true, false, "core/bool@1"),
            port("count", true, false, "core/i64@1"),
        ],
        position: None,
        settings: serde_json::json!({}),
        schema: Some(serde_json::json!({"type": "object", "properties": {
            "mode": {"enum": ["fast", "slow"], "default": "fast"}
        }})),
        references: Vec::new(),
        types: Vec::new(),
        variadic: Vec::new(),
        paths: Vec::new(),
        controls: Default::default(),
        missing: None,
    }
}

/// Runs frames of one canvas over one graph and returns each frame's response.
struct Session {
    context: egui::Context,
    canvas: Canvas,
    graph: GraphView,
    /// Where the canvas starts on the screen.
    corner: Pos2,
    request_inputs: Option<BTreeMap<String, String>>,
}

impl Session {
    fn new() -> Self {
        let mut canvas = Canvas::new();
        canvas.origin = Pos2::ZERO;
        canvas.fit = false;
        canvas.places.insert(id(1), Pos2::new(40.0, 40.0));
        Self {
            context: egui::Context::default(),
            canvas,
            graph: GraphView {
                nodes: vec![node().into()],
                wires: vec![],
            },
            corner: Pos2::ZERO,
            request_inputs: None,
        }
    }

    fn frame(&mut self, events: Vec<egui::Event>) -> CanvasResponse {
        let (response, output) = self.frame_output(events);
        output.drop_without_applying_deltas();
        response
    }

    fn frame_output(&mut self, events: Vec<egui::Event>) -> (CanvasResponse, egui::FullOutput) {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 700.0))),
            events,
            ..Default::default()
        };
        let mut response = CanvasResponse::default();
        let read = |_, port: &str| self.request_inputs.as_ref()?.get(port).cloned();
        let request_input = self
            .request_inputs
            .as_ref()
            .map(|_| &read as &StoredText<'_>);
        let (canvas, graph, corner) = (&mut self.canvas, &self.graph, &mut self.corner);
        let output = self.context.run_ui(input, |ui| {
            *corner = ui.available_rect_before_wrap().min;
            response = canvas.show_with_path_controls(ui, graph, &[], false, request_input);
        });
        (response, output)
    }

    /// The middle of the control of the field on row `index`, on the screen.
    fn control(&self, index: usize) -> Rect {
        let node = &self.graph.nodes[0];
        let layout = card::Card::new(node);
        let control = layout.rows[index].control(layout.size.x).unwrap();
        // The canvas's own transform: from its origin, at its zoom.
        let place = self.canvas.places[&node.id];
        let to_screen =
            |at: Pos2| self.corner + (place + at.to_vec2() - self.canvas.origin) * self.canvas.zoom;
        Rect::from_min_max(to_screen(control.min), to_screen(control.max))
    }

    /// Clicks at `at`: the pointer moves there, presses and lets go.
    fn click(&mut self, at: Pos2) -> CanvasResponse {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let _ = self.frame(vec![egui::Event::PointerMoved(at)]);
        let _ = self.frame(vec![button(true)]);
        self.frame(vec![button(false)])
    }
}

#[test]
fn keyboard_and_click_can_wire_from_either_end_or_cancel_without_editing() {
    let (host, _) = orange_host::Host::new(orange_host::HostOptions::default()).unwrap();
    let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
    let start = host
        .add_node(&mut draft, "core/on-manual", [40.0, 40.0])
        .unwrap();
    let wait = host
        .add_node(&mut draft, "core/wait", [440.0, 40.0])
        .unwrap();
    let mut session = Session::new();
    session.graph = host.draft_view(&draft).unwrap();
    session.canvas.load(&session.graph);
    session.canvas.origin = Pos2::ZERO;
    session.canvas.fit = false;
    let _ = session.frame(vec![]);

    let port_at = |node, key, input| {
        let geometry = Geometry::with_cards(&session.graph, |node| session.canvas.card(node));
        session.corner
            + (session.canvas.places[&node] + geometry.port(node, key, input).unwrap().offset
                - session.canvas.origin)
                * session.canvas.zoom
    };
    let out = port_at(start, "then", false);
    let into = port_at(wait, "in", true);
    let press = |session: &mut Session, key, modifiers| {
        let event = |pressed| egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        };
        let result = session.frame(vec![event(true)]);
        let _ = session.frame(vec![event(false)]);
        result
    };
    let focus_port = |session: &mut Session, at: Pos2| {
        for _ in 0..24 {
            let focused = session.context.memory(|memory| memory.focused());
            if focused
                .and_then(|id| session.context.read_response(id))
                .is_some_and(|response| response.rect.center().distance(at) < 1.0)
            {
                return;
            }
            let _ = press(session, egui::Key::Tab, egui::Modifiers::NONE);
        }
        panic!("could not reach connector at {at:?} with Tab");
    };
    for (first, second) in [(out, into), (into, out)] {
        focus_port(&mut session, first);
        assert!(
            press(&mut session, egui::Key::Enter, egui::Modifiers::NONE)
                .connect
                .is_none()
        );
        assert!(session.canvas.keyboard_wiring.is_some());
        focus_port(&mut session, second);
        assert_eq!(
            press(&mut session, egui::Key::Space, egui::Modifiers::NONE).connect,
            Some(((start, "then".to_owned()), (wait, "in".to_owned())))
        );
        assert!(session.canvas.keyboard_wiring.is_none());
    }
    let click_at = |session: &mut Session, pos, modifiers| {
        let event = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers,
        };
        let _ = session.frame(vec![egui::Event::PointerMoved(pos), event(true)]);
        session.frame(vec![event(false)])
    };
    for (first, second) in [(out, into), (into, out)] {
        let _ = click_at(&mut session, first, egui::Modifiers::NONE);
        assert!(session.canvas.keyboard_wiring.is_some());
        assert_eq!(
            click_at(&mut session, second, egui::Modifiers::NONE).connect,
            Some(((start, "then".to_owned()), (wait, "in".to_owned())))
        );
    }
    focus_port(&mut session, into);
    let _ = press(&mut session, egui::Key::Enter, egui::Modifiers::NONE);
    assert!(
        press(&mut session, egui::Key::Escape, egui::Modifiers::NONE)
            .connect
            .is_none()
    );
    assert!(session.canvas.keyboard_wiring.is_none());
    focus_port(&mut session, into);
    assert!(matches!(
        press(&mut session, egui::Key::F10, egui::Modifiers::SHIFT).context,
        Some((crate::MenuTarget::Port { node, input: true, .. }, _, _)) if node == wait
    ));
}

#[test]
fn failure_disclosure_accepts_keyboard_and_preserves_primary_anchors() {
    let (host, _) = orange_host::Host::new(orange_host::HostOptions::default()).unwrap();
    let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
    let id = host
        .add_node(&mut draft, "core/wait", [40.0, 40.0])
        .unwrap();
    let handler = host
        .add_node(&mut draft, "core/wait", [480.0, 40.0])
        .unwrap();
    let view = host.draft_authoring_view(&draft).unwrap();
    let mut session = Session::new();
    crate::icons::install(&session.context);
    session.graph = view.graph;
    session.canvas.set_failure_routes(view.failure_routes);
    session.canvas.load(&session.graph);
    session.canvas.origin = Pos2::ZERO;
    session.canvas.fit = false;
    let _ = session.frame(vec![]);

    // Represent a provider with one named success exit to exercise the pulse fallback.
    let node = std::sync::Arc::make_mut(&mut session.graph.nodes[0]);
    node.ports
        .iter_mut()
        .find(|port| port.id == "then")
        .unwrap()
        .id = "success".into();
    let layout = session.canvas.card(node);
    let disclosure = layout
        .rows
        .iter()
        .find(|row| matches!(row.left, Some(card::Left::FailureRoutes { .. })))
        .unwrap();
    let at = session.corner
        + (session.canvas.places[&id] + Vec2::new(60.0, disclosure.anchor())
            - session.canvas.origin)
            * session.canvas.zoom;
    let anchor = |session: &Session, key: &str, input: bool| {
        let geometry = Geometry::with_cards(&session.graph, |node| session.canvas.card(node));
        geometry.port(id, key, input).map(|port| port.offset)
    };
    let primary = [
        anchor(&session, "in", true),
        anchor(&session, "success", false),
    ];
    assert!(primary.iter().all(Option::is_some));
    assert_eq!(anchor(&session, "failed", false), None);

    let _ = session.click(at);
    assert!(session.canvas.failure_routes_expanded(id));
    assert!(anchor(&session, "failed", false).is_some());
    assert!(anchor(&session, "cancelled", false).is_some());
    assert!(anchor(&session, "error", false).is_some());
    let key = |key, pressed| egui::Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    for (keyboard, expanded) in [(egui::Key::Enter, false), (egui::Key::Space, true)] {
        let _ = session.frame(vec![key(keyboard, true)]);
        let _ = session.frame(vec![key(keyboard, false)]);
        assert_eq!(session.canvas.failure_routes_expanded(id), expanded);
        assert_eq!(
            [
                anchor(&session, "in", true),
                anchor(&session, "success", false)
            ],
            primary
        );
    }
    session
        .canvas
        .record_step(id, false, orange_host::ControlOutcome::Then);
    let geometry = Geometry::with_cards(&session.graph, |node| session.canvas.card(node));
    assert!(session.canvas.exit_glow(&geometry, id, "success") > 0.0);
    assert_eq!(session.canvas.exit_glow(&geometry, id, "failed"), 0.0);
    for (outcome, route, other) in [
        (orange_host::ControlOutcome::Failed, "failed", "cancelled"),
        (
            orange_host::ControlOutcome::Cancelled,
            "cancelled",
            "failed",
        ),
    ] {
        session.canvas.record_step(id, route == "failed", outcome);
        assert!(session.canvas.exit_glow(&geometry, id, route) > 0.0);
        assert_eq!(session.canvas.exit_glow(&geometry, id, other), 0.0);
        assert_eq!(session.canvas.exit_glow(&geometry, id, "success"), 0.0);
    }
    let screen = |node, port, margin| {
        session.corner
            + (session.canvas.places[&node]
                + geometry.port(node, port, node == handler).unwrap().offset
                - session.canvas.origin)
                * session.canvas.zoom
            + Vec2::new(margin, 0.0)
    };
    let from = screen(id, "failed", 4.0);
    let to = screen(handler, "in", 4.0);
    let mouse = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    let _ = session.frame(vec![egui::Event::PointerMoved(from)]);
    let _ = session.frame(vec![mouse(from, true)]);
    let _ = session.frame(vec![egui::Event::PointerMoved(from + Vec2::X * 20.0)]);

    session.canvas.toggle_failure_routes(id);
    assert!(anchor(&session, "failed", false).is_some());
    assert_eq!(anchor(&session, "cancelled", false), None);
    let _ = session.frame(vec![egui::Event::PointerMoved(to)]);
    let drag = session.frame(vec![mouse(to, false)]);
    assert_eq!(
        drag.connect,
        Some(((id, "failed".into()), (handler, "in".into())))
    );
    session
        .canvas
        .focus_problem(&session.graph, id, Some("cancelled".to_owned()));
    let focus = anchor(&session, "cancelled", false).unwrap();
    let centered = session.canvas.places[&id] + focus - session.canvas.origin;
    assert_eq!(centered * session.canvas.zoom, session.canvas.size / 2.0);
    assert_eq!(anchor(&session, "failed", false), None);
    let node = std::sync::Arc::make_mut(&mut session.graph.nodes[0]);
    node.ports
        .iter_mut()
        .find(|port| port.id == "error")
        .unwrap()
        .connected = true;
    assert!(anchor(&session, "error", false).is_some());
    assert!(anchor(&session, "failed", false).is_some());
}

#[test]
fn double_clicking_the_card_title_requests_a_rename() {
    for (y, expected) in [(HEADER_HEIGHT / 2.0, Some(id(1))), (BODY_TOP, None)] {
        let mut session = Session::new();
        let _ = session.frame(vec![]);
        let at = session.corner
            + (session.canvas.places[&id(1)] + Vec2::new(45.0, y) - session.canvas.origin)
                * session.canvas.zoom;
        assert_eq!(session.click(at).rename_node, None);
        assert_eq!(session.click(at).rename_node, expected);
    }
}

#[test]
fn a_package_node_title_can_be_renamed_in_its_card() {
    let mut session = Session::new();
    let (host, _) = orange_host::Host::with_services(
        orange_host::HostOptions::default(),
        vec![orange_audio::package()],
    )
    .unwrap();
    let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
    let node = host
        .add_node(&mut draft, "org.orange.audio/gain", [40.0, 40.0])
        .unwrap();
    assert!(
        host.rename_node(&mut draft, node, "Headphone gain")
            .unwrap()
    );
    session.graph = host.draft_view(&draft).unwrap();
    session.canvas.places.clear();
    session.canvas.places.insert(node, Pos2::new(40.0, 40.0));
    let name = session.graph.nodes[0].name.clone();
    let title_y = |output: &egui::FullOutput| {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(title) if title.galley.text() == name => Some(title.pos.y),
                _ => None,
            })
            .unwrap()
    };
    let (_, idle) = session.frame_output(vec![]);
    let baseline = title_y(&idle);
    idle.drop_without_applying_deltas();
    let at = session.corner
        + (session.canvas.places[&node] + Vec2::new(45.0, HEADER_HEIGHT / 2.0)
            - session.canvas.origin)
            * session.canvas.zoom;
    session.click(at);
    assert_eq!(session.click(at).rename_node, Some(node));
    let (_, editing) = session.frame_output(vec![]);
    let editing_y = title_y(&editing);
    assert!(
        (editing_y - baseline).abs() <= 1.0,
        "idle {baseline}, editing {editing_y}"
    );
    editing.drop_without_applying_deltas();
    session.frame(vec![egui::Event::Text("Monitor gain".to_owned())]);
    let result = session.frame(vec![egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    assert_eq!(result.set_name, Some((node, "Monitor gain".into())));
    assert!(session.canvas.node_rename.is_none());
}

#[test]
fn a_fitted_card_keeps_its_title_at_overview_zoom() {
    let mut session = Session::new();
    session.canvas.zoom = 0.39;
    let output = session.context.run_ui(egui::RawInput::default(), |ui| {
        let _ = session.canvas.show(ui, &session.graph, &[]);
    });
    let title = output.shapes.iter().find_map(|shape| match &shape.shape {
        egui::Shape::Text(text) if text.galley.text() == "Check" => text
            .galley
            .job
            .sections
            .first()
            .map(|section| section.format.font_id.size),
        _ => None,
    });
    output.drop_without_applying_deltas();
    assert_eq!(title, Some(9.0));
}

#[test]
fn both_number_literal_cards_use_the_math_icon() {
    let mut view = node();
    for definition in ["core/number", "core/whole-number"] {
        view.definition = definition.into();
        assert_eq!(family_icon(&view), crate::icons::MATH);
    }
}

#[test]
fn request_body_is_edited_on_the_card_from_the_complete_stored_text() {
    let mut session = Session::new();
    let body = format!("{{\n  \"data\": \"{}\"\n}}\n", "x".repeat(160));
    session.request_inputs = Some(BTreeMap::from([("content".to_owned(), body.clone())]));
    let node = std::sync::Arc::make_mut(&mut session.graph.nodes[0]);
    node.definition = "org.orange.net/http-request".into();
    node.schema = None;
    let mut input = port("content", true, false, "core/string@1");
    input.value = Some("\"truncated preview…\"".to_owned());
    node.ports = vec![input];
    let _ = session.frame(vec![]);
    assert!(
        session
            .click(session.control(0).center())
            .configure
            .is_none()
    );
    let _ = session.frame(vec![]);
    assert_eq!(session.canvas.expanded_text.as_ref().unwrap().value, body);

    let next = "{\n  \"message\": \"hello\"\n}";
    let select_all = egui::Event::Key {
        key: egui::Key::A,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    };
    let _ = session.frame(vec![select_all, egui::Event::Paste(next.to_owned())]);
    let enter = egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    assert!(session.frame(vec![enter]).set_input.is_none());
    let save = egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    };
    let (node, port, literal) = session.frame(vec![save]).set_input.unwrap();
    assert_eq!((node, port.as_str()), (id(1), "content"));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&literal).unwrap()["value"],
        format!("{next}\n")
    );
    assert!(session.canvas.expanded_text.is_none());

    let _ = session.click(session.control(0).center());
    let _ = session.frame(vec![]);
    let _ = session.frame(vec![egui::Event::Paste("Discard me".into())]);
    let escape = egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    assert!(session.frame(vec![escape]).set_input.is_none());
    assert!(session.canvas.expanded_text.is_none());
}

#[test]
fn dragging_a_text_grip_resizes_the_card_and_recenters_its_input_anchor() {
    let mut session = Session::new();
    session.request_inputs = Some(BTreeMap::new());
    let node = std::sync::Arc::make_mut(&mut session.graph.nodes[0]);
    node.definition = "org.orange.net/mqtt-publish".into();
    node.schema = None;
    node.ports = vec![port("text", true, false, "core/string@1")];
    let _ = session.frame(vec![]);
    let control = session.control(0);
    let before = session.canvas.card(&session.graph.nodes[0]);
    let (height, dot) = (
        before.size.y,
        before.offset(&session.graph.nodes[0].ports[0]),
    );
    let grip = control.center_bottom() - Vec2::new(0.0, 4.0);
    let event = |pos, pressed| egui::Event::PointerButton {
        pos,
        pressed,
        button: egui::PointerButton::Primary,
        modifiers: egui::Modifiers::NONE,
    };
    let _ = session.frame(vec![egui::Event::PointerMoved(grip)]);
    let _ = session.frame(vec![event(grip, true)]);
    let after = grip + Vec2::new(0.0, 46.0);
    let _ = session.frame(vec![egui::Event::PointerMoved(after)]);
    let _ = session.frame(vec![event(after, false)]);
    let changed = session.canvas.card(&session.graph.nodes[0]);
    assert!(changed.size.y > height);
    assert_eq!(
        changed.offset(&session.graph.nodes[0].ports[0]),
        dot.map(|dot| dot + Vec2::new(0.0, (changed.size.y - height) / 2.0))
    );
}

#[test]
fn binary_request_headers_commit_the_line_list_when_the_field_loses_focus() {
    let mut session = Session::new();
    session.request_inputs = Some(BTreeMap::new());
    let node = std::sync::Arc::make_mut(&mut session.graph.nodes[0]);
    node.definition = "org.orange.net/http-request-bytes".into();
    node.schema = None;
    node.ports = vec![port("headers", true, false, "core/list@1<core/string@1>")];
    let _ = session.frame(vec![]);
    let _ = session.click(session.control(0).center());
    let _ = session.frame(vec![]);
    let _ = session.frame(vec![egui::Event::Paste(
        "Accept: application/json\n\nX-Example: 1".into(),
    )]);
    let outside = Pos2::new(12.0, 12.0);
    let _ = session.frame(vec![egui::Event::PointerMoved(outside)]);
    let down = |pressed| egui::Event::PointerButton {
        pos: outside,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    let press = session.frame(vec![down(true)]);
    let release = session.frame(vec![down(false)]);
    let (_, port, literal) = press.set_input.or(release.set_input).unwrap();
    assert_eq!(port, "headers");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&literal).unwrap(),
        serde_json::json!({"type": "core/list@1<core/string@1>", "value": ["Accept: application/json", "X-Example: 1"]}),
    );
}

#[test]
fn multiline_text_fields_keep_their_height_on_focus_and_apply() {
    for definition in [
        "org.orange.net/mqtt-publish",
        "org.orange.net/amqp-publish",
        "org.orange.net/ws-send",
        "org.example/text-node",
    ] {
        let mut session = Session::new();
        session.request_inputs = Some(BTreeMap::new());
        let node = std::sync::Arc::make_mut(&mut session.graph.nodes[0]);
        node.definition = definition.into();
        node.schema = None;
        node.ports = vec![
            port("text", true, false, "core/string@1"),
            port("flag", true, false, "core/bool@1"),
        ];
        let _ = session.frame(vec![]);
        let height = |session: &Session| session.canvas.card(&session.graph.nodes[0]).size.y;
        let anchor = |session: &Session, port: &str| {
            Geometry::with_cards(&session.graph, |node| session.canvas.card(node))
                .port(id(1), port, true)
                .unwrap()
                .offset
        };
        let initial_height = height(&session);
        let text_anchor = anchor(&session, "text");
        let flag_anchor = anchor(&session, "flag");
        assert!(
            session
                .click(session.control(0).center())
                .configure
                .is_none()
        );
        let _ = session.frame(vec![]);
        assert_eq!(height(&session), initial_height, "{definition}");
        assert_eq!(anchor(&session, "text"), text_anchor);
        assert_eq!(anchor(&session, "flag"), flag_anchor);
        let message = "{\n  \"message\": \"hello\"\n}";
        let _ = session.frame(vec![egui::Event::Paste(message.into())]);
        let apply = egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        };
        let (_, key, literal) = session.frame(vec![apply]).set_input.unwrap();
        assert_eq!(key, "text");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&literal).unwrap()["value"],
            message
        );
        assert_eq!(height(&session), initial_height, "{definition}");
        assert_eq!(anchor(&session, "flag"), flag_anchor);
    }
}

/// The rows, by what they hold: flow, flow, condition, count, mode.
#[test]
fn fields_follow_the_flow_rows_in_declared_order() {
    let node = node();
    let layout = card::Card::new(&node);
    let names: Vec<_> = layout
        .rows
        .iter()
        .map(|row| match row.left {
            Some(card::Left::Port(port)) => port.id.clone(),
            Some(card::Left::Field { editor, .. }) => match editor {
                card::FieldOf::Input(port) => port.id.clone(),
                card::FieldOf::Setting(name, _, _) => name.to_owned(),
                _ => "?".into(),
            },
            _ => String::new(),
        })
        .collect();
    assert_eq!(names, ["in", "", "condition", "count", "mode"]);
}

/// Clicking the Yes half of a switch writes true; the No half writes false.
#[test]
fn a_switch_writes_the_half_that_is_clicked() {
    let mut session = Session::new();
    let _ = session.frame(vec![]);
    let switch = session.control(2);
    let yes = Pos2::new(switch.right() - 10.0, switch.center().y);
    let done = session.click(yes);
    assert_eq!(
        done.set_input,
        Some((id(1), "condition".into(), "true".into()))
    );
    std::sync::Arc::make_mut(&mut session.graph.nodes[0]).ports[3].value = Some("true".into());
    let no = Pos2::new(switch.left() + 10.0, switch.center().y);
    let done = session.click(no);
    assert_eq!(
        done.set_input,
        Some((id(1), "condition".into(), "false".into()))
    );
    // Clicking the half already chosen writes nothing.
    std::sync::Arc::make_mut(&mut session.graph.nodes[0]).ports[3].value = Some("false".into());
    assert_eq!(session.click(no).set_input, None);
}

/// A number field is clicked to type, and Enter keeps what was typed.
#[test]
fn a_number_is_typed_after_a_click_and_kept_on_enter() {
    let mut session = Session::new();
    let _ = session.frame(vec![]);
    let field = session.control(3);
    let done = session.click(field.center());
    assert_eq!(done.set_input, None);
    // The field takes the keyboard on the frame after the click.
    let _ = session.frame(vec![]);
    let _ = session.frame(vec![egui::Event::Text("42".into())]);
    let enter = egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    let mut written = None;
    for _ in 0..3 {
        let done = session.frame(vec![enter.clone()]);
        if done.set_input.is_some() {
            written = done.set_input;
            break;
        }
    }
    assert_eq!(written, Some((id(1), "count".into(), "42".into())));
}

#[test]
fn durations_show_milliseconds_and_step_in_exact_nanoseconds() {
    let mut duration = port("attack", true, false, "core/duration@1");
    duration.default = Some("10000000".into());
    assert_eq!(
        fields::shown(&node(), card::FieldOf::Input(&duration)),
        ("10 ms".into(), true)
    );
    let stepped = stepped(Some("10.000001"), 12.0, false, "core/duration@1");
    assert_eq!(
        fields::duration_nanos(&stepped).as_deref(),
        Some("13000001")
    );
    assert_eq!(fields::duration_nanos("0.000001").as_deref(), Some("1"));
    let boundary = i64::MAX.to_string();
    let shown = fields::duration_millis(&boundary).unwrap();
    assert_eq!(fields::duration_nanos(&shown), Some(boundary));
    assert!(fields::duration_nanos("0.0000001").is_none());
}

/// A choice setting opens its list on a click and writes the chosen item.
#[test]
fn a_dropdown_writes_the_chosen_setting() {
    let mut session = Session::new();
    let _ = session.frame(vec![]);
    let field = session.control(4);
    let _ = session.click(field.center());
    // The list opens under the field; its second item is "slow".
    let item = Pos2::new(field.left() + 20.0, field.bottom() + 8.0 + 1.5 * 18.0);
    let mut written = None;
    for dy in [0.0, -6.0, 6.0, -12.0, 12.0] {
        let done = session.click(item + Vec2::new(0.0, dy));
        if done.set_config.is_some() {
            written = done.set_config;
            break;
        }
        let _ = session.click(field.center());
    }
    assert_eq!(written, Some((id(1), "/mode".into(), "slow".into())));
}

/// A port the node may have more of ends the card with a button, and clicking it asks
/// for one more instance.
#[test]
fn a_card_adds_one_more_of_a_growing_port() {
    let mut session = Session::new();
    std::sync::Arc::make_mut(&mut session.graph.nodes[0])
        .variadic
        .push(orange_host::VariadicView {
            port: "branch".into(),
            input: false,
            count: 2,
            min: 1,
            max: None,
        });
    let _ = session.frame(vec![]);
    let last = card::Card::new(&session.graph.nodes[0]).rows.len() - 1;
    let button = session.control(last);
    let done = session.click(button.center());
    assert_eq!(done.add_port, Some((id(1), "branch".into())));
    // At its most, the button is gone.
    std::sync::Arc::make_mut(&mut session.graph.nodes[0]).variadic[0].max = Some(2);
    let layout = card::Card::new(&session.graph.nodes[0]);
    assert!(!layout.rows.iter().any(|row| matches!(
        row.left,
        Some(card::Left::Field {
            editor: card::FieldOf::AddPort(_),
            ..
        })
    )));
}

/// A splitter's band outputs read from low to high frequency, whatever order the bands
/// were drawn in.
#[test]
fn band_outputs_are_ordered_by_frequency() {
    let (host, _) = orange_host::Host::with_services(
        orange_host::HostOptions::default(),
        vec![orange_audio::package()],
    )
    .unwrap();
    let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
    let splitter = host
        .add_node(&mut draft, orange_audio::MULTIBAND_SPECTRUM, [0.0, 0.0])
        .unwrap();
    for range in [[2_000, 3_000], [60, 120], [400, 800]] {
        host.add_audio_band(&mut draft, splitter, range).unwrap();
    }
    let graph = host.draft_view(&draft).unwrap();
    let node = graph.nodes.iter().find(|node| node.id == splitter).unwrap();
    let layout = card::Card::new(node);
    let lows: Vec<f64> = layout
        .rows
        .iter()
        .filter_map(|row| row.right)
        .filter_map(|port| spectrum::band_low(node, &port.id))
        .collect();
    assert_eq!(lows, [60.0, 400.0, 2_000.0]);
}

#[test]
fn audio_trigger_hold_dial_renders_in_preview_and_edits_the_canvas_setting() {
    let (host, _) = orange_host::Host::with_services(
        orange_host::HostOptions::default(),
        vec![orange_audio::package()],
    )
    .unwrap();
    let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
    let trigger = host
        .add_node(&mut draft, orange_audio::NOISE_LEVEL, [40.0, 40.0])
        .unwrap();
    let mut session = Session::new();
    session.graph = host.draft_view(&draft).unwrap();
    session.canvas.places.insert(trigger, Pos2::new(40.0, 40.0));
    let node = session.graph.nodes[0].clone();
    let layout = card::Card::new(&node);
    let hold = layout
        .rows
        .iter()
        .position(|row| {
            matches!(
                row.left,
                Some(card::Left::Field {
                    editor: card::FieldOf::Setting("hold_ms", _, Some(_)),
                    ..
                })
            )
        })
        .unwrap();
    let is_hold_dial = |shapes: &[egui::epaint::ClippedShape]| {
        shapes.iter().any(
            |item| matches!(&item.shape, egui::Shape::Text(text) if text.galley.text() == "150"),
        )
    };
    let (_, canvas) = session.frame_output(vec![]);
    assert!(is_hold_dial(&canvas.shapes));
    canvas.drop_without_applying_deltas();
    let preview = session.context.run_ui(egui::RawInput::default(), |ui| {
        preview_card(
            ui,
            &node,
            &node.name,
            Rect::from_min_size(Pos2::new(20.0, 20.0), layout.size),
            1.0,
        );
    });
    assert!(is_hold_dial(&preview.shapes));
    preview.drop_without_applying_deltas();
    let _ = session.click(session.control(hold).center());
    let done = session.frame(vec![egui::Event::Key {
        key: egui::Key::ArrowRight,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    let (node, setting, value) = done.set_config.expect("hold dial changes the setting");
    assert_eq!(node, trigger);
    assert_eq!(setting, "/hold_ms");
    assert_eq!(value.as_f64(), Some(151.0));

    session.canvas.zoom = *ZOOM_RANGE.end();
    let (_, canvas) = session.frame_output(vec![]);
    // The dial is centered in its slot; its well sits inside the ring of ticks.
    let slot = session.control(hold);
    let center = slot.center();
    let diameter = slot.height().min(slot.width());
    let radius = diameter * 0.5 - 1.0 - (diameter * 0.085).max(3.0) - 2.0;
    assert!(canvas.shapes.iter().any(|item| {
        matches!(&item.shape, egui::Shape::Circle(circle)
            if circle.center.distance(center) < 0.5 && (circle.radius - radius).abs() < 0.1)
    }));
    canvas.drop_without_applying_deltas();
}

#[test]
fn audio_trigger_keeps_stereo_lanes_and_clears_clip_lights_when_stopped() {
    use orange_ui_controls::{UiTokens, audio_meter::ChannelReading};

    let (host, _) = orange_host::Host::with_services(
        orange_host::HostOptions::default(),
        vec![orange_audio::package()],
    )
    .unwrap();
    let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
    let source = host
        .add_node(&mut draft, orange_audio::AUDIO_SOURCE, [20.0, 40.0])
        .unwrap();
    let trigger = host
        .add_node(&mut draft, orange_audio::NOISE_LEVEL, [330.0, 40.0])
        .unwrap();
    host.connect(&mut draft, (source, "stream"), (trigger, "stream"))
        .unwrap();
    let mut session = Session::new();
    UiTokens::install(
        &session.context,
        orange_ui_controls::ThemeMode::Dark,
        orange_ui_controls::Density::Compact,
    );
    session.graph = host.draft_view(&draft).unwrap();
    session.canvas.load(&session.graph);
    session.canvas.fit = false;
    session.canvas.begin_run();
    session.canvas.noise_readings.insert(trigger, -16.0);
    session.canvas.noise_channels.insert(
        trigger,
        spectrum::ChannelLevels::Stereo([
            ChannelReading {
                rms: -12.0,
                peak: 0.0,
                clipped: true,
            },
            ChannelReading {
                rms: -24.0,
                peak: -18.0,
                clipped: false,
            },
        ]),
    );
    let meter_frame = |session: &mut Session, status| {
        let (_, output) = session.frame_output(vec![]);
        let theme = UiTokens::resolve(&session.context).theme;
        let red = theme.meter_zone_inks()[2];
        let unlit = theme.bg_canvas().lerp_to_gamma(red, 0.25);
        let labels = ["L", "R"].map(|label| {
            output
                .shapes
                .iter()
                .find_map(|item| match &item.shape {
                    egui::Shape::Text(text) if text.galley.text() == label => Some(text.pos),
                    _ => None,
                })
                .expect("channel label")
        });
        let mut lights: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|item| match &item.shape {
                egui::Shape::Circle(dot) if [red, unlit].contains(&dot.fill) => {
                    Some((dot.center, dot.fill == red))
                }
                _ => None,
            })
            .collect();
        lights.sort_by(|a, b| a.0.x.total_cmp(&b.0.x));
        assert!(output.shapes.iter().any(
            |item| matches!(&item.shape, egui::Shape::Text(text) if text.galley.text() == status)
        ));
        output.drop_without_applying_deltas();
        (labels, lights)
    };
    let (live_labels, live_lights) = meter_frame(&mut session, t("audio-display-live"));
    assert!(
        matches!(live_lights.as_slice(), [(_, true), (_, false)]),
        "{live_lights:?}"
    );

    session.canvas.audio_running = false;
    session.canvas.mark_stale();
    let (preview_labels, preview_lights) =
        meter_frame(&mut session, t("audio-display-preview-short"));
    assert_eq!(preview_labels, live_labels);
    assert_eq!(
        preview_lights,
        [(live_lights[0].0, false), (live_lights[1].0, false)]
    );
}

#[test]
fn threshold_meter_drag_previews_every_control_and_commits_once() {
    let (host, _) = orange_host::Host::with_services(
        orange_host::HostOptions::default(),
        vec![orange_audio::package()],
    )
    .unwrap();
    let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
    let trigger = host
        .add_node(&mut draft, orange_audio::NOISE_LEVEL, [40.0, 40.0])
        .unwrap();
    let mut session = Session::new();
    session.graph = host.draft_view(&draft).unwrap();
    session.canvas.places.insert(trigger, Pos2::new(40.0, 40.0));
    let node = session.graph.nodes[0].clone();
    let threshold = crate::properties::ThresholdEdit::for_node(&node).unwrap();
    let _ = session.frame(vec![]);
    let zoom = session.canvas.zoom;
    let top = session.corner + (session.canvas.places[&trigger] - session.canvas.origin) * zoom;
    let from = top + Vec2::new(10.0, BODY_TOP + 18.0 + 163.0 * 35.0 / 120.0) * zoom;
    let to = from + Vec2::new(0.0, 32.0) * zoom;
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    session.frame(vec![egui::Event::PointerMoved(from)]);
    session.frame(vec![button(from, true)]);
    session.frame(vec![egui::Event::PointerMoved(from + (to - from) * 0.5)]);
    let (drag, output) = session.frame_output(vec![egui::Event::PointerMoved(to)]);
    assert!(drag.set_config.is_none());
    let shown = threshold.shown(&session.context);
    assert!(shown < -50.0, "{shown}");
    let labels: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|item| match &item.shape {
            egui::Shape::Text(text) => Some(text.galley.text().replace('−', "-")),
            _ => None,
        })
        .collect();
    output.drop_without_applying_deltas();
    assert!(
        labels
            .iter()
            .any(|label| label.contains(&format!("{shown:.1}"))),
        "{labels:?}"
    );
    let (config, pointer, value) = session.frame(vec![button(to, false)]).set_config.unwrap();
    assert_eq!(
        (config, pointer.as_str(), value.as_f64()),
        (trigger, "/threshold_db", Some(shown))
    );
    assert!(session.frame(vec![]).set_config.is_none());
    session
        .context
        .run_ui(egui::RawInput::default(), |ui| {
            let mut response = ui.label("threshold");
            response.mark_changed();
            assert_eq!(threshold.update(&response, f64::NAN), None);
            assert_eq!(threshold.update(&response, f64::INFINITY), None);
        })
        .drop_without_applying_deltas();

    let mut changed = (*node).clone();
    changed
        .ports
        .push(port("sidechain", true, false, "core/f64@1"));
    assert!(!spectrum::is_noise(&changed));
    assert!(port_offset(&changed, "sidechain", true).is_some());
}

/// Dragging a band's edge on the splitter's chart changes that edge and keeps the other.
#[test]
fn dragging_a_band_edge_resizes_the_band() {
    let (host, _) = orange_host::Host::with_services(
        orange_host::HostOptions::default(),
        vec![orange_audio::package()],
    )
    .unwrap();
    let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
    let splitter = host
        .add_node(&mut draft, orange_audio::MULTIBAND_SPECTRUM, [0.0, 0.0])
        .unwrap();
    host.add_audio_band(&mut draft, splitter, [100, 1_000])
        .unwrap();
    let mut session = Session::new();
    session.graph = host.draft_view(&draft).unwrap();
    session.canvas.places.clear();
    session
        .canvas
        .places
        .insert(splitter, Pos2::new(40.0, 40.0));
    let _ = session.frame(vec![]);
    let node = session.graph.nodes[0].clone();
    let band = node.settings["bands"]
        .as_object()
        .unwrap()
        .keys()
        .next()
        .unwrap()
        .parse::<u16>()
        .unwrap();
    // The card on the screen, through the canvas's own transform.
    let place = session.canvas.places[&splitter];
    let corner = session.corner + (place - session.canvas.origin) * session.canvas.zoom;
    let rect = Rect::from_min_size(corner, node_size(&node) * session.canvas.zoom);
    let plot = spectrum::plot_rect(rect, session.canvas.zoom);
    let x = |hz: f64| plot.left() + plot.width() * spectrum::plot_fraction(hz);
    let from = Pos2::new(x(1_000.0), plot.center().y);
    let to = Pos2::new(x(2_000.0), plot.center().y);
    let button = |at, pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    let _ = session.frame(vec![egui::Event::PointerMoved(from)]);
    let _ = session.frame(vec![button(from, true)]);
    for step in 1..=4 {
        let at = from + (to - from) * (step as f32 / 4.0);
        let _ = session.frame(vec![egui::Event::PointerMoved(at)]);
    }
    let done = session.frame(vec![button(to, false)]);
    let Some((_, BandEdit::Change(changed, Some(settings)))) = done.spectrum_edit else {
        panic!("no band change: {:?}", done.spectrum_edit);
    };
    assert_eq!(changed, band);
    assert_eq!(settings["low_hz"].as_f64(), Some(100.0));
    let high = settings["high_hz"].as_f64().unwrap();
    assert!((1_900.0..=2_100.0).contains(&high), "{high}");
}

/// A way out glows as a step leaves through it, wired or not, and only that one.
#[test]
fn the_way_out_a_step_takes_glows_and_fades() {
    let mut session = Session::new();
    session.canvas.record_entry_route(id(1), "yes".into());
    let geometry = Geometry::new(&session.graph);
    assert!(session.canvas.exit_glow(&geometry, id(1), "yes") > 0.9);
    assert_eq!(session.canvas.exit_glow(&geometry, id(1), "no"), 0.0);
    // A step that ends by its only way out lights it too.
    let mut single = node();
    single.ports.retain(|port| port.id != "no");
    session.graph.nodes = vec![single.into()];
    session
        .canvas
        .record_step(id(1), false, orange_host::ControlOutcome::Then);
    let geometry = Geometry::new(&session.graph);
    assert!(session.canvas.exit_glow(&geometry, id(1), "yes") > 0.9);
}
