use super::*;
use crate::Editor;
use orange_host::{Host, HostOptions};
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
