//! Key presses while the editor is in the background, for On key pressed set to Anywhere.
//!
//! Windows, macOS and X11 register system hotkeys through `global-hotkey`. Registration
//! must happen on the thread that runs the window's event loop, so the desktop applies
//! the wanted set from its frame; presses arrive from the platform's own loop and fire
//! even while the window is minimized. On Wayland the desktop's GlobalShortcuts portal
//! is asked instead, and the person approves or changes each shortcut there.

use eframe::egui::Key;
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};

/// A key with exactly these modifiers held.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct Chord {
    pub key: Key,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    /// The macOS Command key.
    pub command: bool,
}

/// What a backend reports back to the keyboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Signal {
    Pressed(Chord),
    Released(Chord),
    /// The chords that now fire anywhere. Others fire only while the editor has focus.
    Active(BTreeSet<Chord>),
    /// Something the person should read in the Log.
    Problem(String),
}

pub(crate) type Sink = Arc<dyn Fn(Signal) + Send + Sync>;

/// Keys that type nothing and so can be taken from other applications on their own.
fn stands_alone(key: Key) -> bool {
    use Key::*;
    matches!(
        key,
        F1 | F2
            | F3
            | F4
            | F5
            | F6
            | F7
            | F8
            | F9
            | F10
            | F11
            | F12
            | F13
            | F14
            | F15
            | F16
            | F17
            | F18
            | F19
            | F20
            | F21
            | F22
            | F23
            | F24
            | MediaPlayPause
            | MediaTrackNext
            | MediaTrackPrevious
            | MediaStop
            | AudioVolumeMute
            | AudioVolumeDown
            | AudioVolumeUp
            | Pause
    )
}

/// Whether a chord may be taken from every application while a flow runs. A key that
/// types text needs Ctrl, Alt or Command, so typing elsewhere is never swallowed.
pub(crate) fn check(chord: &Chord) -> Result<(), &'static str> {
    if chord.command && !cfg!(target_os = "macos") {
        return Err("Command is the macOS key. Choose Ctrl or Alt to use this key anywhere.");
    }
    if code(chord.key).is_none() {
        return Err("This key cannot be used anywhere. Choose another key.");
    }
    if !(chord.ctrl || chord.alt || chord.command || stands_alone(chord.key)) {
        return Err(
            "To use a key anywhere, choose a function or media key, or add Ctrl, Alt or Command.",
        );
    }
    Ok(())
}

