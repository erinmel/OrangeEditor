//! Focused-window logical key presses enter through run-owned SDK subscriptions.
use eframe::egui::{Event, Key, Modifiers};
use orange_sdk::{Arguments, Definition, Emitter, ServicePackage, ServiceSource, Session, Value};
use serde_json::{Map, Value as Json, json};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError, Weak};

#[cfg(test)]
const ON_KEY: &str = "org.orange.input/on-key-pressed";

#[derive(Debug, Clone)]
struct Binding {
    key: Key,
    modifiers: Modifiers,
    repeat: bool,
    /// The chord is exactly the modifiers held, each one named on its own (version
    /// 3); older nodes name the Primary modifier and match logically.
    exact: bool,
}

impl Binding {
    fn matches(&self, held: Modifiers) -> bool {
        if !self.exact {
            return held.matches_logically(self.modifiers);
        }
        // egui reports Ctrl as `command` too on Windows and Linux; Command is the
        // macOS key only.
        held.ctrl == self.modifiers.ctrl
            && held.shift == self.modifiers.shift
            && held.alt == self.modifiers.alt
            && held.mac_cmd == self.modifiers.mac_cmd
    }
}

/// The value of a true-or-false argument, if the node has that input.
fn flag(arguments: &Arguments, name: &str) -> Option<bool> {
    match arguments.get(name) {
        Some(Value::Bool(on)) => Some(*on),
        _ => None,
    }
}

#[derive(Debug, Default)]
struct State {
    next: u64,
    entries: BTreeMap<u64, (Binding, Emitter)>,
}

#[derive(Debug, Default)]
pub struct Keyboard {
    state: Arc<Mutex<State>>,
    held: BTreeSet<Key>,
    armed: BTreeSet<Key>,
}

#[derive(Debug)]
struct Source(Arc<Mutex<State>>);

#[derive(Debug)]
struct Subscription {
    state: Weak<Mutex<State>>,
    id: u64,
}
impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(state) = self.state.upgrade() {
            state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .entries
                .remove(&self.id);
        }
    }
}

fn modifiers(name: &str) -> Option<Modifiers> {
    let mut modifiers = Modifiers::NONE;
    for part in name.split('+') {
        match part {
            "None" => {}
            "Primary" => modifiers.command = true,
            "Alt" => modifiers.alt = true,
            "Shift" => modifiers.shift = true,
            _ => return None,
        }
    }
    Some(modifiers)
}

impl ServiceSource for Source {
    fn start_with(
        &self,
        session: Option<Arc<dyn Session>>,
        settings: &Map<String, Json>,
        emitter: Emitter,
    ) -> Result<Box<dyn Send>, String> {
        self.start_with_arguments(session, settings, &Arguments::new(Vec::new()), emitter)
    }

    fn start_with_arguments(
        &self,
        _session: Option<Arc<dyn Session>>,
        settings: &Map<String, Json>,
        arguments: &Arguments,
        emitter: Emitter,
    ) -> Result<Box<dyn Send>, String> {
        let selected = match arguments.get("key") {
            Some(Value::Str(key)) if !key.is_empty() => Some(key.as_str()),
            _ => None,
        };
        let key = selected
            .or_else(|| settings.get("key").and_then(Json::as_str))
            .unwrap_or("F8");
        let key = Key::from_name(key).ok_or("Choose a supported logical key")?;
        // Version 3 names each modifier as a true-or-false input; older versions
        // name a chord in a setting.
        let exact = flag(arguments, "ctrl").is_some();
        let modifiers = if exact {
            Modifiers {
                ctrl: flag(arguments, "ctrl").unwrap_or(false),
                shift: flag(arguments, "shift").unwrap_or(false),
                alt: flag(arguments, "alt").unwrap_or(false),
                mac_cmd: flag(arguments, "command").unwrap_or(false),
                command: false,
            }
        } else {
            let modifier_name = settings
                .get("modifiers")
                .and_then(Json::as_str)
                .unwrap_or("None");
            modifiers(modifier_name).ok_or("Choose supported modifiers")?
        };
        let repeat = flag(arguments, "repeat").unwrap_or_else(|| {
            settings
                .get("repeat")
                .and_then(Json::as_bool)
                .unwrap_or(false)
        });
        let binding = Binding {
            key,
            modifiers,
            repeat,
            exact,
        };
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let id = state
            .next
            .checked_add(1)
            .ok_or("Keyboard subscription identifiers exhausted")?;
        state.next = id;
        state.entries.insert(id, (binding, emitter));
        Ok(Box::new(Subscription {
            state: Arc::downgrade(&self.0),
            id,
        }))
    }
}

