//! The base audio package: playing sound files on an output device and reacting to the
//! level of an input device.
//!
//! It is an ordinary [`ServicePackage`], built on the plugin SDK alone. An application
//! adds [`package`] to its host; documents then declare `org.orange.audio/output` and
//! `org.orange.audio/input` connections and use the nodes below. An application that
//! does not want audio leaves the package out.
//!
//! Playback reads WAV (integer or float PCM), FLAC, MP3 and Ogg Vorbis files, converts them to the
//! device's rate and channels, and mixes up to [`MAX_VOICES`] sounds per output. A
//! sound plays once or repeats until stopped; an output's sounds can be paused,
//! resumed, moved to a position and scaled by one volume. The device callback only
//! mixes prepared samples. These controls act on every sound of an output connection:
//! sounds that need separate control play on separate connections. Streaming from disk
//! and playlists are not provided.
// Product code reports failures as typed errors; each remaining panic is a documented
// internal invariant (10, "Panic policy").
#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]

mod device;
mod level;
mod mixer;
mod wav;

pub use device::{Device, Devices, NULL_DEVICE, devices};
pub use level::SILENCE_DB;
pub use mixer::MAX_VOICES;
pub use wav::{MAX_FILE_BYTES, MAX_SECONDS};

use device::{Ended, Levels, Output};
use level::Gate;
use orange_sdk::{
    Call, ConnectionAdapter, Definition, Emitter, FaultText, KernelFault, Listeners, Probe,
    ServiceNode, ServicePackage, ServiceSource, Session, SettingField, Value,
};
use serde_json::{Map, Value as Json};
use std::any::Any;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

/// Package namespace of the definitions and adapter kinds here.
pub const PACKAGE: &str = "org.orange.audio";
/// Adapter kind of an output device. Setting: `device`, a [`Device::setting`] from
/// [`devices`] (the endpoint's identifier where the system has one, else its name; a
/// name is still accepted) or [`NULL_DEVICE`]; absent means the system default. The
/// device opens at first use, and again at the next use after it failed or stopped.
pub const OUTPUT_ADAPTER: &str = "org.orange.audio/output";
/// Adapter kind of an input device whose level is measured. Settings: `device` as for
/// an output; `threshold_db` (default -30) and `release_db` (default 6 below the
/// threshold) for [`ON_LEVEL`]; `window_ms` (5 to 1,000, default 50), the length of
/// each measurement; `hold_ms` (0 to 60,000, default 0), how long the level must stay at
/// the threshold or above before the entry fires, counted in whole measurements.
pub const INPUT_ADAPTER: &str = "org.orange.audio/input";
/// Plays a WAV, FLAC, MP3 or Ogg Vorbis file on an output connection and continues when it has ended. Inputs
/// `path` and `gain` (0 to 8; 1 is unchanged).
pub const PLAY: &str = "org.orange.audio/play";
/// Plays one of the sound files of a folder and continues when it has ended. Inputs
/// `folder`, `pick` and `gain`; output `file`, the name of the file played. The
/// folder's sound files are taken in name order and `pick` chooses among them,
/// starting again from the first when it is past the last: a counter makes a playlist
/// and a seeded random number a repeatable shuffle. A folder without sound files is
/// the fault `org.orange.audio/Empty`.
pub const PLAY_FROM: &str = "org.orange.audio/play-from";
/// Stops every sound playing on an output connection.
pub const STOP: &str = "org.orange.audio/stop";
/// Plays a file like [`PLAY`], starting again each time it ends, and continues only
/// when it is stopped.
pub const LOOP: &str = "org.orange.audio/loop";
/// Holds every sound on an output connection where it is.
pub const PAUSE: &str = "org.orange.audio/pause";
/// Lets the paused sounds of an output connection continue.
pub const RESUME: &str = "org.orange.audio/resume";
/// Moves every sound on an output connection to `seconds` from its start (0 to
/// [`MAX_SECONDS`]). A sound shorter than that ends; a repeating one wraps around.
pub const SEEK: &str = "org.orange.audio/seek";
/// Sets the volume of an output connection: `gain` (0 to 8; 1 is unchanged) scales
/// everything it plays, on top of each sound's own gain.
pub const SET_VOLUME: &str = "org.orange.audio/set-volume";
/// Reads the latest measured level of an input connection in dBFS as `level`.
pub const INPUT_LEVEL: &str = "org.orange.audio/input-level";
/// Entry fired when an input connection's level reaches its threshold, with the level
/// as `payload`. It fires again only after the level has fallen to the release level.
pub const ON_LEVEL: &str = "org.orange.audio/on-level";