/// The physical key `global-hotkey` registers for a logical key. Windows and X11 look
/// letters up through the keyboard layout; macOS registers the US position.
pub(crate) fn code(key: Key) -> Option<Code> {
    use Key as K;
    Some(match key {
        K::A => Code::KeyA,
        K::B => Code::KeyB,
        K::C => Code::KeyC,
        K::D => Code::KeyD,
        K::E => Code::KeyE,
        K::F => Code::KeyF,
        K::G => Code::KeyG,
        K::H => Code::KeyH,
        K::I => Code::KeyI,
        K::J => Code::KeyJ,
        K::K => Code::KeyK,
        K::L => Code::KeyL,
        K::M => Code::KeyM,
        K::N => Code::KeyN,
        K::O => Code::KeyO,
        K::P => Code::KeyP,
        K::Q => Code::KeyQ,
        K::R => Code::KeyR,
        K::S => Code::KeyS,
        K::T => Code::KeyT,
        K::U => Code::KeyU,
        K::V => Code::KeyV,
        K::W => Code::KeyW,
        K::X => Code::KeyX,
        K::Y => Code::KeyY,
        K::Z => Code::KeyZ,
        K::Num0 => Code::Digit0,
        K::Num1 => Code::Digit1,
        K::Num2 => Code::Digit2,
        K::Num3 => Code::Digit3,
        K::Num4 => Code::Digit4,
        K::Num5 => Code::Digit5,
        K::Num6 => Code::Digit6,
        K::Num7 => Code::Digit7,
        K::Num8 => Code::Digit8,
        K::Num9 => Code::Digit9,
        K::F1 => Code::F1,
        K::F2 => Code::F2,
        K::F3 => Code::F3,
        K::F4 => Code::F4,
        K::F5 => Code::F5,
        K::F6 => Code::F6,
        K::F7 => Code::F7,
        K::F8 => Code::F8,
        K::F9 => Code::F9,
        K::F10 => Code::F10,
        K::F11 => Code::F11,
        K::F12 => Code::F12,
        K::F13 => Code::F13,
        K::F14 => Code::F14,
        K::F15 => Code::F15,
        K::F16 => Code::F16,
        K::F17 => Code::F17,
        K::F18 => Code::F18,
        K::F19 => Code::F19,
        K::F20 => Code::F20,
        K::F21 => Code::F21,
        K::F22 => Code::F22,
        K::F23 => Code::F23,
        K::F24 => Code::F24,
        K::ArrowUp => Code::ArrowUp,
        K::ArrowDown => Code::ArrowDown,
        K::ArrowLeft => Code::ArrowLeft,
        K::ArrowRight => Code::ArrowRight,
        K::Space => Code::Space,
        K::Enter => Code::Enter,
        K::Escape => Code::Escape,
        K::Tab => Code::Tab,
        K::Backspace => Code::Backspace,
        K::Insert => Code::Insert,
        K::Delete => Code::Delete,
        K::Home => Code::Home,
        K::End => Code::End,
        K::PageUp => Code::PageUp,
        K::PageDown => Code::PageDown,
        K::Minus => Code::Minus,
        K::Equals => Code::Equal,
        K::Comma => Code::Comma,
        K::Period => Code::Period,
        K::Semicolon => Code::Semicolon,
        K::Slash => Code::Slash,
        K::Backslash => Code::Backslash,
        K::OpenBracket => Code::BracketLeft,
        K::CloseBracket => Code::BracketRight,
        K::Backtick => Code::Backquote,
        K::Quote => Code::Quote,
        K::MediaPlayPause => Code::MediaPlayPause,
        K::MediaTrackNext => Code::MediaTrackNext,
        K::MediaTrackPrevious => Code::MediaTrackPrevious,
        K::MediaStop => Code::MediaStop,
        K::AudioVolumeMute => Code::AudioVolumeMute,
        K::AudioVolumeDown => Code::AudioVolumeDown,
        K::AudioVolumeUp => Code::AudioVolumeUp,
        K::Pause => Code::Pause,
        _ => return None,
    })
}

fn hotkey(chord: &Chord) -> Option<HotKey> {
    let mut mods = Modifiers::empty();
    mods.set(Modifiers::CONTROL, chord.ctrl);
    mods.set(Modifiers::SHIFT, chord.shift);
    mods.set(Modifiers::ALT, chord.alt);
    mods.set(Modifiers::META, chord.command);
    Some(HotKey::new(Some(mods), code(chord.key)?))
}

/// The chord as the person reads it, such as `Ctrl+Alt+F9`.
pub(crate) fn caption(chord: &Chord) -> String {
    let mut parts = Vec::new();
    for (held, name) in [
        (chord.ctrl, "Ctrl"),
        (chord.alt, "Alt"),
        (chord.shift, "Shift"),
        (chord.command, "Command"),
    ] {
        if held {
            parts.push(name);
        }
    }
    parts.push(chord.key.name());
    parts.join("+")
}

