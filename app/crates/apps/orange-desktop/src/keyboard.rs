//! Logical key presses enter through run-owned SDK subscriptions: while the editor has
//! focus, or anywhere for a binding set to Anywhere (see [`global`]).
mod global;

use eframe::egui::{Event, Key, Modifiers};
use global::{Chord, Signal};
use orange_sdk::{Arguments, Definition, Emitter, ServicePackage, ServiceSource, Session, Value};
use serde_json::{Map, Value as Json, json};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError, Weak};

#[cfg(test)]
const ON_KEY: &str = "org.orange.input/on-key-pressed";

#[derive(Debug, Clone)]
struct Binding {
    key: Key,
    /// Exactly the modifiers that must be held, each one named on its own.
    modifiers: Modifiers,
    repeat: bool,
    /// Also while the editor is in the background, when the system allows the chord.
    anywhere: bool,
}

impl Binding {
    fn chord(&self) -> Chord {
        Chord {
            key: self.key,
            ctrl: self.modifiers.ctrl,
            shift: self.modifiers.shift,
            alt: self.modifiers.alt,
            command: self.modifiers.mac_cmd,
        }
    }

    fn matches(&self, held: Modifiers) -> bool {
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

/// Most problems kept for the Log between frames.
const MAX_PROBLEMS: usize = 32;

#[derive(Debug, Default)]
struct State {
    next: u64,
    entries: BTreeMap<u64, (Binding, Emitter)>,
    /// Chords the system delivers from anywhere.
    active: BTreeSet<Chord>,
    /// Background chords held down, so a held key fires once.
    pressed: BTreeSet<Chord>,
    problems: Vec<String>,
}

impl State {
    /// Takes in what the background backend reports and returns whom to fire.
    fn signal(&mut self, signal: Signal) -> Vec<Emitter> {
        match signal {
            Signal::Pressed(chord) => {
                if !self.active.contains(&chord) || !self.pressed.insert(chord) {
                    return Vec::new();
                }
                self.entries
                    .values()
                    .filter(|(binding, _)| binding.anywhere && binding.chord() == chord)
                    .map(|(_, emitter)| emitter.clone())
                    .collect()
            }
            Signal::Released(chord) => {
                let _ = self.pressed.remove(&chord);
                Vec::new()
            }
            Signal::Active(active) => {
                self.pressed.retain(|chord| active.contains(chord));
                self.active = active;
                Vec::new()
            }
            Signal::Problem(text) => {
                if self.problems.len() < MAX_PROBLEMS {
                    self.problems.push(text);
                }
                Vec::new()
            }
        }
    }
}

/// Fires the entries a background signal reaches, outside the registry lock.
fn deliver(state: &Mutex<State>, signal: Signal) {
    let emitters = state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .signal(signal);
    for emitter in emitters {
        let _ = emitter.emit(None);
    }
}

#[derive(Debug, Default)]
pub struct Keyboard {
    state: Arc<Mutex<State>>,
    held: BTreeSet<Key>,
    armed: BTreeSet<Key>,
    /// Started on the first frame that has a binding set to Anywhere.
    backend: Option<global::Backend>,
    /// Why the backend could not start, reported once per change of the wanted chords.
    unavailable: Option<String>,
    /// The chords last asked of the backend.
    applied: BTreeSet<Chord>,
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
        let key = settings.get("key").and_then(Json::as_str).unwrap_or("F8");
        let key = Key::from_name(key).ok_or("Choose a supported logical key")?;
        let binding = Binding {
            key,
            modifiers: Modifiers {
                ctrl: flag(arguments, "ctrl").unwrap_or(false),
                shift: flag(arguments, "shift").unwrap_or(false),
                alt: flag(arguments, "alt").unwrap_or(false),
                mac_cmd: flag(arguments, "command").unwrap_or(false),
                command: false,
            },
            repeat: flag(arguments, "repeat").unwrap_or(false),
            anywhere: flag(arguments, "anywhere").unwrap_or(false),
        };
        if binding.anywhere {
            global::check(&binding.chord())?;
        }
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
        // Version 3: the key from a list, and each modifier its own No | Yes input.
        let off = || json!({"type": "core/bool@1", "value": false});
        let definition = Definition::entry("org.orange.input", "on-key-pressed", "On key pressed")
            .version(3)
            .description("Fires when the chosen key is pressed with exactly the modifiers set to Yes, while this editor has focus. Editing text and editor shortcuts take priority. Command is the macOS key. Release the key to fire again, unless Repeat is Yes. With Anywhere set to Yes it also fires while other applications are in front, once per press; a key that types text then needs Ctrl, Alt or Command.")
            .category("input", "Input")
            .keywords("keyboard, hotkey, shortcut, keybind, key press, input, trigger")
            .setting("key", json!({"title":"Key", "type":"string", "enum": Key::ALL.iter().copied().map(Key::name).collect::<Vec<_>>(), "default":"F8"}))
            .input_default("ctrl", "core/bool@1", off())
            .input_default("shift", "core/bool@1", off())
            .input_default("alt", "core/bool@1", off())
            .input_default("command", "core/bool@1", off())
            .input_default("repeat", "core/bool@1", off())
            .input_default("anywhere", "core/bool@1", off())
            .build();
        ServicePackage::new("org.orange.input", "0.1.0")
            .about(
                "Keyboard",
                "Starts flows when keys are pressed, while this editor has focus or anywhere.",
            )
            .with_catalogs(crate::CATALOGS)
            .source(&definition, Source(Arc::clone(&self.state)))
    }

    /// Makes the chords set to Anywhere the ones the system delivers. Call it from the
    /// thread that runs the window's event loop.
    fn sync_anywhere(&mut self) {
        let wanted: BTreeSet<Chord> = self
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entries
            .values()
            .filter(|(binding, _)| binding.anywhere)
            .map(|(binding, _)| binding.chord())
            .collect();
        if wanted == self.applied {
            return;
        }
        if self.backend.is_none() && self.unavailable.is_none() && !wanted.is_empty() {
            let state = Arc::downgrade(&self.state);
            let sink: global::Sink = Arc::new(move |signal| {
                if let Some(state) = state.upgrade() {
                    deliver(&state, signal);
                }
            });
            match global::Backend::start(sink) {
                Ok(backend) => self.backend = Some(backend),
                Err(reason) => self.unavailable = Some(reason),
            }
        }
        match (&mut self.backend, &self.unavailable) {
            (Some(backend), _) => backend.apply(&wanted),
            (None, Some(reason)) if !wanted.is_empty() => {
                deliver(&self.state, Signal::Problem(reason.clone()));
            }
            _ => {}
        }
        self.applied = wanted;
    }

    /// Fires the bindings this frame's key events reach and keeps the Anywhere chords
    /// registered. Returns what the person should read in the Log.
    pub fn handle(
        &mut self,
        events: &[Event],
        focused: bool,
        allowed: impl Fn(Key, Modifiers) -> bool,
    ) -> Vec<String> {
        self.sync_anywhere();
        let problems = std::mem::take(
            &mut self
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .problems,
        );
        if !focused {
            self.held.clear();
            self.armed.clear();
            return problems;
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
            let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            let emitters: Vec<_> = state
                .entries
                .values()
                .filter(|(binding, _)| {
                    binding.key == *key
                        && binding.matches(*modifiers)
                        && ((!was_down && !repeat) || binding.repeat)
                        // The system delivers an active Anywhere chord itself.
                        && !(binding.anywhere && state.active.contains(&binding.chord()))
                })
                .map(|(_, emitter)| emitter.clone())
                .collect();
            drop(state);
            for emitter in emitters {
                let _ = emitter.emit(None);
            }
        }
        problems
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

    fn start(
        keyboard: &Keyboard,
        key: &str,
        flags: &[&str],
        count: &Arc<AtomicUsize>,
    ) -> Result<Box<dyn Send>, String> {
        let seen = Arc::clone(count);
        let mut settings = Map::new();
        let _ = settings.insert("key".into(), json!(key));
        let arguments = Arguments::new(
            flags
                .iter()
                .map(|name| ((*name).to_owned(), Value::Bool(true)))
                .collect(),
        );
        Source(Arc::clone(&keyboard.state)).start_with_arguments(
            None,
            &settings,
            &arguments,
            Emitter::new(move |_| {
                seen.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }),
        )
    }

    #[test]
    fn anywhere_presses_fire_once_per_press_and_never_twice_with_focus() {
        let mut keyboard = Keyboard::default();
        let count = Arc::new(AtomicUsize::new(0));
        let lease = start(&keyboard, "F9", &["anywhere"], &count).unwrap();
        let chord = Chord {
            key: Key::F9,
            ctrl: false,
            shift: false,
            alt: false,
            command: false,
        };
        // As if the backend had registered it; no system hotkey is taken by the test.
        keyboard.applied = BTreeSet::from([chord]);
        // Not yet active: the system has not confirmed it, so only focus fires it.
        deliver(&keyboard.state, Signal::Pressed(chord));
        assert_eq!(count.load(Ordering::Relaxed), 0);
        let _ = keyboard.handle(&[event_for(Key::F9, true, false)], true, |_, _| true);
        assert_eq!(count.load(Ordering::Relaxed), 1);
        let _ = keyboard.handle(&[event_for(Key::F9, false, false)], true, |_, _| true);

        deliver(&keyboard.state, Signal::Active(BTreeSet::from([chord])));
        deliver(&keyboard.state, Signal::Pressed(chord));
        deliver(&keyboard.state, Signal::Pressed(chord));
        assert_eq!(count.load(Ordering::Relaxed), 2, "a held key fires once");
        // The focused path leaves an active chord to the system.
        let _ = keyboard.handle(&[event_for(Key::F9, true, false)], true, |_, _| true);
        assert_eq!(count.load(Ordering::Relaxed), 2);
        deliver(&keyboard.state, Signal::Released(chord));
        deliver(&keyboard.state, Signal::Pressed(chord));
        assert_eq!(count.load(Ordering::Relaxed), 3);

        deliver(&keyboard.state, Signal::Problem("taken".into()));
        assert_eq!(keyboard.handle(&[], false, |_, _| true), ["taken"]);
        assert!(keyboard.handle(&[], false, |_, _| true).is_empty());
        drop(lease);
        deliver(&keyboard.state, Signal::Released(chord));
        deliver(&keyboard.state, Signal::Pressed(chord));
        assert_eq!(
            count.load(Ordering::Relaxed),
            3,
            "a stopped entry never fires"
        );
    }

    #[test]
    fn a_typing_key_set_to_anywhere_is_refused_when_the_flow_starts() {
        let keyboard = Keyboard::default();
        let count = Arc::new(AtomicUsize::new(0));
        assert!(start(&keyboard, "K", &["anywhere"], &count).is_err());
        assert!(start(&keyboard, "K", &["anywhere", "shift"], &count).is_err());
        assert!(start(&keyboard, "K", &["anywhere", "ctrl"], &count).is_ok());
        // Focus-only bindings keep accepting any key.
        assert!(start(&keyboard, "K", &[], &count).is_ok());
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
                == Some(Value::I64(42))
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
