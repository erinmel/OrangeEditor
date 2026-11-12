//! The base audio package: playing sound files on an output device and reacting to the
//! level of an input device.
//!
//! It is an ordinary [`ServicePackage`], built on the plugin SDK alone. An application
//! adds [`package`] to its host; documents then declare `org.orange.audio/output` and
//! `org.orange.audio/input` connections and use the nodes below. An application that
//! does not want audio leaves the package out.
//!
//! Playback reads WAV files (integer or float PCM), converts them to the device's rate
//! and channels, and mixes up to [`MAX_VOICES`] sounds per output. The device callback
//! only mixes prepared samples. Other file formats, seeking,
//! looping and per-sound routing are not provided.
// Product code reports failures as typed errors; each remaining panic is a documented
// internal invariant (10, "Panic policy").
#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]

mod device;
mod level;
mod mixer;
mod wav;

pub use device::{Devices, NULL_DEVICE, devices};
pub use level::SILENCE_DB;
pub use mixer::MAX_VOICES;
pub use wav::{MAX_FILE_BYTES, MAX_SECONDS};

use device::{Ended, Levels, Output};
use level::Gate;
use orange_sdk::{
    Call, ConnectionAdapter, EmitError, Emitter, FaultText, KernelFault, ServiceNode,
    ServicePackage, ServiceSource, Session, Value,
};
use serde_json::{Map, Value as Json};
use std::any::Any;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::thread;
use std::time::Duration;

/// Package namespace of the definitions and adapter kinds here.
pub const PACKAGE: &str = "org.orange.audio";
/// Adapter kind of an output device. Setting: `device`, a name from [`devices`] or
/// [`NULL_DEVICE`]; absent means the system default. The device opens at first use.
pub const OUTPUT_ADAPTER: &str = "org.orange.audio/output";
/// Adapter kind of an input device whose level is measured. Settings: `device` as for
/// an output; `threshold_db` (default -30) and `release_db` (default 6 below the
/// threshold) for [`ON_LEVEL`]; `window_ms` (5 to 1,000, default 50), the length of
/// each measurement.
pub const INPUT_ADAPTER: &str = "org.orange.audio/input";
/// Plays a WAV file on an output connection and continues when it has ended. Inputs
/// `path` and `gain` (0 to 8; 1 is unchanged).
pub const PLAY: &str = "org.orange.audio/play";
/// Stops every sound playing on an output connection.
pub const STOP: &str = "org.orange.audio/stop";
/// Reads the latest measured level of an input connection in dBFS as `level`.
pub const INPUT_LEVEL: &str = "org.orange.audio/input-level";
/// Entry fired when an input connection's level reaches its threshold, with the level
/// as `payload`. It fires again only after the level has fallen to the release level.
pub const ON_LEVEL: &str = "org.orange.audio/on-level";

const DEFAULT_THRESHOLD_DB: f64 = -30.0;
const DEFAULT_HYSTERESIS_DB: f64 = 6.0;
const DEFAULT_WINDOW_MS: u64 = 50;
const POLL: Duration = Duration::from_millis(10);

const fn fault(code: &'static str, key: &'static str, message: &'static str) -> KernelFault {
    KernelFault {
        code: FaultText::Static(code),
        message_key: FaultText::Static(key),
        message,
    }
}
const NO_SESSION: KernelFault = fault(
    "org.orange.audio/Connection",
    "org.orange.audio/connection-kind",
    "The node needs an audio connection of the right kind.",
);
const DEVICE: KernelFault = fault(
    "org.orange.audio/Device",
    "org.orange.audio/device-unavailable",
    "The audio device could not be opened or has stopped.",
);
const BAD_INPUT: KernelFault = fault(
    "org.orange.audio/Input",
    "org.orange.audio/input-invalid",
    "The path or gain is not valid.",
);
const FILE: KernelFault = fault(
    "org.orange.audio/File",
    "org.orange.audio/file-unreadable",
    "The sound file is missing or is not a supported WAV file.",
);
const TOO_LARGE: KernelFault = fault(
    "org.orange.audio/TooLarge",
    "org.orange.audio/file-too-large",
    "The sound file is too large or too long.",
);
const BUSY: KernelFault = fault(
    "org.orange.audio/Busy",
    "org.orange.audio/too-many-sounds",
    "Too many sounds are already playing on the device.",
);