/// The chord in the XDG shortcut format the portal takes as a preferred trigger.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn xdg_trigger(chord: &Chord) -> Option<String> {
    use Key as K;
    let name: String = match chord.key {
        key if key.name().len() == 1 && key.name().as_bytes()[0].is_ascii_alphanumeric() => {
            key.name().to_ascii_lowercase()
        }
        key if matches!(key.name().as_bytes(), [b'F', ..]) && code(key).is_some() => {
            key.name().to_owned()
        }
        K::ArrowUp => "Up".into(),
        K::ArrowDown => "Down".into(),
        K::ArrowLeft => "Left".into(),
        K::ArrowRight => "Right".into(),
        K::Space => "space".into(),
        K::Enter => "Return".into(),
        K::Escape => "Escape".into(),
        K::Tab => "Tab".into(),
        K::Backspace => "BackSpace".into(),
        K::Insert => "Insert".into(),
        K::Delete => "Delete".into(),
        K::Home => "Home".into(),
        K::End => "End".into(),
        K::PageUp => "Prior".into(),
        K::PageDown => "Next".into(),
        K::Minus => "minus".into(),
        K::Equals => "equal".into(),
        K::Comma => "comma".into(),
        K::Period => "period".into(),
        K::Semicolon => "semicolon".into(),
        K::Slash => "slash".into(),
        K::Backslash => "backslash".into(),
        K::OpenBracket => "bracketleft".into(),
        K::CloseBracket => "bracketright".into(),
        K::Backtick => "grave".into(),
        K::Quote => "apostrophe".into(),
        K::MediaPlayPause => "XF86AudioPlay".into(),
        K::MediaTrackNext => "XF86AudioNext".into(),
        K::MediaTrackPrevious => "XF86AudioPrev".into(),
        K::MediaStop => "XF86AudioStop".into(),
        K::AudioVolumeMute => "XF86AudioMute".into(),
        K::AudioVolumeDown => "XF86AudioLowerVolume".into(),
        K::AudioVolumeUp => "XF86AudioRaiseVolume".into(),
        K::Pause => "Pause".into(),
        _ => return None,
    };
    let mut parts = Vec::new();
    for (held, modifier) in [
        (chord.ctrl, "CTRL"),
        (chord.alt, "ALT"),
        (chord.shift, "SHIFT"),
        (chord.command, "LOGO"),
    ] {
        if held {
            parts.push(modifier.to_owned());
        }
    }
    parts.push(name);
    Some(parts.join("+"))
}

/// The system service that delivers background key presses.
pub(crate) enum Backend {
    Hotkeys(Hotkeys),
    #[cfg(target_os = "linux")]
    Portal(async_channel::Sender<Vec<Chord>>),
}

impl std::fmt::Debug for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Hotkeys(_) => "Backend::Hotkeys",
            #[cfg(target_os = "linux")]
            Self::Portal(_) => "Backend::Portal",
        })
    }
}

impl Backend {
    /// Starts the backend for this session. Call it on the thread that runs the window's
    /// event loop.
    pub(crate) fn start(sink: Sink) -> Result<Self, String> {
        #[cfg(target_os = "linux")]
        if std::env::var_os("WAYLAND_DISPLAY").is_some_and(|display| !display.is_empty()) {
            return Ok(Self::Portal(portal::spawn(sink)));
        }
        Hotkeys::start(sink).map(Self::Hotkeys)
    }

    /// Makes `wanted` the chords that fire anywhere. The outcome comes back as signals.
    pub(crate) fn apply(&mut self, wanted: &BTreeSet<Chord>) {
        match self {
            Self::Hotkeys(hotkeys) => hotkeys.apply(wanted),
            #[cfg(target_os = "linux")]
            Self::Portal(requests) => {
                let _ = requests.send_blocking(wanted.iter().copied().collect());
            }
        }
    }
}

/// System hotkeys on Windows, macOS and X11.
pub(crate) struct Hotkeys {
    manager: global_hotkey::GlobalHotKeyManager,
    registered: BTreeMap<Chord, HotKey>,
    /// Hotkey identifiers back to chords, read by the event handler on the platform's
    /// thread.
    chords: Arc<Mutex<BTreeMap<u32, Chord>>>,
    sink: Sink,
}

impl Hotkeys {
    fn start(sink: Sink) -> Result<Self, String> {
        let manager = global_hotkey::GlobalHotKeyManager::new()
            .map_err(|error| format!("Keys cannot be used anywhere on this system: {error}"))?;
        let chords = Arc::new(Mutex::new(BTreeMap::<u32, Chord>::new()));
        let (lookup, events) = (Arc::clone(&chords), Arc::clone(&sink));
        // The handler is process-wide and set once; the desktop starts one backend.
        global_hotkey::GlobalHotKeyEvent::set_event_handler(Some(
            move |event: global_hotkey::GlobalHotKeyEvent| {
                let chord = lookup
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .get(&event.id)
                    .copied();
                if let Some(chord) = chord {
                    events(match event.state {
                        global_hotkey::HotKeyState::Pressed => Signal::Pressed(chord),
                        global_hotkey::HotKeyState::Released => Signal::Released(chord),
                    });
                }
            },
        ));
        Ok(Self {
            manager,
            registered: BTreeMap::new(),
            chords,
            sink,
        })
    }