const DEFAULT_THRESHOLD_DB: f64 = -30.0;
const DEFAULT_HYSTERESIS_DB: f64 = 6.0;
const DEFAULT_WINDOW_MS: u64 = 50;
const POLL: Duration = Duration::from_millis(10);
/// How long an input waits before opening its device again after it failed.
const FIRST_RETRY: Duration = Duration::from_millis(500);
const MAX_RETRY: Duration = Duration::from_secs(5);

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
    "The path, gain or position is not valid.",
);
const FILE: KernelFault = fault(
    "org.orange.audio/File",
    "org.orange.audio/file-unreadable",
    "The sound file is missing or is not a supported WAV, FLAC, MP3 or Ogg Vorbis file.",
);
const TOO_LARGE: KernelFault = fault(
    "org.orange.audio/TooLarge",
    "org.orange.audio/file-too-large",
    "The sound file is too large or too long.",
);
const EMPTY: KernelFault = fault(
    "org.orange.audio/Empty",
    "org.orange.audio/folder-empty",
    "The folder has no sound files.",
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

/// The `device` choices of a connection: the devices of one direction the system lists
/// now, by identifier and shown by name, then [`NULL_DEVICE`]. Listing asks the OS
/// audio host, so it is done only when an editor asks, never on a flow's or the
/// device's thread.
fn device_choices(pick: fn(Devices) -> Vec<Device>) -> Vec<orange_sdk::SettingChoices> {
    let (found, problem) = match devices() {
        Ok(found) => (pick(found), None),
        Err(reason) => (Vec::new(), Some(reason)),
    };
    let mut values: Vec<String> = found.iter().map(|d| d.setting().to_owned()).collect();
    let mut labels: Vec<String> = found.into_iter().map(|device| device.name).collect();
    values.push(NULL_DEVICE.to_owned());
    labels.push("Silent (no device)".to_owned());
    vec![orange_sdk::SettingChoices {
        setting: "device".to_owned(),
        unset: "System default".to_owned(),
        values,
        labels,
        problem,
    }]
}

/// An output connection. The device opens when a node first plays on it, so a document
/// opens on a machine without that device and only the playing node fails. A device
/// that could not be opened, or that failed or stopped since, is opened again by the
/// next node that plays: plugging it in later is enough.
#[derive(Debug)]
struct OutputSession {
    device: Option<String>,
    output: Mutex<Option<Arc<Output>>>,
}
impl OutputSession {
    /// The open output, opening the device first when needed.
    fn output(&self) -> Result<Arc<Output>, KernelFault> {
        let mut held = self.output.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(output) = held.as_ref().filter(|output| !output.broken()) {
            return Ok(Arc::clone(output));
        }
        // A failure is not kept: the next node tries again.
        *held = None;
        let output = Arc::new(Output::open(self.device.as_deref()).map_err(|_| DEVICE)?);
        *held = Some(Arc::clone(&output));
        Ok(output)
    }
    /// The output if the device is open and working, without opening it.
    fn opened(&self) -> Option<Arc<Output>> {
        self.output
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .filter(|output| !output.broken())
            .cloned()
    }
    /// Drops `output` after it stalled, so the next node opens the device again.
    fn forget(&self, output: &Arc<Output>) {
        let mut held = self.output.lock().unwrap_or_else(PoisonError::into_inner);
        if held.as_ref().is_some_and(|kept| Arc::ptr_eq(kept, output)) {
            *held = None;
        }
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
            output: Mutex::new(None),
        }))
    }
    fn choices(&self) -> Vec<orange_sdk::SettingChoices> {
        device_choices(|found| found.outputs)
    }
}