fn device_setting(settings: &Map<String, Json>) -> Result<Option<String>, String> {
    match settings.get("device") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::String(name)) if !name.is_empty() => Ok(Some(name.clone())),
        Some(_) => Err("`device` must be a device name".into()),
    }
}

/// An output connection. The device opens when a node first plays on it, so a document
/// opens on a machine without that device and only the playing node fails.
#[derive(Debug)]
struct OutputSession {
    device: Option<String>,
    output: OnceLock<Result<Output, String>>,
}
impl OutputSession {
    fn output(&self) -> Result<&Output, KernelFault> {
        self.output
            .get_or_init(|| Output::open(self.device.as_deref()))
            .as_ref()
            .map_err(|_| DEVICE)
    }
}
impl Session for OutputSession {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Debug)]
struct OutputAdapter;
impl ConnectionAdapter for OutputAdapter {
    fn open(&self, settings: &Map<String, Json>) -> Result<Arc<dyn Session>, String> {
        Ok(Arc::new(OutputSession {
            device: device_setting(settings)?,
            output: OnceLock::new(),
        }))
    }
}

fn output<'a>(call: &Call<'a>) -> Result<&'a OutputSession, KernelFault> {
    call.session
        .and_then(|session| session.as_any().downcast_ref::<OutputSession>())
        .ok_or(NO_SESSION)
}

#[derive(Debug)]
struct Play;
impl ServiceNode for Play {
    fn run(&self, call: &Call<'_>) -> Result<Vec<Value>, KernelFault> {
        let session = output(call)?;
        let [Value::Str(path), Value::F64(gain)] = call.inputs else {
            return Err(BAD_INPUT);
        };
        if path.is_empty() || !(0.0..=8.0).contains(gain) {
            return Err(BAD_INPUT);
        }
        let output = session.output()?;
        let sound =
            wav::load(Path::new(path.as_str()), output.rate, output.channels).map_err(|error| {
                match error {
                    wav::WavError::Unreadable => FILE,
                    wav::WavError::TooLarge => TOO_LARGE,
                }
            })?;
        let samples = sound.len();
        #[expect(clippy::cast_possible_truncation, reason = "gain is within 0..=8")]
        let id = output.play(sound, *gain as f32).ok_or(BUSY)?;
        match output.wait(id, samples, || call.lease.is_cancelled()) {
            Ended::Done => Ok(Vec::new()),
            Ended::Stalled => Err(DEVICE),
        }
    }
}

#[derive(Debug)]
struct Stop;
impl ServiceNode for Stop {
    fn run(&self, call: &Call<'_>) -> Result<Vec<Value>, KernelFault> {
        // A device nothing has played on has nothing to stop and is not opened for it.
        match output(call)?.output.get() {
            Some(Ok(output)) if !output.stop_all() => Err(DEVICE),
            _ => Ok(Vec::new()),
        }
    }
}

struct InputShared {
    device: Option<String>,
    window: Duration,
    gate: Gate,
    levels: Arc<Levels>,
    listeners: Mutex<Vec<(u64, Emitter)>>,
    next_listener: AtomicU64,
    /// Set once a flow has attached its triggers; the device is not opened before.
    started: AtomicBool,
    stop: AtomicBool,
    /// Why the device could not be opened, once that is known.
    failed: OnceLock<String>,
}
impl std::fmt::Debug for InputShared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InputShared")
            .field("device", &self.device)
            .field("window", &self.window)
            .finish_non_exhaustive()
    }
}

/// An input connection. Dropping the last reference closes the device.
#[derive(Debug)]
struct InputSession(Arc<InputShared>);
impl Session for InputSession {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn start(&self) {
        self.0.started.store(true, Ordering::Release);
    }
}
impl Drop for InputSession {
    fn drop(&mut self) {
        self.0.stop.store(true, Ordering::Release);
    }
}

/// Owns the capture stream and turns threshold crossings into entry firings.
fn listen(shared: &InputShared) {
    while !shared.started.load(Ordering::Acquire) {
        if shared.stop.load(Ordering::Acquire) {
            return;
        }
        thread::sleep(POLL);
    }
    let stream = device::open_input(
        shared.device.as_deref(),
        shared.window,
        shared.gate,
        Arc::clone(&shared.levels),
    );
    let _stream = match stream {
        Ok(stream) => stream,
        Err(reason) => {
            let _ = shared.failed.set(reason);
            return;
        }
    };
    let mut seen = shared.levels.fired().0;
    while !shared.stop.load(Ordering::Acquire) {
        let (count, level) = shared.levels.fired();
        if count != seen {
            // Crossings closer together than one poll are reported as one.
            seen = count;
            let payload = Value::F64(level);
            shared
                .listeners
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .retain(|(_, emitter)| {
                    emitter.emit(Some(payload.clone())) != Err(EmitError::Closed)
                });
        }
        thread::sleep(POLL);
    }
}