    fn apply(&mut self, wanted: &BTreeSet<Chord>) {
        let gone: Vec<_> = self
            .registered
            .keys()
            .filter(|chord| !wanted.contains(chord))
            .copied()
            .collect();
        for chord in gone {
            if let Some(hotkey) = self.registered.remove(&chord) {
                let _ = self.manager.unregister(hotkey);
                let _ = self
                    .chords
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(&hotkey.id());
            }
        }
        for chord in wanted {
            if self.registered.contains_key(chord) {
                continue;
            }
            let Some(hotkey) = hotkey(chord) else {
                continue;
            };
            match self.manager.register(hotkey) {
                Ok(()) => {
                    let _ = self
                        .chords
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .insert(hotkey.id(), *chord);
                    let _ = self.registered.insert(*chord, hotkey);
                }
                Err(_) => (self.sink)(Signal::Problem(format!(
                    "{} is taken by the system or another application, so it works only while the editor has focus.",
                    caption(chord)
                ))),
            }
        }
        (self.sink)(Signal::Active(self.registered.keys().copied().collect()));
    }
}

impl Drop for Hotkeys {
    fn drop(&mut self) {
        for hotkey in self.registered.values() {
            let _ = self.manager.unregister(*hotkey);
        }
    }
}

/// The GlobalShortcuts portal on Wayland, on a thread of its own.
#[cfg(target_os = "linux")]
mod portal {
    use super::{Chord, Signal, Sink, caption, xdg_trigger};
    use ashpd::desktop::CreateSessionOptions;
    use ashpd::desktop::global_shortcuts::{BindShortcutsOptions, GlobalShortcuts, NewShortcut};
    use futures_lite::StreamExt as _;
    use std::collections::{BTreeMap, BTreeSet};

    const UNAVAILABLE: &str = "This desktop offers no global shortcuts portal, so keys set to Anywhere work only while the editor has focus.";

    pub(super) fn spawn(sink: Sink) -> async_channel::Sender<Vec<Chord>> {
        let (requests, wanted) = async_channel::unbounded();
        let started = std::thread::Builder::new()
            .name("orange-key-portal".into())
            .spawn({
                let sink = sink.clone();
                move || futures_lite::future::block_on(run(&wanted, &sink))
            });
        if started.is_err() {
            sink(Signal::Problem(UNAVAILABLE.into()));
        }
        requests
    }

    enum Next {
        Wanted(Option<Vec<Chord>>),
        Pressed(Option<String>),
        Released(Option<String>),
    }

    async fn refuse_all(wanted: &async_channel::Receiver<Vec<Chord>>, sink: &Sink) {
        while let Ok(chords) = wanted.recv().await {
            if !chords.is_empty() {
                sink(Signal::Problem(UNAVAILABLE.into()));
            }
            sink(Signal::Active(BTreeSet::new()));
        }
    }