impl Keyboard {
    pub fn package(&self) -> Result<ServicePackage, orange_sdk::DefinitionError> {
        let definition = Definition::entry("org.orange.input", "on-key-pressed", "On key pressed")
            .description("Fires when a logical key is pressed while this editor is focused. Editing text and editor shortcuts take priority. Primary means Ctrl on Windows/Linux and Command on macOS. Release the key to fire again; optional repeat follows the operating system. No global keyboard hook.")
            .category("input", "Input")
            .keywords("keyboard, hotkey, shortcut, keybind, key press, input, trigger")
            .setting("key", json!({"title":"Key", "type":"string", "enum": Key::ALL.iter().copied().map(Key::name).collect::<Vec<_>>(), "default":"F8"}))
            .setting("modifiers", json!({"title":"Modifiers", "type":"string", "enum":["None","Primary","Alt","Shift","Primary+Shift","Primary+Alt","Alt+Shift","Primary+Alt+Shift"], "default":"None"}))
            .setting("repeat", json!({"title":"Allow key repeat", "type":"boolean", "default":false}));
        let previous = definition.clone().hidden().build();
        let wired = definition
            .clone()
            .version(2)
            .input_default(
                "key",
                "core/string@1",
                json!({"type": "core/string@1", "value": ""}),
            )
            .hidden()
            .build();
        // Version 3: the key from a list, and each modifier its own No | Yes input.
        let off = || json!({"type": "core/bool@1", "value": false});
        let definition = Definition::entry("org.orange.input", "on-key-pressed", "On key pressed")
            .version(3)
            .description("Fires when the chosen key is pressed with exactly the modifiers set to Yes, while this editor has focus. Editing text and editor shortcuts take priority. Command is the macOS key. Release the key to fire again, unless Repeat is Yes. No global keyboard hook.")
            .category("input", "Input")
            .keywords("keyboard, hotkey, shortcut, keybind, key press, input, trigger")
            .setting("key", json!({"title":"Key", "type":"string", "enum": Key::ALL.iter().copied().map(Key::name).collect::<Vec<_>>(), "default":"F8"}))
            .input_default("ctrl", "core/bool@1", off())
            .input_default("shift", "core/bool@1", off())
            .input_default("alt", "core/bool@1", off())
            .input_default("command", "core/bool@1", off())
            .input_default("repeat", "core/bool@1", off())
            .build();
        ServicePackage::new("org.orange.input", "0.1.0")
            .with_catalogs(crate::CATALOGS)
            .source(&previous, Source(Arc::clone(&self.state)))?
            .source(&wired, Source(Arc::clone(&self.state)))?
            .source(&definition, Source(Arc::clone(&self.state)))
    }

    pub fn handle(
        &mut self,
        events: &[Event],
        focused: bool,
        allowed: impl Fn(Key, Modifiers) -> bool,
    ) {
        if !focused {
            self.held.clear();
            self.armed.clear();
            return;
        }
        for event in events {
            let Event::Key {
                key,
                pressed,
                repeat,
                modifiers,
                ..
            } = event
            else {
                continue;
            };
            if !pressed {
                self.held.remove(key);
                self.armed.remove(key);
                continue;
            }
            let was_down = !self.held.insert(*key);
            let accepted = allowed(*key, *modifiers);
            if !repeat && !was_down && accepted {
                self.armed.insert(*key);
            }
            if !accepted || !self.armed.contains(key) {
                continue;
            }
            // Release the registry lock before entering the host's bounded ingress.
            let emitters: Vec<_> = self
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .entries
                .values()
                .filter(|(binding, _)| {
                    binding.key == *key
                        && binding.matches(*modifiers)
                        && ((!was_down && !repeat) || binding.repeat)
                })
                .map(|(_, emitter)| emitter.clone())
                .collect();
            for emitter in emitters {
                let _ = emitter.emit(None);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn event(pressed: bool, repeat: bool) -> Event {
        event_for(Key::F8, pressed, repeat)
    }

    fn event_for(key: Key, pressed: bool, repeat: bool) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat,
            modifiers: Modifiers::NONE,
        }
    }

    #[test]
    fn focused_presses_rearm_after_release_and_stop_with_the_subscription() {
        let mut keyboard = Keyboard::default();
        let count = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&count);
        let source = Source(Arc::clone(&keyboard.state));
        let lease = source
            .start_with(
                None,
                &Map::new(),
                Emitter::new(move |_| {
                    seen.fetch_add(1, Ordering::Relaxed);
                    Ok(())
                }),
            )
            .unwrap();
        keyboard.handle(&[event(true, false), event(true, true)], true, |_, _| true);
        assert_eq!(count.load(Ordering::Relaxed), 1);
        keyboard.handle(&[event(false, false), event(true, false)], true, |_, _| {
            true
        });
        assert_eq!(count.load(Ordering::Relaxed), 2);
        keyboard.handle(&[], false, |_, _| true);
        keyboard.handle(&[event(true, true)], true, |_, _| true);
        assert_eq!(count.load(Ordering::Relaxed), 2);
        keyboard.handle(&[event(false, false), event(true, false)], true, |_, _| {
            false
        });
        keyboard.handle(&[event(true, true)], true, |_, _| true);
        assert_eq!(count.load(Ordering::Relaxed), 2);
        drop(lease);
        keyboard.handle(&[event(false, false), event(true, false)], true, |_, _| {
            true
        });
        assert_eq!(count.load(Ordering::Relaxed), 2);
        assert!(keyboard.state.lock().unwrap().entries.is_empty());
    }