fn number(settings: &Map<String, Json>, name: &str) -> Result<Option<f64>, String> {
    match settings.get(name) {
        None => Ok(None),
        Some(value) => value
            .as_f64()
            .filter(|number| number.is_finite())
            .map(Some)
            .ok_or_else(|| format!("`{name}` must be a number")),
    }
}

#[derive(Debug)]
struct InputAdapter;
impl ConnectionAdapter for InputAdapter {
    fn open(&self, settings: &Map<String, Json>) -> Result<Arc<dyn Session>, String> {
        let threshold = number(settings, "threshold_db")?.unwrap_or(DEFAULT_THRESHOLD_DB);
        let release = number(settings, "release_db")?.unwrap_or(threshold - DEFAULT_HYSTERESIS_DB);
        if release > threshold {
            return Err("`release_db` must not be above `threshold_db`".into());
        }
        let window = match settings.get("window_ms") {
            None => DEFAULT_WINDOW_MS,
            Some(value) => value
                .as_u64()
                .filter(|ms| (5..=1_000).contains(ms))
                .ok_or("`window_ms` must be an integer from 5 to 1000")?,
        };
        let shared = Arc::new(InputShared {
            device: device_setting(settings)?,
            window: Duration::from_millis(window),
            gate: Gate::new(threshold, release),
            levels: Arc::new(Levels::new()),
            listeners: Mutex::new(Vec::new()),
            next_listener: AtomicU64::new(0),
            started: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            failed: OnceLock::new(),
        });
        let worker = Arc::clone(&shared);
        let _ = thread::Builder::new()
            .name("orange-audio-input".into())
            .spawn(move || listen(&worker))
            .map_err(|error| error.to_string())?;
        Ok(Arc::new(InputSession(shared)))
    }
}

fn input(session: Option<&dyn Session>) -> Option<&Arc<InputShared>> {
    session?
        .as_any()
        .downcast_ref::<InputSession>()
        .map(|session| &session.0)
}

#[derive(Debug)]
struct InputLevel;
impl ServiceNode for InputLevel {
    fn run(&self, call: &Call<'_>) -> Result<Vec<Value>, KernelFault> {
        let shared = input(call.session).ok_or(NO_SESSION)?;
        if shared.failed.get().is_some() {
            return Err(DEVICE);
        }
        Ok(vec![Value::F64(shared.levels.latest())])
    }
}

/// Removes one entry's emitter when its flow lets go of it.
struct Listening(Arc<InputShared>, u64);
impl Drop for Listening {
    fn drop(&mut self) {
        self.0
            .listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|(id, _)| *id != self.1);
    }
}

#[derive(Debug)]
struct OnLevel;
impl ServiceSource for OnLevel {
    fn start(
        &self,
        session: Option<Arc<dyn Session>>,
        emitter: Emitter,
    ) -> Result<Box<dyn Send>, String> {
        let shared =
            input(session.as_deref()).ok_or("the entry needs an audio input connection")?;
        let id = shared.next_listener.fetch_add(1, Ordering::Relaxed);
        shared
            .listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((id, emitter));
        Ok(Box::new(Listening(Arc::clone(shared), id)))
    }
}

const PLAY_DEFINITION: &str = r#"{
  "definition_id": "org.orange.audio/play",
  "label": {"key": "org.orange.audio/play-label", "fallback": "Play sound"},
  "description": {"key": "org.orange.audio/play-description",
    "fallback": "Plays a WAV file on an output device and continues when it has ended."},
  "editor": {"category_id": "org.orange.audio/category-audio",
    "category": {"key": "org.orange.audio/category-audio", "fallback": "Audio"}},
  "definition_version": 1,
  "config_version": 1,
  "kind": "async",
  "suspends": "always",
  "determinism": "external",
  "connection": true,
  "ports": [
    {"port_id": "path", "direction": "input", "kind": "data", "type": "core/string@1"},
    {"port_id": "gain", "direction": "input", "kind": "data", "type": "core/f64@1"}
  ]
}"#;