fn output<'a>(call: &Call<'a>) -> Result<&'a OutputSession, KernelFault> {
    call.session
        .and_then(|session| session.as_any().downcast_ref::<OutputSession>())
        .ok_or(NO_SESSION)
}

/// Plays a file once, or again and again until it is stopped.
#[derive(Debug)]
struct Play {
    looped: bool,
}
impl ServiceNode for Play {
    fn run(&self, call: &Call<'_>) -> Result<Vec<Value>, KernelFault> {
        let [Value::Str(path), Value::F64(gain)] = call.inputs else {
            return Err(BAD_INPUT);
        };
        play(call, Path::new(path.as_str()), *gain, self.looped)?;
        Ok(Vec::new())
    }
}

/// Plays a file on the call's output connection and waits for it to end.
fn play(call: &Call<'_>, path: &Path, gain: f64, looped: bool) -> Result<(), KernelFault> {
    let session = output(call)?;
    if path.as_os_str().is_empty() || !(0.0..=8.0).contains(&gain) {
        return Err(BAD_INPUT);
    }
    let output = session.output()?;
    let sound = wav::load(path, output.rate, output.channels).map_err(|error| match error {
        wav::WavError::Unreadable => FILE,
        wav::WavError::TooLarge => TOO_LARGE,
    })?;
    #[expect(clippy::cast_possible_truncation, reason = "gain is within 0..=8")]
    let id = output.play(sound, gain as f32, looped).ok_or(BUSY)?;
    match output.wait(id, || call.lease.is_cancelled()) {
        Ended::Done => Ok(()),
        Ended::Stalled => {
            session.forget(&output);
            Err(DEVICE)
        }
    }
}

/// Most files of a folder considered by [`PLAY_FROM`].
pub const MAX_FOLDER_SOUNDS: usize = 10_000;

/// The sound file `pick` chooses in a folder: its files with a supported extension in
/// name order, counted round and round.
fn choose(folder: &Path, pick: u64) -> Result<std::path::PathBuf, KernelFault> {
    let mut sounds: Vec<_> = std::fs::read_dir(folder)
        .map_err(|_| FILE)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| {
                        ["wav", "flac", "mp3", "ogg"]
                            .iter()
                            .any(|known| extension.eq_ignore_ascii_case(known))
                    })
        })
        .take(MAX_FOLDER_SOUNDS)
        .collect();
    sounds.sort();
    let count = u64::try_from(sounds.len()).unwrap_or(u64::MAX);
    let at = pick.checked_rem(count).ok_or(EMPTY)?;
    Ok(sounds.swap_remove(usize::try_from(at).unwrap_or(0)))
}

/// Plays the file of a folder that a number chooses.
#[derive(Debug)]
struct PlayFrom;
impl ServiceNode for PlayFrom {
    fn run(&self, call: &Call<'_>) -> Result<Vec<Value>, KernelFault> {
        let [Value::Str(folder), Value::U64(pick), Value::F64(gain)] = call.inputs else {
            return Err(BAD_INPUT);
        };
        if folder.is_empty() {
            return Err(BAD_INPUT);
        }
        // The session is checked before the folder is read.
        let _ = output(call)?;
        let path = choose(Path::new(folder.as_str()), *pick)?;
        play(call, &path, *gain, false)?;
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        Ok(vec![Value::Str(Arc::new(name))])
    }
}

