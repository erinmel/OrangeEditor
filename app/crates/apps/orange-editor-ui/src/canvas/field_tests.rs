//! Card fields driven through egui input, frame by frame, as a person uses them.
use super::*;

fn id(n: u32) -> NodeId {
    format!("20000000-0000-4000-8000-{n:012x}").parse().unwrap()
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
        output.drop_without_applying_deltas();
        response
    }

    /// The middle of the control of the field on row `index`, on the screen.
    fn control(&self, index: usize) -> Rect {
        let node = &self.graph.nodes[0];
        let layout = card::Card::new(node);
        let control = layout.rows[index].control(layout.size.x).unwrap();
        // The canvas's own transform: from its origin, at its zoom.
        let place = self.canvas.places[&id(1)];
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
fn messaging_and_package_text_ports_expand_on_the_card_and_collapse_after_applying() {
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
        let compact = height(&session);
        let text_anchor = anchor(&session, "text");
        let flag_anchor = anchor(&session, "flag");
        assert!(
            session
                .click(session.control(0).center())
                .configure
                .is_none()
        );
        let _ = session.frame(vec![]);
        let growth = height(&session) - compact;
        assert!(growth > 0.0, "{definition}");
        assert_eq!(anchor(&session, "text"), text_anchor);
        assert_eq!(
            anchor(&session, "flag"),
            flag_anchor + Vec2::new(0.0, growth)
        );
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
        assert_eq!(height(&session), compact, "{definition}");
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
                card::FieldOf::Setting(name, _) => name.to_owned(),
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
        host.add_filtered_audio_band(&mut draft, splitter, range)
            .unwrap();
    }
    let graph = host.draft_view(&draft).unwrap();
    let node = graph.nodes.iter().find(|node| node.id == splitter).unwrap();
    let layout = card::Card::new(node);
    let lows: Vec<f64> = layout
        .rows
        .iter()
        .filter_map(|row| row.right)
        .filter_map(|port| spectrum::band_of(&port.id))
        .filter_map(|band| spectrum::band_low(node, band))
        .collect();
    assert_eq!(lows, [60.0, 400.0, 2_000.0]);
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
    host.add_filtered_audio_band(&mut draft, splitter, [100, 1_000])
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