const STOP_DEFINITION: &str = r#"{
  "definition_id": "org.orange.audio/stop",
  "label": {"key": "org.orange.audio/stop-label", "fallback": "Stop sounds"},
  "description": {"key": "org.orange.audio/stop-description",
    "fallback": "Stops every sound playing on an output device."},
  "editor": {"category_id": "org.orange.audio/category-audio",
    "category": {"key": "org.orange.audio/category-audio", "fallback": "Audio"}},
  "definition_version": 1,
  "config_version": 1,
  "kind": "async",
  "suspends": "always",
  "determinism": "external",
  "connection": true,
  "ports": []
}"#;

const INPUT_LEVEL_DEFINITION: &str = r#"{
  "definition_id": "org.orange.audio/input-level",
  "label": {"key": "org.orange.audio/input-level-label", "fallback": "Input level"},
  "description": {"key": "org.orange.audio/input-level-description",
    "fallback": "Reads the latest level of an input device in dBFS."},
  "editor": {"category_id": "org.orange.audio/category-audio",
    "category": {"key": "org.orange.audio/category-audio", "fallback": "Audio"}},
  "definition_version": 1,
  "config_version": 1,
  "kind": "async",
  "suspends": "always",
  "determinism": "external",
  "connection": true,
  "ports": [
    {"port_id": "level", "direction": "output", "kind": "data", "type": "core/f64@1",
      "source": "execution", "available_on": ["then"]}
  ]
}"#;

const ON_LEVEL_DEFINITION: &str = r#"{
  "definition_id": "org.orange.audio/on-level",
  "label": {"key": "org.orange.audio/on-level-label", "fallback": "On input level"},
  "description": {"key": "org.orange.audio/on-level-description",
    "fallback": "Fires when an input device's level reaches the connection's threshold."},
  "editor": {"category_id": "org.orange.audio/category-audio",
    "category": {"key": "org.orange.audio/category-audio", "fallback": "Audio"}},
  "definition_version": 1,
  "config_version": 1,
  "kind": "entry",
  "determinism": "external",
  "connection": true,
  "ports": [
    {"port_id": "payload", "direction": "output", "kind": "data", "type": "core/f64@1",
      "source": "payload"}
  ]
}"#;

/// The package to pass to `Host::with_services`.
pub fn package() -> ServicePackage {
    let package = ServicePackage::new(PACKAGE, env!("CARGO_PKG_VERSION"))
        .adapter(OUTPUT_ADAPTER, OutputAdapter)
        .adapter(INPUT_ADAPTER, InputAdapter)
        .node(PLAY_DEFINITION, Play)
        .and_then(|package| package.node(STOP_DEFINITION, Stop))
        .and_then(|package| package.node(INPUT_LEVEL_DEFINITION, InputLevel))
        .and_then(|package| package.source(ON_LEVEL_DEFINITION, OnLevel));
    match package {
        Ok(package) => package,
        // The definitions are constants of this crate, checked by its tests.
        Err(error) => unreachable!("built-in definition is valid: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(json: Json) -> Map<String, Json> {
        json.as_object().unwrap().clone()
    }

    #[test]
    fn the_package_builds_with_four_nodes_and_two_connection_kinds() {
        let package = package();
        assert_eq!(package.id(), PACKAGE);
        assert_eq!(package.nodes().len(), 3);
        assert_eq!(package.sources().len(), 1);
        assert_eq!(package.adapters().len(), 2);
    }

    #[test]
    fn connection_settings_are_checked_when_a_document_opens() {
        assert!(OutputAdapter.open(&settings(serde_json::json!({}))).is_ok());
        assert!(
            OutputAdapter
                .open(&settings(serde_json::json!({"device": 3})))
                .is_err()
        );
        let null = serde_json::json!({"device": NULL_DEVICE});
        assert!(InputAdapter.open(&settings(null)).is_ok());
        for bad in [
            serde_json::json!({"device": NULL_DEVICE, "threshold_db": "loud"}),
            serde_json::json!({"device": NULL_DEVICE, "threshold_db": -40, "release_db": -30}),
            serde_json::json!({"device": NULL_DEVICE, "window_ms": 2}),
        ] {
            assert!(InputAdapter.open(&settings(bad)).is_err());
        }
    }
}