#[derive(Debug)]
struct Stop;
impl ServiceNode for Stop {
    fn run(&self, call: &Call<'_>) -> Result<Vec<Value>, KernelFault> {
        // A device nothing has played on has nothing to stop and is not opened for it.
        match output(call)?.opened() {
            Some(output) if !output.stop_all() => Err(DEVICE),
            _ => Ok(Vec::new()),
        }
    }
}

/// What a control node does to the sounds of its output connection.
#[derive(Debug, Clone, Copy)]
enum Control {
    Pause,
    Resume,
    Seek,
    Volume,
}
impl ServiceNode for Control {
    fn run(&self, call: &Call<'_>) -> Result<Vec<Value>, KernelFault> {
        let session = output(call)?;
        let number = |range: std::ops::RangeInclusive<f64>| match call.inputs {
            [Value::F64(number)] if range.contains(number) => Ok(*number),
            _ => Err(BAD_INPUT),
        };
        #[expect(clippy::cast_precision_loss, reason = "a limit of 600 is exact")]
        let position = 0.0..=MAX_SECONDS as f64;
        // A volume is set before anything plays, so it opens the device; the others
        // have nothing to act on while the device is closed.
        let sent = match (self, session.opened()) {
            (Self::Volume, _) => {
                #[expect(clippy::cast_possible_truncation, reason = "gain is within 0..=8")]
                let gain = number(0.0..=8.0)? as f32;
                session.output()?.volume(gain)
            }
            (Self::Seek, opened) => {
                let seconds = number(position)?;
                match opened {
                    Some(output) => output.seek(seconds),
                    _ => true,
                }
            }
            (Self::Pause, Some(output)) => output.pause(true),
            (Self::Resume, Some(output)) => output.pause(false),
            (Self::Pause | Self::Resume, _) => true,
        };
        if sent { Ok(Vec::new()) } else { Err(DEVICE) }
    }
}