    #[test]
    fn key_node_is_discoverable_and_starts_only_with_a_running_flow() {
        let mut keyboard = Keyboard::default();
        let (mut host, _) = orange_host::Host::with_services(
            orange_host::HostOptions::default(),
            vec![keyboard.package().unwrap()],
        )
        .unwrap();
        assert!(
            host.search_nodes("keybind", "en")
                .iter()
                .any(|node| node.definition == ON_KEY)
        );
        let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
        let entry = host.add_node(&mut draft, ON_KEY, [0.0, 0.0]).unwrap();
        // Ctrl+F9: the key from its list, Ctrl set to Yes.
        host.set_config(&mut draft, entry, &json!({"key": "F9"}))
            .unwrap();
        host.set_input(&mut draft, (entry, "ctrl"), Some("true"))
            .unwrap();
        let variable = host
            .declare_variable(
                &mut draft,
                &orange_host::Variable {
                    id: None,
                    name: "Key presses".into(),
                    ty: "core/i64@1".into(),
                    scope: orange_host::VarScope::Run,
                    persistent: false,
                    initial: None,
                },
            )
            .unwrap();
        let sink = host
            .add_node(&mut draft, "core/set-variable", [300.0, 0.0])
            .unwrap();
        host.set_config(&mut draft, sink, &json!({"variable": variable}))
            .unwrap();
        host.set_input(&mut draft, (sink, "value"), Some("42"))
            .unwrap();
        host.connect(&mut draft, (entry, "then"), (sink, "in"))
            .unwrap();
        assert!(host.problems(&mut draft).unwrap().is_empty());
        assert!(keyboard.state.lock().unwrap().entries.is_empty());
        let flow = host.open(&draft.text().unwrap()).unwrap();
        let variable_id = variable.parse::<orange_host::VariableId>().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while keyboard.state.lock().unwrap().entries.is_empty() {
            assert!(
                std::time::Instant::now() < deadline,
                "key source was not armed"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        keyboard.handle(&[event(true, false)], true, |_, _| true);
        assert_eq!(
            host.inspect(flow, move |run| run.variable(variable_id).cloned())
                .unwrap(),
            None
        );
        // F9 alone is not the chord; Ctrl+Shift+F9 is not either.
        keyboard.handle(
            &[
                event_for(Key::F9, true, false),
                event_for(Key::F9, false, false),
            ],
            true,
            |_, _| true,
        );
        let chord = |shift| {
            let mut press = event_for(Key::F9, true, false);
            if let Event::Key { modifiers, .. } = &mut press {
                *modifiers = Modifiers {
                    ctrl: true,
                    shift,
                    command: !cfg!(target_os = "macos"),
                    ..Modifiers::NONE
                };
            }
            press
        };
        keyboard.handle(
            &[chord(true), event_for(Key::F9, false, false)],
            true,
            |_, _| true,
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert_eq!(
            host.inspect(flow, move |run| run.variable(variable_id).cloned())
                .unwrap(),
            None
        );
        keyboard.handle(&[chord(false)], true, |_, _| true);
        loop {
            if host
                .inspect(flow, move |run| run.variable(variable_id).cloned())
                .unwrap()
                == Some(orange_host::Value::I64(42))
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "key press did not reach the graph action"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        host.stop(flow, orange_host::StopMode::Graceful).unwrap();
        host.close(flow).unwrap();
        assert!(keyboard.state.lock().unwrap().entries.is_empty());
    }
}
