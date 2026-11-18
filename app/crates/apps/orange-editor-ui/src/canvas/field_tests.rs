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
                nodes: vec![node()],
                wires: vec![],
            },
            corner: Pos2::ZERO,
        }
    }

    fn frame(&mut self, events: Vec<egui::Event>) -> CanvasResponse {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 700.0))),
            events,
            ..Default::default()
        };
        let mut response = CanvasResponse::default();
        let (canvas, graph, corner) = (&mut self.canvas, &self.graph, &mut self.corner);
        let output = self.context.run_ui(input, |ui| {
            *corner = ui.available_rect_before_wrap().min;
            response = canvas.show(ui, graph, &[]);
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
    session.graph.nodes[0].ports[3].value = Some("true".into());
    let no = Pos2::new(switch.left() + 10.0, switch.center().y);
    let done = session.click(no);
    assert_eq!(
        done.set_input,
        Some((id(1), "condition".into(), "false".into()))
    );
    // Clicking the half already chosen writes nothing.
    session.graph.nodes[0].ports[3].value = Some("false".into());
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
