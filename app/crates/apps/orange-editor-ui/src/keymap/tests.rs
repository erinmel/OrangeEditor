use super::*;
use crate::Editor;
use orange_host::{Host, HostOptions};

#[test]
fn typed_shortcuts_round_trip_and_report_collisions() {
    for typed in [
        "Ctrl + shift + k",
        "Ctrl++",
        "Alt+F12",
        "Escape",
        "Shift+Tab",
        "+",
    ] {
        let binding = parse_shortcut(typed).expect(typed);
        assert_eq!(parse_shortcut(&format_shortcut(binding)), Some(binding));
    }
    assert_eq!(parse_shortcut("Ctrl+Alt"), None);
    assert_eq!(parse_shortcut("Hyper+S"), None);
    let keys = Keymap::default();
    let save = parse_shortcut(&keys.label("save")).unwrap();
    assert_eq!(keys.conflict("duplicate", save), Some("save"));
    assert_eq!(keys.conflict("save", save), None);
}

#[test]
fn older_shortcuts_keep_their_binding_when_it_conflicts_with_a_new_default() {
    let mut keys = Keymap::default();
    keys.restore(&serde_json::json!({
        "duplicate": {"key": "1", "command": true},
        "arrange": {"key": "3", "command": true}
    }));
    assert_eq!(keys.0["duplicate"], original("left-panel"));
    assert_eq!(keys.0["left-panel"], None);
    assert_eq!(keys.0["arrange"], parse_shortcut("Command+3"));
    assert_eq!(keys.0["right-panel"], original("right-panel"));
}

#[test]
fn panel_and_group_shortcuts_work_from_the_editor() {
    let (mut host, _) = Host::new(HostOptions::default()).unwrap();
    let mut editor = Editor::new();
    editor.open_untitled(&mut host, orange_host::new_document().unwrap());
    let first = editor.add_node(&mut host, "core/whole-number").unwrap();
    let second = editor.add_node(&mut host, "core/whole-number").unwrap();
    editor.canvas.select_many(&[first, second]);
    editor.panels.properties = false;
    editor.panels.packages = true;
    let ctx = egui::Context::default();
    let mut press = |editor: &mut Editor, action| {
        let binding = original(action).unwrap();
        for pressed in [true, false] {
            ctx.run_ui(
                egui::RawInput {
                    events: vec![
                        egui::Event::ModifiersChanged(binding.modifiers),
                        egui::Event::Key {
                            key: binding.logical_key,
                            modifiers: binding.modifiers,
                            physical_key: None,
                            pressed,
                            repeat: false,
                        },
                    ],
                    ..Default::default()
                },
                |ui| editor.ui(ui, &mut host),
            )
            .drop_without_applying_deltas();
        }
    };

    assert!(editor.panels.nodes);
    press(&mut editor, "left-panel");
    assert!(!editor.panels.nodes);
    press(&mut editor, "left-panel");
    assert!(editor.panels.nodes);
    press(&mut editor, "right-panel");
    assert!(!editor.panels.properties && !editor.panels.packages);
    press(&mut editor, "right-panel");
    assert!(!editor.panels.properties && editor.panels.packages);
    press(&mut editor, "snap");
    assert!(!editor.canvas.snap);
    press(&mut editor, "ungroup");
    assert!(editor.groups().is_empty());
    press(&mut editor, "group");
    assert_eq!(editor.groups().len(), 1);
    press(&mut editor, "ungroup");
    assert!(editor.groups().is_empty());
    assert_eq!(editor.graph().unwrap().nodes.len(), 2);
}

#[test]
fn recording_an_inline_shortcut_keeps_all_modifiers() {
    let mut editor = Editor::new();
    editor.begin_shortcut_edit("save", original("save"));
    editor.shortcut_edit.as_mut().unwrap().recording = true;
    let context = egui::Context::default();
    let frame = |editor: &mut Editor, events| {
        context
            .run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    editor.keybindings_ui(ui, "save graph");
                },
            )
            .drop_without_applying_deltas();
    };
    frame(&mut editor, vec![]);
    let modifiers = Modifiers::CTRL.plus(Modifiers::SHIFT).plus(Modifiers::ALT);
    frame(
        &mut editor,
        vec![
            egui::Event::ModifiersChanged(modifiers),
            egui::Event::Key {
                key: Key::K,
                physical_key: None,
                modifiers,
                pressed: true,
                repeat: false,
            },
            egui::Event::Text("K".to_owned()),
        ],
    );
    let edit = editor.shortcut_edit.as_ref().unwrap();
    assert!(!edit.recording);
    assert_eq!(
        parse_shortcut(&edit.text),
        Some(normalized(modifiers, Key::K))
    );
    assert_eq!(editor.canvas.keys.0["save"], original("save"));
}