    async fn run(wanted: &async_channel::Receiver<Vec<Chord>>, sink: &Sink) {
        let Ok(portal) = GlobalShortcuts::new().await else {
            return refuse_all(wanted, sink).await;
        };
        let (Ok(mut pressed), Ok(mut released)) = (
            portal.receive_activated().await,
            portal.receive_deactivated().await,
        ) else {
            return refuse_all(wanted, sink).await;
        };
        let mut session = None;
        let mut by_id = BTreeMap::<String, Chord>::new();
        loop {
            let next = futures_lite::future::or(
                async { Next::Wanted(wanted.recv().await.ok()) },
                futures_lite::future::or(
                    async {
                        Next::Pressed(pressed.next().await.map(|a| a.shortcut_id().to_owned()))
                    },
                    async {
                        Next::Released(released.next().await.map(|d| d.shortcut_id().to_owned()))
                    },
                ),
            )
            .await;
            match next {
                Next::Wanted(None) | Next::Pressed(None) | Next::Released(None) => break,
                Next::Pressed(Some(id)) => {
                    if let Some(chord) = by_id.get(&id) {
                        sink(Signal::Pressed(*chord));
                    }
                }
                Next::Released(Some(id)) => {
                    if let Some(chord) = by_id.get(&id) {
                        sink(Signal::Released(*chord));
                    }
                }
                Next::Wanted(Some(chords)) => {
                    // A new session for each set: not every portal lets a session bind twice.
                    if let Some(old) = session.take() {
                        let _ = ashpd::desktop::Session::close(&old).await;
                    }
                    by_id.clear();
                    if chords.is_empty() {
                        sink(Signal::Active(BTreeSet::new()));
                        continue;
                    }
                    let shortcuts: Vec<_> = chords
                        .iter()
                        .filter_map(|chord| {
                            let trigger = xdg_trigger(chord)?;
                            let id = trigger.to_ascii_lowercase();
                            let _ = by_id.insert(id.clone(), *chord);
                            Some(
                                NewShortcut::new(
                                    id,
                                    format!("OrangeEditor: On key pressed {}", caption(chord)),
                                )
                                .preferred_trigger(trigger.as_str()),
                            )
                        })
                        .collect();
                    let Ok(created) = portal.create_session(CreateSessionOptions::default()).await
                    else {
                        sink(Signal::Problem(UNAVAILABLE.into()));
                        sink(Signal::Active(BTreeSet::new()));
                        continue;
                    };
                    let bound = portal
                        .bind_shortcuts(&created, &shortcuts, None, BindShortcutsOptions::default())
                        .await
                        .and_then(|request| request.response());
                    session = Some(created);
                    match bound {
                        Ok(response) => {
                            let active: BTreeSet<Chord> = response
                                .shortcuts()
                                .iter()
                                .filter_map(|shortcut| by_id.get(shortcut.id()).copied())
                                .collect();
                            for chord in chords.iter().filter(|chord| !active.contains(chord)) {
                                sink(Signal::Problem(format!(
                                    "The desktop did not allow {} anywhere, so it works only while the editor has focus.",
                                    caption(chord)
                                )));
                            }
                            sink(Signal::Active(active));
                        }
                        Err(_) => {
                            sink(Signal::Problem(
                                "The desktop did not allow the keys set to Anywhere, so they work only while the editor has focus.".into(),
                            ));
                            sink(Signal::Active(BTreeSet::new()));
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(key: Key, ctrl: bool, alt: bool) -> Chord {
        Chord {
            key,
            ctrl,
            shift: false,
            alt,
            command: false,
        }
    }

    #[test]
    fn only_chords_that_never_swallow_typing_are_taken_anywhere() {
        assert!(check(&chord(Key::F9, false, false)).is_ok());
        assert!(check(&chord(Key::MediaPlayPause, false, false)).is_ok());
        assert!(check(&chord(Key::K, true, false)).is_ok());
        assert!(check(&chord(Key::K, false, true)).is_ok());
        // Typing keys alone, or with Shift alone, would take text from other applications.
        assert!(check(&chord(Key::K, false, false)).is_err());
        assert!(
            check(&Chord {
                shift: true,
                ..chord(Key::K, false, false)
            })
            .is_err()
        );
        assert!(check(&chord(Key::Space, false, false)).is_err());
        assert!(
            check(&chord(Key::F25, true, false)).is_err(),
            "no system code"
        );
        let command = Chord {
            command: true,
            ..chord(Key::K, false, false)
        };
        assert_eq!(check(&command).is_ok(), cfg!(target_os = "macos"));
    }

    #[test]
    fn every_function_key_up_to_f24_and_every_letter_has_a_code() {
        for key in Key::ALL {
            let name = key.name();
            let letter = name.len() == 1 && name.as_bytes()[0].is_ascii_uppercase();
            let function = name
                .strip_prefix('F')
                .and_then(|n| n.parse::<u8>().ok())
                .is_some_and(|n| n <= 24);
            if letter || function {
                assert!(code(*key).is_some(), "{name}");
                assert!(xdg_trigger(&chord(*key, true, false)).is_some(), "{name}");
            }
        }
    }

    #[test]
    fn triggers_and_captions_name_the_modifiers_in_one_order() {
        let shortcut = Chord {
            key: Key::A,
            ctrl: true,
            shift: true,
            alt: true,
            command: false,
        };
        assert_eq!(xdg_trigger(&shortcut).as_deref(), Some("CTRL+ALT+SHIFT+a"));
        assert_eq!(caption(&shortcut), "Ctrl+Alt+Shift+A");
        assert_eq!(
            xdg_trigger(&chord(Key::F9, false, false)).as_deref(),
            Some("F9")
        );
        assert_eq!(
            xdg_trigger(&chord(Key::PageDown, true, false)).as_deref(),
            Some("CTRL+Next")
        );
    }
}