struct InputShared {
    device: Option<String>,
    window: Duration,
    gate: Gate,
    levels: Arc<Levels>,
    listeners: Listeners,
    /// Set once a flow has attached its triggers; the device is not opened before.
    started: AtomicBool,
    stop: AtomicBool,
    /// Why the device could not be opened the last time it was tried; cleared once it
    /// opens.
    failed: Mutex<Option<String>>,
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

impl InputShared {
    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }
    fn failure(&self) -> Option<String> {
        self.failed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
    fn set_failure(&self, failure: Option<String>) {
        *self.failed.lock().unwrap_or_else(PoisonError::into_inner) = failure;
    }
    /// Waits `time`, or less when the session ends.
    fn rest(&self, time: Duration) {
        let mut waited = Duration::ZERO;
        while waited < time && !self.stopped() {
            thread::sleep(POLL);
            waited += POLL;
        }
    }
}

/// Owns the capture stream and turns threshold crossings into entry firings. A device
/// that cannot be opened, or that fails while open, is opened again with backoff, so a
/// microphone plugged in later is heard.
fn listen(shared: &InputShared) {
    while !shared.started.load(Ordering::Acquire) {
        if shared.stopped() {
            return;
        }
        thread::sleep(POLL);
    }
    let mut retry = FIRST_RETRY;
    let mut seen = shared.levels.fired().0;
    while !shared.stopped() {
        let _ = shared.levels.take_broken();
        let stream = device::open_input(
            shared.device.as_deref(),
            shared.window,
            shared.gate,
            Arc::clone(&shared.levels),
        );
        let stream = match stream {
            Ok(stream) => stream,
            Err(reason) => {
                shared.set_failure(Some(reason));
                shared.rest(retry);
                retry = (retry * 2).min(MAX_RETRY);
                continue;
            }
        };
        shared.set_failure(None);
        retry = FIRST_RETRY;
        while !shared.stopped() {
            if shared.levels.take_broken() {
                shared.set_failure(Some("the input device stopped".to_owned()));
                break;
            }
            let (count, level) = shared.levels.fired();
            if count != seen {
                // Crossings closer together than one poll are reported as one.
                seen = count;
                let payload = Value::F64(level);
                shared.listeners.emit(|()| Some(payload.clone()));
            }
            thread::sleep(POLL);
        }
        drop(stream);
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

/// Starts an input's listening thread from its settings; it waits for `started`.
fn spawn_input(settings: &Map<String, Json>) -> Result<Arc<InputShared>, String> {
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
    let hold = match settings.get("hold_ms") {
        None => 0,
        Some(value) => value
            .as_u64()
            .filter(|ms| *ms <= 60_000)
            .ok_or("`hold_ms` must be an integer from 0 to 60000")?,
    };
    let hold = u32::try_from(hold.div_ceil(window)).unwrap_or(u32::MAX);
    let shared = Arc::new(InputShared {
        device: device_setting(settings)?,
        window: Duration::from_millis(window),
        gate: Gate::held(threshold, release, hold),
        levels: Arc::new(Levels::new()),
        listeners: Listeners::new(),
        started: AtomicBool::new(false),
        stop: AtomicBool::new(false),
        failed: Mutex::new(None),
    });
    let worker = Arc::clone(&shared);
    let _ = thread::Builder::new()
        .name("orange-audio-input".into())
        .spawn(move || listen(&worker))
        .map_err(|error| error.to_string())?;
    Ok(shared)
}

/// The level an input device hears now, measured apart from every flow while an editor
/// shows it. Dropping it closes the device.
struct InputProbe(Arc<InputShared>);
impl Probe for InputProbe {
    fn read(&self) -> Result<f64, String> {
        match self.0.failure() {
            Some(reason) => Err(reason),
            None => Ok(self.0.levels.latest()),
        }
    }
}
impl Drop for InputProbe {
    fn drop(&mut self) {
        self.0.stop.store(true, Ordering::Release);
    }
}

#[derive(Debug)]
struct InputAdapter;
impl ConnectionAdapter for InputAdapter {
    fn open(&self, settings: &Map<String, Json>) -> Result<Arc<dyn Session>, String> {
        Ok(Arc::new(InputSession(spawn_input(settings)?)))
    }
    fn choices(&self) -> Vec<orange_sdk::SettingChoices> {
        device_choices(|found| found.inputs)
    }
    fn fields(&self) -> Vec<SettingField> {
        let field = |setting: &str, label: &str, help: &str, unit: &str| SettingField {
            setting: setting.to_owned(),
            label: label.to_owned(),
            help: help.to_owned(),
            unit: unit.to_owned(),
            min: crate::SILENCE_DB,
            max: 0.0,
            default: None,
            whole: false,
        };
        vec![
            SettingField {
                default: Some(DEFAULT_THRESHOLD_DB),
                ..field(
                    "threshold_db",
                    "Threshold",
                    "The level at which the trigger fires.",
                    "dBFS",
                )
            },
            field(
                "release_db",
                "Release",
                "The level the sound must fall to before the trigger can fire again; 6 below the threshold when left out.",
                "dBFS",
            ),
            SettingField {
                min: 0.0,
                max: 60_000.0,
                default: Some(0.0),
                whole: true,
                ..field(
                    "hold_ms",
                    "Hold",
                    "How long the sound must stay at the threshold before the trigger fires.",
                    "ms",
                )
            },
            SettingField {
                min: 5.0,
                max: 1_000.0,
                default: Some(50.0),
                whole: true,
                ..field(
                    "window_ms",
                    "Window",
                    "The length of each measurement.",
                    "ms",
                )
            },
        ]
    }
    fn probe(&self, settings: &Map<String, Json>) -> Option<Result<Box<dyn Probe>, String>> {
        Some(spawn_input(settings).map(|shared| {
            shared.started.store(true, Ordering::Release);
            Box::new(InputProbe(shared)) as Box<dyn Probe>
        }))
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
        if shared.failure().is_some() {
            return Err(DEVICE);
        }
        Ok(vec![Value::F64(shared.levels.latest())])
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
        Ok(Listeners::listen(
            Arc::clone(shared),
            |shared| &shared.listeners,
            (),
            emitter,
        ))
    }
}

/// Gain 1, the sound as it is, for a `gain` input left empty.
fn unit_gain() -> Json {
    serde_json::json!({"type": "core/f64@1", "bits": format!("{:016x}", 1.0f64.to_bits())})
}

fn play_definition() -> String {
    Definition::node(crate::PACKAGE, "play", "Play sound")
        .description("Plays a sound file on an output device and continues when it has ended.")
        .category("audio", "Audio")
        .uses(OUTPUT_ADAPTER)
        .input("path", "core/string@1")
        .path_input("path", false, SOUND_FILES)
        .input_default("gain", "core/f64@1", unit_gain())
        .build()
}

/// The file name extensions of the sound files played.
const SOUND_FILES: &[&str] = &["wav", "flac", "mp3", "ogg"];

fn play_from_definition() -> String {
    Definition::node(crate::PACKAGE, "play-from", "Play sound from folder")
        .description(
            "Plays one sound file of a folder, in name order, that `pick` chooses: a counter makes a playlist, and Random u64 (org.orange.math/random-u64) wired into `pick` plays one at random. Without `pick` it plays the first.",
        )
        .category("audio", "Audio")
        .uses(OUTPUT_ADAPTER)
        .input("folder", "core/string@1")
        .path_input("folder", true, &[])
        .input_default("pick", "core/u64@1", serde_json::json!({"type": "core/u64@1", "value": "0"}))
        .input_default("gain", "core/f64@1", unit_gain())
        .output("file", "core/string@1")
        .build()
}

fn loop_definition() -> String {
    Definition::node(crate::PACKAGE, "loop", "Loop sound")
        .description("Plays a sound file again and again on an output device until it is stopped.")
        .category("audio", "Audio")
        .uses(OUTPUT_ADAPTER)
        .input("path", "core/string@1")
        .path_input("path", false, SOUND_FILES)
        .input_default("gain", "core/f64@1", unit_gain())
        .build()
}

fn pause_definition() -> String {
    Definition::node(crate::PACKAGE, "pause", "Pause sounds")
        .description("Holds every sound playing on an output device where it is.")
        .category("audio", "Audio")
        .uses(OUTPUT_ADAPTER)
        .build()
}

fn resume_definition() -> String {
    Definition::node(crate::PACKAGE, "resume", "Resume sounds")
        .description("Lets the paused sounds of an output device continue.")
        .category("audio", "Audio")
        .uses(OUTPUT_ADAPTER)
        .build()
}

fn seek_definition() -> String {
    Definition::node(crate::PACKAGE, "seek", "Seek sounds")
        .description("Moves every sound playing on an output device to a position in seconds.")
        .category("audio", "Audio")
        .uses(OUTPUT_ADAPTER)
        .input("seconds", "core/f64@1")
        .build()
}

fn set_volume_definition() -> String {
    Definition::node(crate::PACKAGE, "set-volume", "Set volume")
        .description("Sets the volume of everything an output device plays.")
        .category("audio", "Audio")
        .uses(OUTPUT_ADAPTER)
        .input("gain", "core/f64@1")
        .build()
}

fn stop_definition() -> String {
    Definition::node(crate::PACKAGE, "stop", "Stop sounds")
        .description("Stops every sound playing on an output device.")
        .category("audio", "Audio")
        .uses(OUTPUT_ADAPTER)
        .build()
}

fn input_level_definition() -> String {
    Definition::node(crate::PACKAGE, "input-level", "Microphone level")
        .description("Reads how loud the microphone, or another input device, is now, in dBFS.")
        .category("audio", "Audio")
        .uses(INPUT_ADAPTER)
        .output("level", "core/f64@1")
        .build()
}

fn on_level_definition() -> String {
    Definition::entry(crate::PACKAGE, "on-level", "When the microphone hears sound")
        .description("Fires when the microphone, or another input device, gets as loud as its connection's threshold.")
        .category("audio", "Audio")
        .uses(INPUT_ADAPTER)
        .payload("core/f64@1")
        .build()
}

/// The package to pass to `Host::with_services`.
pub fn package() -> ServicePackage {
    let package = ServicePackage::new(PACKAGE, env!("CARGO_PKG_VERSION"))
        .about(
            "Audio",
            "Plays sound files on an output device and listens to a microphone or other input device.",
        )
        .adapter(OUTPUT_ADAPTER, OutputAdapter)
        .adapter(INPUT_ADAPTER, InputAdapter)
        .node(&play_definition(), Play { looped: false })
        .and_then(|package| package.node(&loop_definition(), Play { looped: true }))
        .and_then(|package| package.node(&play_from_definition(), PlayFrom))
        .and_then(|package| package.node(&stop_definition(), Stop))
        .and_then(|package| package.node(&pause_definition(), Control::Pause))
        .and_then(|package| package.node(&resume_definition(), Control::Resume))
        .and_then(|package| package.node(&seek_definition(), Control::Seek))
        .and_then(|package| package.node(&set_volume_definition(), Control::Volume))
        .and_then(|package| package.node(&input_level_definition(), InputLevel))
        .and_then(|package| package.source(&on_level_definition(), OnLevel));
    match package {
        Ok(package) => package,
        // The definitions are constants of this crate, checked by its tests.
        Err(error) => unreachable!("built-in definition is valid: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_number_chooses_among_a_folders_sound_files_in_name_order() {
        let folder = std::env::temp_dir().join(format!("orange-audio-pick-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(folder.join("sub.wav")).unwrap();
        assert_eq!(choose(&folder, 0), Err(EMPTY));
        for name in ["b.WAV", "a.ogg", "notes.txt", "c.mp3"] {
            std::fs::write(folder.join(name), b"").unwrap();
        }
        let chosen = |pick| {
            let path = choose(&folder, pick).unwrap();
            path.file_name().unwrap().to_string_lossy().into_owned()
        };
        // Text files and folders are not sounds; the count goes round.
        assert_eq!(
            [chosen(0), chosen(1), chosen(2), chosen(3), chosen(u64::MAX)],
            ["a.ogg", "b.WAV", "c.mp3", "a.ogg", "a.ogg"]
        );
        assert_eq!(choose(&folder.join("missing"), 0), Err(FILE));
        std::fs::remove_dir_all(&folder).unwrap();
    }

    fn settings(json: Json) -> Map<String, Json> {
        json.as_object().unwrap().clone()
    }

    #[test]
    fn the_package_builds_with_its_nodes_and_two_connection_kinds() {
        let package = package();
        assert_eq!(package.id(), PACKAGE);
        assert_eq!(package.nodes().len(), 9);
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
            serde_json::json!({"device": NULL_DEVICE, "hold_ms": -1}),
            serde_json::json!({"device": NULL_DEVICE, "hold_ms": 60_001}),
        ] {
            assert!(InputAdapter.open(&settings(bad)).is_err());
        }
    }

    #[test]
    fn an_output_that_failed_is_opened_again_at_the_next_use() {
        // A device that cannot be opened is not remembered as failed.
        let missing = OutputSession {
            device: Some("no such device here".into()),
            output: Mutex::new(None),
        };
        assert_eq!(missing.output().err(), Some(DEVICE));
        assert!(missing.output.lock().unwrap().is_none());

        let silent = OutputSession {
            device: Some(NULL_DEVICE.into()),
            output: Mutex::new(None),
        };
        assert!(silent.opened().is_none());
        let first = silent.output().unwrap();
        assert!(Arc::ptr_eq(&first, &silent.output().unwrap()));
        // One whose stream reported an error is replaced, not reused.
        first.fail();
        assert!(silent.opened().is_none());
        let second = silent.output().unwrap();
        assert!(!Arc::ptr_eq(&first, &second));
        // One that stalled is forgotten.
        silent.forget(&second);
        assert!(silent.opened().is_none());
    }

    #[test]
    fn an_input_offers_its_trigger_settings_and_a_live_level() {
        let fields = InputAdapter.fields();
        let names: Vec<_> = fields.iter().map(|field| field.setting.as_str()).collect();
        assert_eq!(
            names,
            ["threshold_db", "release_db", "hold_ms", "window_ms"]
        );
        assert_eq!(fields[0].default, Some(DEFAULT_THRESHOLD_DB));
        assert!(fields[2].whole && fields[3].whole);

        // The silent device hears nothing, and says so as silence.
        let probe = InputAdapter
            .probe(&settings(serde_json::json!({"device": NULL_DEVICE})))
            .unwrap()
            .unwrap();
        thread::sleep(POLL * 3);
        assert_eq!(probe.read(), Ok(SILENCE_DB));
        drop(probe);
        // Settings that cannot work are refused before anything opens.
        let bad = settings(serde_json::json!({"device": NULL_DEVICE, "window_ms": 1}));
        assert!(InputAdapter.probe(&bad).unwrap().is_err());

        // A device that is not there is reported, and tried again.
        let missing = InputAdapter
            .probe(&settings(
                serde_json::json!({"device": "no such device here"}),
            ))
            .unwrap()
            .unwrap();
        let started = std::time::Instant::now();
        while missing.read().is_ok() && started.elapsed() < Duration::from_secs(10) {
            thread::sleep(POLL);
        }
        assert!(missing.read().is_err());
    }

    #[test]
    fn a_device_is_named_by_its_identifier_where_the_system_has_one() {
        let named = Device {
            id: None,
            name: "Speakers".into(),
        };
        let identified = Device {
            id: Some("wasapi:{0.0.0.00000000}.{1234}".into()),
            name: "Speakers".into(),
        };
        assert_eq!(named.setting(), "Speakers");
        assert_eq!(identified.setting(), "wasapi:{0.0.0.00000000}.{1234}");
        // Choices always end with the silent device and label every value.
        let choices = OutputAdapter.choices();
        assert_eq!(
            choices[0].values.last().map(String::as_str),
            Some(NULL_DEVICE)
        );
        assert_eq!(choices[0].values.len(), choices[0].labels.len());
    }

    #[test]
    fn sound_inputs_offer_a_file_or_folder_chooser() {
        let (_, play) = orange_sdk::NodeDefinition::described(&play_definition()).unwrap();
        assert_eq!(play.paths[0].port, "path");
        assert!(!play.paths[0].folder && play.paths[0].extensions.contains(&"ogg".to_owned()));
        let (_, from) = orange_sdk::NodeDefinition::described(&play_from_definition()).unwrap();
        assert!(from.paths[0].folder);
    }

    /// Needs a microphone, so it is not part of the normal run:
    /// `cargo test -p orange-audio -- --ignored`.
    #[test]
    #[ignore = "needs an audio input device"]
    fn the_default_microphone_is_measured_live() {
        let probe = InputAdapter.probe(&Map::new()).unwrap().unwrap();
        let mut loudest = SILENCE_DB;
        for _ in 0..20 {
            thread::sleep(Duration::from_millis(100));
            loudest = loudest.max(probe.read().unwrap());
        }
        // A muted microphone hears digital silence, which is still a reading.
        println!("loudest {loudest:.1} dBFS");
        assert!((SILENCE_DB..=0.0).contains(&loudest));
    }
}