#[test]
fn typing_a_conflict_only_reassigns_after_explicit_confirmation() {
    let mut editor = Editor::new();
    editor.settings_open = true;
    editor.settings_tab = crate::settings::Section::Shortcuts;
    editor.begin_shortcut_edit("duplicate", original("duplicate"));
    let context = egui::Context::default();
    let frame = |editor: &mut Editor, events| {
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 800.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                let _ = editor.settings_window(ui.ctx());
            },
        );
        let shapes = std::mem::take(&mut output.shapes);
        output.drop_without_applying_deltas();
        shapes
    };
    for _ in 0..3 {
        frame(&mut editor, vec![]);
    }
    let save = editor.canvas.keys.label("save");
    frame(&mut editor, vec![egui::Event::Text(save.clone())]);
    let mut shapes = Vec::new();
    for _ in 0..8 {
        shapes = frame(&mut editor, vec![]);
    }
    assert_eq!(editor.shortcut_edit.as_ref().unwrap().text, save);
    assert_eq!(editor.canvas.keys.0["save"], original("save"));
    assert_eq!(editor.canvas.keys.0["duplicate"], original("duplicate"));
    let at = shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == t("shortcut-reassign") => {
                Some(text.pos + text.galley.size() / 2.0)
            }
            _ => None,
        })
        .expect("explicit reassign button");
    let click = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    frame(&mut editor, vec![egui::Event::PointerMoved(at)]);
    frame(&mut editor, vec![click(true)]);
    frame(&mut editor, vec![click(false)]);
    assert_eq!(editor.canvas.keys.0["save"], None);
    assert_eq!(editor.canvas.keys.label("duplicate"), save);
}
#[test]
fn grid_preferences_round_trip_and_keep_defaults_for_old_settings() {
    use crate::canvas::{DEFAULT_GRID_STEP, GridStyle};
    let mut editor = Editor::new();
    editor.canvas.grid_step = 36;
    editor.canvas.grid_style = GridStyle::Lines;
    editor.canvas.snap = false;
    let encoded = editor.preferences();
    editor.restore_preferences(&serde_json::json!({"version": 1}));
    assert_eq!(editor.canvas.grid_step, DEFAULT_GRID_STEP);
    assert_eq!(editor.canvas.grid_style, GridStyle::Dots);
    assert!(editor.canvas.snap);
    editor.restore_preferences(&encoded);
    assert_eq!(editor.canvas.grid_step, 36);
    assert_eq!(editor.canvas.grid_style, GridStyle::Lines);
    assert!(!editor.canvas.snap);
}

#[test]
fn saved_rebinding_is_used_by_the_editor_keyboard_entrypoint() {
    let (mut host, _) = Host::new(HostOptions::default()).unwrap();
    let mut editor = Editor::new();
    editor.open(&mut host, "keys.json", orange_host::new_document().unwrap());
    let node = editor.add_node(&mut host, "core/whole-number").unwrap();
    editor.canvas.select(Some(node));
    editor.canvas.keys.0.insert(
        "duplicate".into(),
        Some(KeyboardShortcut::new(Modifiers::NONE, Key::F7)),
    );
    let saved = editor.preferences();
    editor.canvas.keys = Keymap::default();
    editor.restore_preferences(&saved);
    let context = egui::Context::default();
    let mut frame = |editor: &mut Editor, key: Option<(Key, Modifiers)>| {
        let mut input = egui::RawInput::default();
        if let Some((key, modifiers)) = key {
            input.events = vec![
                egui::Event::ModifiersChanged(modifiers),
                egui::Event::Key {
                    key,
                    modifiers,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                },
            ];
        }
        context
            .run_ui(input, |ui| editor.ui(ui, &mut host))
            .drop_without_applying_deltas();
    };
    frame(&mut editor, None);
    frame(&mut editor, None);
    frame(&mut editor, Some((Key::D, Modifiers::COMMAND)));
    assert_eq!(editor.graph().unwrap().nodes.len(), 1);
    frame(&mut editor, Some((Key::F7, Modifiers::NONE)));
    assert_eq!(editor.graph().unwrap().nodes.len(), 2);
}
#[test]
fn pure_values_remain_visible_when_the_graph_starts() {
    let (mut host, _) = Host::new(HostOptions::default()).unwrap();
    let mut editor = Editor::new();
    editor.open(
        &mut host,
        "value.json",
        orange_host::new_document().unwrap(),
    );
    let node = editor.add_node(&mut host, "core/whole-number").unwrap();
    assert!(editor.set_input(&mut host, (node, "value"), Some("17")));
    assert_eq!(editor.canvas.shown_value(node, "result"), Some("17"));
    editor.run(&mut host);
    assert!(editor.is_running());
    assert_eq!(editor.canvas.shown_value(node, "result"), Some("17"));
    editor.stop(&mut host);
}
