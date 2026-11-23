//! The base audio package: playing sound files on an output device and reacting to the
//! level of an input device.
//!
//! It is an ordinary [`ServicePackage`], built on the plugin SDK alone. An application
//! adds [`package`] to its host; documents then declare `org.orange.audio/output` and
//! `org.orange.audio/input` connections and use the nodes below. An application that
//! does not want audio leaves the package out.
//!
//! Playback reads WAV, AIFF, FLAC, MP3, Ogg Vorbis, M4A and Opus files, converts
//! them to the device's rate and channels, and mixes up to
//! [`MAX_VOICES`] sounds per output. A sound plays once or repeats until stopped;
//! an output's sounds can be paused,
//! resumed, moved to a position and scaled by one volume. The device callback only
//! mixes prepared samples. These controls act on every sound of an output connection:
//! sounds that need separate control play on separate connections. File-backed
//! audio sources can also receive progressive HTTP(S) responses.
//!
//! [`AUDIO_SOURCE`] publishes an audio stream ([`STREAM_TYPE`]) from a device or
//! sound file. [`LIVE_LEVEL`], [`MULTIBAND_SPECTRUM`], [`NOISE_GATE`], [`TRANSIENT`], [`AUDIO_OUT`]
//! and [`RECORD`] take the stream as an input. The value names the capture but
//! carries no samples. Samples stay in a bounded ring that each consumer reads at
//! its own pace (see `stream`).
//!
//! Analyzers carry their own band or window and read with their own cursor on a
//! worker. The audio callback only measures and stores.
// Product code reports failures as typed errors; each remaining panic is a documented
// internal invariant (10, "Panic policy").
#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]

mod analysis;
include!(concat!(env!("OUT_DIR"), "/locales.rs"));
mod device;
mod file_source;
mod level;
mod live_nodes;
mod mixer;
mod processors;
mod reactive;
mod spectrum;
mod stream;
mod wav;

pub use device::{Device, Devices, LOOPBACK_OFFERED, LOOPBACK_PREFIX, NULL_DEVICE, devices};
pub use level::SILENCE_DB;
pub use live_nodes::{
    AUDIO_FORMAT, AUDIO_SOURCE, COMPRESSOR, FILTER, FORMAT_TYPE, GAIN, LIMITER, LIVE_LEVEL,
    MULTIBAND_SPECTRUM, NOISE_GATE, NOISE_LEVEL, SOUND_CONTROL, SOURCE_CONTROL, SOURCE_FINISHED,
    TRANSIENT,
};
pub use mixer::MAX_VOICES;
pub use stream::{MAX_LAG_MS, RING_SECONDS, START_MS};
pub use wav::{MAX_FILE_BYTES, MAX_SECONDS};

use analysis::{BandMeter, Listening, Onsets};
use device::{Ended, Levels, Output};
use orange_sdk::{
    Call, ConnectionAdapter, Definition, EmitError, Emitter, KernelFault, Lifecycle, Listeners,
    Probe, RecordRepr, ServiceNode, ServicePackage, ServiceSource, Session, Value,
};
use serde_json::{Map, Value as Json};
use std::any::Any;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::thread;
use std::time::{Duration, Instant};
use stream::{Cursor, Ring};

/// Package namespace of the definitions and adapter kinds here.
pub const PACKAGE: &str = "org.orange.audio";
/// Adapter kind of an output device. Setting: `device`, a [`Device::setting`] from
/// [`devices`] (the endpoint's identifier where the system has one, else its name; a
/// name is still accepted) or [`NULL_DEVICE`]; absent means the system default. The
/// device opens at first use, and again at the next use after it failed or stopped.
pub const OUTPUT_ADAPTER: &str = "org.orange.audio/output";
/// Adapter kind of an audio input: a device, what an output plays, one application or a
/// sound file, as [`AUDIO_SOURCE`] gives it. Settings: `device` as for an output, or
/// `file`; `window_ms` (5 to 1,000, default 50), the length of each level measurement.
pub const INPUT_ADAPTER: &str = "org.orange.audio/input";
/// Plays a WAV, AIFF, FLAC, MP3, Ogg Vorbis or M4A file on an output connection.
/// Execution continues when playback ends. Inputs
/// `path` and `gain` (0 to 8; 1 is unchanged).
pub const PLAY: &str = "org.orange.audio/play";
/// Plays one of the sound files of a folder and continues when it has ended. Inputs
/// `folder`, `pick` and `gain`; output `file`, the name of the file played. The
/// folder's sound files are taken in name order and `pick` chooses among them,
/// starting again from the first when it is past the last: a counter makes a playlist
/// and a seeded random number a repeatable shuffle. A folder without sound files is
/// the fault `org.orange.audio/Empty`.
pub const PLAY_FROM: &str = "org.orange.audio/play-from";
/// Plays a file like [`PLAY`], starting again each time it ends, and continues only
/// when it is stopped.
pub const LOOP: &str = "org.orange.audio/loop";
/// The type of an audio stream: a record whose one field, `source`, names a live
/// input. It holds no samples, so copying it is free and every consumer reads the
/// same sound; it stops working when its input connection closes.
pub const STREAM_TYPE: &str = "org.orange.audio/stream@1";
/// Plays an audio `stream` on an output connection at `gain` (0 to 8), converted to
/// the output's rate and channels, until the output's sounds are stopped, the run
/// stops it or the stream's input closes. Streams played on one output are mixed.
pub const AUDIO_OUT: &str = "org.orange.audio/audio-out";
/// Records `seconds` (more than 0, up to [`MAX_RECORD_SECONDS`]) of an audio `stream`
/// to a 16-bit PCM WAV file at `path`, at the stream's own rate and channels, and
/// gives the seconds recorded as `recorded`. Stopping the run ends the file early, still valid.
pub const RECORD: &str = "org.orange.audio/record";
/// Longest recording [`RECORD`] makes.
pub const MAX_RECORD_SECONDS: f64 = 3_600.0;
/// How long [`AUDIO_SOURCE`] and [`RECORD`] wait for an input device to open.
pub const OPEN_WAIT: Duration = Duration::from_secs(3);

const DEFAULT_WINDOW_MS: u64 = 50;
const POLL: Duration = Duration::from_millis(10);
/// How often an open input checks that its device still works.
const DEVICE_CHECK: Duration = Duration::from_millis(100);
/// How long an input waits before opening its device again after it failed.
const FIRST_RETRY: Duration = Duration::from_millis(500);
const MAX_RETRY: Duration = Duration::from_secs(5);

const NO_SESSION: KernelFault = KernelFault::from_static(
    "org.orange.audio/Connection",
    "org.orange.audio/connection-kind",
    "The node needs an audio connection of the right kind.",
);
const DEVICE: KernelFault = KernelFault::from_static(
    "org.orange.audio/Device",
    "org.orange.audio/device-unavailable",
    "The audio device could not be opened or has stopped.",
);
const BAD_INPUT: KernelFault = KernelFault::from_static(
    "org.orange.audio/Input",
    "org.orange.audio/input-invalid",
    "The path, gain or position is not valid.",
);
const FILE: KernelFault = KernelFault::from_static(
    "org.orange.audio/File",
    "org.orange.audio/file-unreadable",
    "The sound file is missing or uses an unsupported audio format.",
);
const TOO_LARGE: KernelFault = KernelFault::from_static(
    "org.orange.audio/TooLarge",
    "org.orange.audio/file-too-large",
    "The sound file is too large or too long.",
);
const EMPTY: KernelFault = KernelFault::from_static(
    "org.orange.audio/Empty",
    "org.orange.audio/folder-empty",
    "The folder has no sound files.",
);
const BUSY: KernelFault = KernelFault::from_static(
    "org.orange.audio/Busy",
    "org.orange.audio/too-many-sounds",
    "Too many sounds are already playing on the device.",
);
const NOT_FILE: KernelFault = KernelFault::from_static(
    "org.orange.audio/NotFile",
    "org.orange.audio/source-not-file",
    "The node controls a sound file source, and this source is a device.",
);
const ENDED: KernelFault = KernelFault::from_static(
    "org.orange.audio/Ended",
    "org.orange.audio/stream-ended",
    "The audio stream's input is closed.",
);
const WRITE: KernelFault = KernelFault::from_static(
    "org.orange.audio/Write",
    "org.orange.audio/file-unwritable",
    "The recording could not be written to the file.",
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
fn device_choices(
    pick: fn(Devices) -> Vec<(String, String)>,
    include_apps: bool,
) -> Vec<orange_sdk::SettingChoices> {
    let (mut found, mut problem) = match devices() {
        Ok(found) => (pick(found), None),
        Err(reason) => (Vec::new(), Some(reason)),
    };
    if include_apps {
        match device::apps::choices() {
            Ok(apps) => found.extend(apps),
            Err(reason) => {
                problem = Some(problem.map_or_else(
                    || reason.to_owned(),
                    |problem| format!("{problem}\n{reason}"),
                ));
            }
        }
    }
    let (mut values, mut labels): (Vec<String>, Vec<String>) = found.into_iter().unzip();
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

/// A listed device as a choice: the value its setting holds and its name.
fn choice(device: Device) -> (String, String) {
    (
        device.setting().to_owned(),
        device::apps::device_label(&device.name),
    )
}

/// The `device` choices of an input: its capture devices, then what each output device
/// plays where the system offers that.
fn input_choices(found: Devices) -> Vec<(String, String)> {
    #[cfg(windows)]
    let loopbacks = found.outputs.into_iter().map(|device| {
        (
            format!("{LOOPBACK_PREFIX}{}", device.setting()),
            format!(
                "{} (what it plays)",
                device::apps::device_label(&device.name)
            ),
        )
    });
    #[cfg(not(windows))]
    let loopbacks = std::iter::empty();
    found
        .inputs
        .into_iter()
        .map(choice)
        .chain(loopbacks)
        .collect()
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
    fn about(&self) -> orange_sdk::AdapterAbout {
        orange_sdk::AdapterAbout {
            label: "Audio output".into(),
            purpose: "Plays sounds on a speaker or other output device.".into(),
        }
    }
    fn choices(&self) -> Vec<orange_sdk::SettingChoices> {
        device_choices(
            |found| found.outputs.into_iter().map(choice).collect(),
            false,
        )
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
                        SOUND_FILES
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
    streams: Mutex<BTreeMap<u64, Weak<RecordRepr>>>,
    /// What a stream value names this input by (see [`SOURCES`]).
    id: u64,
    /// The ring of the device while it is open.
    ring: Mutex<Option<Arc<Ring>>>,
    device: Option<String>,
    window: Duration,
    levels: Arc<Levels>,
    /// The sound file this source plays instead of capturing a device.
    file: Option<file_source::FileSettings>,
    /// Where a file source is; unused for a device.
    transport: Mutex<file_source::Transport>,
    /// On finished entries of a file source.
    finished: Listeners,
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

/// The inputs open in this process, by the ID their stream values carry. An entry goes
/// when its input is dropped, so a stream of a closed input is refused, not reused.
static SOURCES: Mutex<BTreeMap<u64, Weak<InputShared>>> = Mutex::new(BTreeMap::new());
static NEXT_SOURCE: AtomicU64 = AtomicU64::new(0);

/// A new input's ID. The first is taken from the clock, so a stream value kept from an
/// earlier run of the application names no input of this one instead of another.
fn next_source() -> u64 {
    let _ = NEXT_SOURCE.compare_exchange(
        0,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |since| u64::try_from(since.as_nanos()).unwrap_or(1) | 1),
        Ordering::Relaxed,
        Ordering::Relaxed,
    );
    NEXT_SOURCE.fetch_add(1, Ordering::Relaxed)
}

impl Drop for InputShared {
    fn drop(&mut self) {
        let _ = SOURCES
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.id);
    }
}

/// The stream value of an input.
fn stream_value(input: &InputShared, run_id: u64) -> Result<Value, KernelFault> {
    let mut streams = input.streams.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(record) = streams.get(&run_id).and_then(Weak::upgrade) {
        return Ok(Value::Record(record));
    }
    streams.retain(|_, record| record.strong_count() > 0);
    let record = Arc::new(RecordRepr {
        ty: STREAM_TYPE.parse().map_err(|_| BAD_INPUT)?,
        fields: Box::new([Some(Value::U64(input.id))]),
    });
    let _ = streams.insert(run_id, Arc::downgrade(&record));
    Ok(Value::Record(record))
}

/// The input a stream value names, while it is open.
fn source(value: &Value, run_id: u64) -> Result<Arc<InputShared>, KernelFault> {
    let Value::Record(record) = value else {
        return Err(BAD_INPUT);
    };
    let [Some(Value::U64(id))] = &record.fields[..] else {
        return Err(BAD_INPUT);
    };
    if record.ty.as_str() != STREAM_TYPE {
        return Err(BAD_INPUT);
    }
    let source = SOURCES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(id)
        .and_then(Weak::upgrade);
    source
        .filter(|source| {
            !source.stopped()
                && source
                    .streams
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .get(&run_id)
                    .and_then(Weak::upgrade)
                    .is_some_and(|issued| Arc::ptr_eq(record, &issued))
        })
        .ok_or(ENDED)
}

impl InputShared {
    fn register(
        file: Option<file_source::FileSettings>,
        device: Option<String>,
        window: Duration,
    ) -> Arc<Self> {
        let shared = Arc::new(Self {
            streams: Mutex::new(BTreeMap::new()),
            id: next_source(),
            ring: Mutex::new(None),
            file,
            device,
            window,
            transport: Mutex::new(file_source::Transport::default()),
            finished: Listeners::new(),
            levels: Arc::new(Levels::new()),
            started: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            failed: Mutex::new(None),
        });
        SOURCES
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(shared.id, Arc::downgrade(&shared));
        shared
    }

    fn replace_ring(&self, ring: Option<Arc<Ring>>) {
        let previous = std::mem::replace(
            &mut *self.ring.lock().unwrap_or_else(PoisonError::into_inner),
            ring,
        );
        if let Some(previous) = previous {
            previous.close();
        }
    }
    /// The device's ring once it is open and capturing, opening it first when no flow
    /// has started the input yet. Waits up to `patience` (for ever when `None`) while
    /// the device is not open, and gives up when `cancelled`. An input that cannot be
    /// opened is the fault `Device` once it has been tried; a closed connection is
    /// `Ended`.
    fn wait_ring(
        &self,
        patience: Option<Duration>,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Arc<Ring>, KernelFault> {
        self.started.store(true, Ordering::Release);
        let started = Instant::now();
        loop {
            if self.stopped() || cancelled() {
                return Err(ENDED);
            }
            if let Some(ring) = self.current_ring() {
                return Ok(ring);
            }
            if patience.is_some_and(|patience| started.elapsed() >= patience)
                || (patience.is_some() && self.failure().is_some())
            {
                return Err(DEVICE);
            }
            thread::sleep(POLL);
        }
    }
    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }
    fn current_ring(&self) -> Option<Arc<Ring>> {
        self.ring
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .filter(|ring| !ring.closed())
            .cloned()
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
            thread::sleep(POLL.min(time - waited));
            waited += POLL;
        }
    }
    /// [`rest`](Self::rest), then whether the session is still open.
    fn rest_ended(&self, time: Duration) -> bool {
        self.rest(time);
        !self.stopped()
    }
    fn transport(&self) -> std::sync::MutexGuard<'_, file_source::Transport> {
        self.transport
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// Owns the capture stream; analyzers read its ring. A device that cannot be opened, or
/// that fails while open, is opened again with backoff, so a microphone plugged in
/// later is heard.
fn listen(shared: &Arc<InputShared>) {
    while !shared.started.load(Ordering::Acquire) {
        if shared.stopped() {
            return;
        }
        thread::sleep(POLL);
    }
    if let Some(file) = &shared.file {
        file_source::run(shared, file);
        return;
    }
    let mut retry = FIRST_RETRY;
    while !shared.stopped() {
        let _ = shared.levels.take_broken();
        let stream = device::open_input(
            shared.device.as_deref(),
            shared.window,
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
        *shared.ring.lock().unwrap_or_else(PoisonError::into_inner) =
            Some(Arc::clone(stream.ring()));
        shared.set_failure(None);
        retry = FIRST_RETRY;
        while !shared.stopped() {
            if shared.levels.take_broken() {
                shared.set_failure(Some("the input device stopped".to_owned()));
                break;
            }
            shared.rest(DEVICE_CHECK);
        }
        *shared.ring.lock().unwrap_or_else(PoisonError::into_inner) = None;
        // Closes the ring: its readers end or wait for the next one.
        drop(stream);
    }
}

/// Starts an input's listening thread from its settings; it waits for `started`. The
/// device is the only setting that matters: thresholds belong to the entries that
/// listen (On band level, On transient). `window_ms` sets the level meter's window.
fn spawn_input(settings: &Map<String, Json>) -> Result<Arc<InputShared>, String> {
    let window = match settings.get("window_ms") {
        None => DEFAULT_WINDOW_MS,
        Some(value) => value
            .as_u64()
            .filter(|ms| (5..=1_000).contains(ms))
            .ok_or("`window_ms` must be an integer from 5 to 1000")?,
    };
    let shared = InputShared::register(
        file_source::FileSettings::of(settings)?,
        device_setting(settings)?,
        Duration::from_millis(window),
    );
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
    fn about(&self) -> orange_sdk::AdapterAbout {
        orange_sdk::AdapterAbout {
            label: "Audio source".into(),
            purpose: "A microphone, system or app audio where available, or a sound file the graph plays and listens to.".into(),
        }
    }
    fn texts(&self) -> Vec<orange_sdk::SettingText> {
        vec![orange_sdk::SettingText {
            setting: "file".into(),
            label: "Sound file".into(),
            help: "Enter a local sound file or an HTTP(S) audio URL.".into(),
            hint: "C:/Music/track.ogg".into(),
            secret: false,
            required: false,
            list: false,
        }]
    }
    fn toggles(&self) -> Vec<orange_sdk::SettingToggle> {
        let toggle = |setting: &str, label: &str, help: &str| orange_sdk::SettingToggle {
            setting: setting.into(),
            label: label.into(),
            help: help.into(),
            default: false,
        };
        vec![
            toggle(
                "loop",
                "Loop",
                "A file starts again at its end until it is stopped.",
            ),
            toggle(
                "autoplay",
                "Play at start",
                "A file plays as soon as a flow uses it, without a Play node.",
            ),
            toggle(
                "monitor",
                "Hear it",
                "A file is also played on the default output; otherwise it is only listened to.",
            ),
        ]
    }
    fn choices(&self) -> Vec<orange_sdk::SettingChoices> {
        device_choices(input_choices, true)
    }
    fn probes(&self) -> bool {
        true
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

/// What an analyzer entry does with each window's band level: fire or not.
trait Detector: Clone + Send + 'static {
    fn step(&mut self, level: f64) -> bool;
}
impl Detector for Onsets {
    fn step(&mut self, level: f64) -> bool {
        Onsets::step(self, level)
    }
}

/// Stops an analyzer's thread when its flow drops it.
struct Analyzing(Arc<Lifecycle>);
impl Drop for Analyzing {
    fn drop(&mut self) {
        self.0.stop();
    }
}

/// Sends each measured window and signals gaps with `None`. Returns a cancellation
/// guard; every reader follows the same connection to a replacement capture ring.
fn watch_windows(
    input: Arc<InputShared>,
    listening: Listening,
    observe: impl FnMut(Option<f64>) -> bool + Send + 'static,
) -> Result<Box<dyn Send>, String> {
    watch_window_measurements(
        input,
        listening.window,
        move |ring| BandMeter::new(ring, &listening),
        |meter, samples, report| meter.push(samples, report),
        observe,
    )
}

fn watch_window_measurements<M, T>(
    input: Arc<InputShared>,
    window: Duration,
    make: impl Fn(&Ring) -> M + Send + 'static,
    push: impl Fn(&mut M, &[f32], &mut dyn FnMut(T)) + Send + 'static,
    mut observe: impl FnMut(Option<T>) -> bool + Send + 'static,
) -> Result<Box<dyn Send>, String> {
    let life = Arc::new(Lifecycle::new());
    let running = Arc::clone(&life);
    thread::Builder::new()
        .name("orange-audio-analyze".into())
        .spawn(move || {
            let mut block = vec![0.0f32; 8_192];
            loop {
                let Ok(ring) = input.wait_ring(None, &|| running.stopped()) else {
                    return;
                };
                let mut meter = make(&ring);
                let mut cursor = Cursor::new(Arc::clone(&ring));
                // Read about four times per window: often enough to fire within one.
                let pace = (window / 4).clamp(Duration::from_millis(2), POLL);
                while !running.stopped() {
                    let previous_dropped = cursor.dropped;
                    let read = cursor.read(&mut block);
                    if read == 0 {
                        if ring.closed() {
                            break;
                        }
                        let _ = running.rest(pace);
                        continue;
                    }
                    if previous_dropped != cursor.dropped {
                        meter = make(&ring);
                        if !observe(None) {
                            return;
                        }
                    }
                    let mut active = true;
                    push(&mut meter, &block[..read], &mut |level| {
                        active = active && observe(Some(level))
                    });
                    if !active {
                        return;
                    }
                }
                if !running.stopped() && !observe(None) {
                    return;
                }
            }
        })
        .map_err(|error| error.to_string())?;
    Ok(Box::new(Analyzing(life)))
}

/// Fires the entry for each qualifying window. An overwritten or replaced ring
/// cannot carry a hold or an onset detector's state across a missing interval.
fn analyze(
    input: Arc<InputShared>,
    listening: Listening,
    initial: impl Detector,
    emitter: Emitter,
) -> Result<Box<dyn Send>, String> {
    let mut detector = initial.clone();
    watch_windows(input, listening, move |window| {
        if let Some(level) = window {
            !detector.step(level) || emitter.emit(Some(Value::F64(level))) != Err(EmitError::Closed)
        } else {
            detector = initial.clone();
            true
        }
    })
}

fn input_of(session: Option<&Arc<dyn Session>>) -> Result<Arc<InputShared>, String> {
    input(session.map(AsRef::as_ref))
        .cloned()
        .ok_or_else(|| "the entry needs an audio input connection".to_owned())
}

fn transient(
    input: Arc<InputShared>,
    settings: &Map<String, Json>,
    emitter: Emitter,
) -> Result<Box<dyn Send>, String> {
    let listening = Listening::of(settings, (45.0, 120.0, 10.0))?;
    let sensitivity = analysis::number(settings, "sensitivity", 0.5)?;
    let floor = analysis::number(settings, "floor_db", -50.0)?;
    let gap_ms = analysis::number(settings, "min_gap_ms", 120.0)?;
    if !(0.0..=1.0).contains(&sensitivity) || !(0.0..=10_000.0).contains(&gap_ms) {
        return Err("`sensitivity` is from 0 to 1 and `min_gap_ms` from 0 to 10000".into());
    }
    let window_ms = listening.window.as_secs_f64() * 1_000.0;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "checked settings"
    )]
    let gap = (gap_ms / window_ms).ceil() as u32;
    analyze(
        input,
        listening,
        Onsets::new(floor, sensitivity, gap),
        emitter,
    )
}

/// What a transport node does to its file source.
#[derive(Debug, Clone, Copy)]
enum SourceControl {
    Play,
    Pause,
    Resume,
    Stop,
    Seek,
}
impl ServiceNode for SourceControl {
    fn run(&self, call: &Call<'_>) -> Result<Vec<Value>, KernelFault> {
        let shared = input(call.session).ok_or(NO_SESSION)?;
        self.apply(shared, call.inputs)
    }
}

impl SourceControl {
    fn apply(self, shared: &InputShared, inputs: &[Value]) -> Result<Vec<Value>, KernelFault> {
        if shared.file.is_none() {
            return Err(NOT_FILE);
        }
        let seconds = match (self, inputs) {
            (Self::Seek, [Value::F64(seconds)]) if (0.0..=MAX_RECORD_SECONDS).contains(seconds) => {
                *seconds
            }
            (Self::Seek, _) => return Err(BAD_INPUT),
            _ => 0.0,
        };
        // Using the source starts its thread, as a flow does for an entry.
        shared.started.store(true, Ordering::Release);
        let mut transport = shared.transport();
        match self {
            Self::Play => transport.play(),
            Self::Pause => transport.pause(),
            Self::Resume => transport.resume(),
            Self::Stop => transport.stop(),
            Self::Seek => {
                let (rate, channels) = shared
                    .current_ring()
                    .map_or((file_source::RATE, file_source::CHANNELS), |ring| {
                        (ring.rate, ring.channels)
                    });
                transport.seek(seconds, rate, channels);
            }
        }
        Ok(Vec::new())
    }
}

fn when_finished(shared: Arc<InputShared>, emitter: Emitter) -> Result<Box<dyn Send>, String> {
    if shared.file.is_none() {
        return Err(
            "Connect Source finished to a Sound source set to a file or an HTTP audio URL.".into(),
        );
    }
    Ok(Listeners::listen(
        shared,
        |shared| &shared.finished,
        (),
        emitter,
    ))
}

fn stream_gain(gain: f64) -> Result<f32, KernelFault> {
    if !(0.0..=8.0).contains(&gain) {
        return Err(BAD_INPUT);
    }
    #[expect(clippy::cast_possible_truncation, reason = "gain is within 0..=8")]
    let gain = gain as f32;
    Ok(gain)
}

fn play_stream(
    session: &OutputSession,
    source: &InputShared,
    gain: f32,
    cancelled: impl Fn() -> bool,
    mut playing: impl FnMut(bool),
) -> Result<(), KernelFault> {
    loop {
        let ring = match source.wait_ring(None, &cancelled) {
            Ok(ring) => ring,
            Err(fault) if fault == ENDED => return Ok(()),
            Err(fault) => return Err(fault),
        };
        let output = session.output()?;
        if cancelled() {
            return Ok(());
        }
        let id = output.play_live(Arc::clone(&ring), gain).ok_or(BUSY)?;
        playing(true);
        let outcome = output.wait(id, &cancelled);
        playing(false);
        match outcome {
            Ended::Stalled => {
                session.forget(&output);
                return Err(DEVICE);
            }
            Ended::Done if cancelled() || !ring.closed() => return Ok(()),
            // The input device closed; follow it to the next one.
            Ended::Done => {}
        }
    }
}

/// Samples taken from the ring per write to the file.
const RECORD_BLOCK: usize = 8_192;

#[derive(Debug)]
struct Record;
impl ServiceNode for Record {
    fn run(&self, call: &Call<'_>) -> Result<Vec<Value>, KernelFault> {
        let [stream, Value::Str(path), Value::F64(seconds)] = call.inputs else {
            return Err(BAD_INPUT);
        };
        if path.is_empty() || !(*seconds > 0.0 && *seconds <= MAX_RECORD_SECONDS) {
            return Err(BAD_INPUT);
        }
        let source = source(stream, call.lease.run_id())?;
        let cancelled = || call.lease.is_cancelled();
        let ring = source.wait_ring(Some(OPEN_WAIT), &cancelled)?;
        record(
            Cursor::new(ring),
            Path::new(path.as_str()),
            *seconds,
            &cancelled,
        )
        .map(|recorded| vec![Value::F64(recorded)])
    }
}

/// Writes `seconds` of what `cursor` reads to a WAV file and returns the seconds
/// written. Ends early, with a valid file, when `cancelled`; a device that stops
/// mid-way leaves a valid file of what was heard and is the fault `Device`.
fn record(
    mut cursor: Cursor,
    path: &Path,
    seconds: f64,
    cancelled: &dyn Fn() -> bool,
) -> Result<f64, KernelFault> {
    let (rate, channels) = (cursor.ring().rate, cursor.ring().channels);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "at most an hour of frames"
    )]
    let total = (seconds * f64::from(rate)) as u64 * u64::from(channels);
    let spec = hound::WavSpec {
        channels,
        sample_rate: rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).map_err(|_| WRITE)?;
    let mut block = vec![0.0f32; RECORD_BLOCK / usize::from(channels) * usize::from(channels)];
    let mut done = 0u64;
    let mut stopped = false;
    while done < total && !cancelled() {
        let want = usize::try_from(total - done).map_or(block.len(), |left| left.min(block.len()));
        let Some(buffer) = block.get_mut(..want) else {
            break;
        };
        let read = cursor.read(buffer);
        for sample in buffer.iter().take(read) {
            #[expect(clippy::cast_possible_truncation, reason = "clamped to full scale")]
            let pcm = (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;
            writer.write_sample(pcm).map_err(|_| WRITE)?;
        }
        done += read as u64;
        if read == 0 {
            if cursor.ring().closed() {
                stopped = true;
                break;
            }
            thread::sleep(POLL);
        }
    }
    writer.finalize().map_err(|_| WRITE)?;
    if stopped {
        return Err(DEVICE);
    }
    #[expect(clippy::cast_precision_loss, reason = "at most an hour of frames")]
    Ok(done as f64 / f64::from(channels) / f64::from(rate))
}

/// Gain 1, the sound as it is, for a `gain` input left empty.
fn unit_gain() -> Json {
    serde_json::json!({"type": "core/f64@1", "bits": format!("{:016x}", 1.0f64.to_bits())})
}

fn play_definition() -> String {
    Definition::node(PACKAGE, "play", "Play sound")
        .keywords("sound, music, speaker, song")
        .description("Plays a sound file on an output device and continues when it has ended.")
        .category("audio", "Audio")
        .uses(OUTPUT_ADAPTER)
        .input("path", "core/string@1")
        .path_input("path", false, SOUND_FILES)
        .input_default("gain", "core/f64@1", unit_gain())
        .build()
}

/// The file name extensions of the sound files played.
const SOUND_FILES: &[&str] = &[
    "wav", "flac", "mp3", "ogg", "m4a", "aif", "aiff", "opus", "webm",
];

fn play_from_definition() -> String {
    Definition::node(PACKAGE, "play-from", "Play sound from folder")
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
    Definition::node(PACKAGE, "loop", "Loop sound")
        .description("Repeats the sound file until Sound control sends Stop to the same output. The next step runs after the loop stops.")
        .category("audio", "Audio")
        .uses(OUTPUT_ADAPTER)
        .input("path", "core/string@1")
        .path_input("path", false, SOUND_FILES)
        .input_default("gain", "core/f64@1", unit_gain())
        .build()
}

/// A number setting with its limits and the value used when it is left out.
fn number_setting(minimum: f64, maximum: f64, default: f64) -> Json {
    serde_json::json!({"type": "number", "minimum": minimum, "maximum": maximum, "default": default})
}

/// The band and window settings every analyzer entry has.
fn band_settings(definition: Definition, low: f64, high: f64, window: f64) -> Definition {
    definition
        .setting("low_hz", number_setting(0.0, 24_000.0, low))
        .setting("high_hz", number_setting(0.0, 24_000.0, high))
        .setting("window_ms", number_setting(5.0, 1_000.0, window))
}

fn transient_settings(definition: Definition) -> Definition {
    band_settings(definition, 45.0, 120.0, 10.0)
        .setting("sensitivity", number_setting(0.0, 1.0, 0.5))
        .setting("floor_db", number_setting(SILENCE_DB, 0.0, -50.0))
        .setting("min_gap_ms", number_setting(0.0, 10_000.0, 120.0))
}

fn stream_input(definition: Definition) -> Definition {
    definition.input("stream", STREAM_TYPE)
}

fn record_definition() -> String {
    stream_input(
        Definition::node(PACKAGE, "record", "Record to file")
            .description("Records an audio stream to a WAV file for a number of seconds, then continues with the seconds recorded. Stopping the run ends the file early.")
            .category("audio", "Audio"),
    )
    .input("path", "core/string@1")
    .save_path_input("path", &["wav"])
    .input_default(
        "seconds",
        "core/f64@1",
        serde_json::json!({"type": "core/f64@1", "bits": format!("{:016x}", 10.0f64.to_bits())}),
    )
    .output("recorded", "core/f64@1")
    .build()
}

/// The package to pass to `Host::with_services`.
pub fn package() -> ServicePackage {
    let package = ServicePackage::new(PACKAGE, env!("CARGO_PKG_VERSION"))
        .about(
            "Audio",
            "Plays sound files and live audio on output devices, and listens to, measures and records microphones and other inputs.",
        )
        .with_catalogs(CATALOGS)
        .adapter(OUTPUT_ADAPTER, OutputAdapter)
        .adapter(INPUT_ADAPTER, InputAdapter)
        .record("stream", &[("source", "core/u64@1")])
        .node(&play_definition(), Play { looped: false })
        .and_then(|package| package.node(&loop_definition(), Play { looped: true }))
        .and_then(|package| package.node(&play_from_definition(), PlayFrom))
        .and_then(|package| package.node(&record_definition(), Record))
        .and_then(live_nodes::add);
    match package {
        Ok(package) => package,
        // The definitions are constants of this crate, checked by its tests.
        Err(error) => unreachable!("built-in definition is valid: {}", error),
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
        for name in ["b.WAV", "a.ogg", "notes.txt", "c.mp3", "d.M4A", "e.AIF"] {
            std::fs::write(folder.join(name), b"").unwrap();
        }
        let chosen = |pick| {
            let path = choose(&folder, pick).unwrap();
            path.file_name().unwrap().to_string_lossy().into_owned()
        };
        // Text files and folders are not sounds; the count goes round.
        assert_eq!(
            (0..6).map(chosen).collect::<Vec<_>>(),
            ["a.ogg", "b.WAV", "c.mp3", "d.M4A", "e.AIF", "a.ogg"]
        );
        assert_eq!(chosen(u64::MAX), "a.ogg");
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
        // Play, Loop, Play from folder, Record, Source control, Sound control.
        assert_eq!(package.nodes().len(), 6);
        // Audio out, Frequency Splitter, Audio Level Trigger, Transient, Source finished.
        assert_eq!(package.sources().len(), 5);
        // Audio source, Noise Gate, Compressor, Limiter, Gain, Filter, Audio format,
        // Level.
        assert_eq!(package.lives().len(), 8);
        assert_eq!(package.replacements().len(), 1);
        assert_eq!(package.adapters().len(), 2);
        assert_eq!(package.types().len(), 5);
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
        // Thresholds belong to the entries now; an old connection that still names
        // them opens, and a bad meter window is still refused.
        let old = serde_json::json!({"device": NULL_DEVICE, "threshold_db": -40});
        assert!(InputAdapter.open(&settings(old)).is_ok());
        let bad = serde_json::json!({"device": NULL_DEVICE, "window_ms": 2});
        assert!(InputAdapter.open(&settings(bad)).is_err());
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
    fn an_input_offers_a_live_level_and_no_thresholds() {
        // Thresholds are settings of each listening entry, not of the device.
        assert!(InputAdapter.fields().is_empty());

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
        let started = Instant::now();
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
        assert!(!play.paths[0].folder);
        assert!(play.paths[0].extensions.contains(&"ogg".to_owned()));
        assert!(play.paths[0].extensions.contains(&"m4a".to_owned()));
        assert!(play.paths[0].extensions.contains(&"aif".to_owned()));
        assert!(play.paths[0].extensions.contains(&"aiff".to_owned()));
        let (_, from) = orange_sdk::NodeDefinition::described(&play_from_definition()).unwrap();
        assert!(from.paths[0].folder);
    }
    /// An input on the test tone: a square wave of amplitude 0.5 (-6.02 dBFS) at
    /// 48 kHz in two channels, measured in 10 ms windows.
    fn tone_input() -> InputSession {
        InputSession(
            spawn_input(&settings(
                serde_json::json!({"device": device::TONE_DEVICE, "window_ms": 10}),
            ))
            .unwrap(),
        )
    }

    #[test]
    fn a_stream_names_its_input_until_the_input_closes() {
        let input = tone_input();
        let value = stream_value(&input.0, 10).unwrap();
        assert!(Arc::ptr_eq(&source(&value, 10).unwrap(), &input.0));
        assert_eq!(source(&value, 11).err(), Some(ENDED));
        let Value::Record(record) = &value else {
            unreachable!()
        };
        let restored = Value::Record(Arc::new(RecordRepr {
            ty: record.ty.clone(),
            fields: record.fields.clone(),
        }));
        assert_eq!(source(&restored, 10).err(), Some(ENDED));
        let ring = input.0.wait_ring(Some(OPEN_WAIT), &|| false).unwrap();
        assert_eq!((ring.rate, ring.channels), (48_000, 2));
        let shared = Arc::downgrade(&input.0);
        drop(input);
        // The listening thread lets go, the ring closes and the stream is refused.
        let started = Instant::now();
        while shared.upgrade().is_some() && started.elapsed() < Duration::from_secs(5) {
            thread::sleep(POLL);
        }
        assert!(ring.closed());
        assert_eq!(source(&value, 10).err(), Some(ENDED));
        assert_eq!(source(&Value::U64(1), 10).err(), Some(BAD_INPUT));
    }

    #[test]
    fn an_input_stream_passes_through_to_an_output_and_its_level_is_measured() {
        let input = tone_input();
        let ring = input.0.wait_ring(Some(OPEN_WAIT), &|| false).unwrap();
        let output = Output::open(Some(NULL_DEVICE)).unwrap();
        assert_eq!(output.heard(), 0.0);
        let id = output.play_live(Arc::clone(&ring), 0.5).unwrap();
        let started = Instant::now();
        let mut peak = output.heard();
        // Input and output ticks are independent. Check the first audible tick
        // rather than reading a later one which may have underrun.
        while peak == 0.0 && started.elapsed() < Duration::from_secs(5) {
            thread::sleep(POLL);
            peak = output.heard();
        }
        // The square wave arrives scaled by the gain.
        assert!((peak - 0.25).abs() < 1e-6, "{peak}");
        assert!((input.0.levels.latest() + 6.0206).abs() < 0.01);
        // Stopping the output's sounds ends the live sound.
        assert!(output.stop_all());
        assert_eq!(output.wait(id, || false), Ended::Done);
        thread::sleep(POLL * 3);
        assert_eq!(output.heard(), 0.0);

        // A live sound ends when its input closes.
        let id = output.play_live(Arc::clone(&ring), 1.0).unwrap();
        drop(input);
        assert_eq!(output.wait(id, || false), Ended::Done);
        assert!(ring.closed());
    }

    #[test]
    fn a_stream_is_recorded_to_a_wav_file_at_its_rate_and_channels() {
        let input = tone_input();
        let ring = input.0.wait_ring(Some(OPEN_WAIT), &|| false).unwrap();
        let path =
            std::env::temp_dir().join(format!("orange-audio-record-{}.wav", std::process::id()));
        let started = Instant::now();
        let seconds = record(Cursor::new(Arc::clone(&ring)), &path, 0.25, &|| false).unwrap();
        // It took about as long as it recorded: it waited for the samples.
        assert!(started.elapsed() >= Duration::from_millis(200));
        assert!((seconds - 0.25).abs() < 1e-9);
        let mut reader = hound::WavReader::open(&path).unwrap();
        let spec = reader.spec();
        assert_eq!(
            (spec.sample_rate, spec.channels, spec.bits_per_sample),
            (48_000, 2, 16)
        );
        let samples: Vec<i16> = reader.samples::<i16>().map(Result::unwrap).collect();
        assert_eq!(samples.len(), 24_000);
        assert!(samples.iter().all(|sample| sample.unsigned_abs() == 16_383));

        // Cancelling ends the file early, still readable.
        let seconds = record(Cursor::new(Arc::clone(&ring)), &path, 60.0, &|| true).unwrap();
        assert_eq!(seconds, 0.0);
        assert_eq!(hound::WavReader::open(&path).unwrap().len(), 0);
        // A file that cannot be made is a fault.
        let folder = std::env::temp_dir();
        assert_eq!(
            record(Cursor::new(ring), &folder, 0.1, &|| false).err(),
            Some(WRITE)
        );
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn an_input_offers_what_each_output_plays_where_the_system_can() {
        let found = Devices {
            inputs: vec![Device {
                id: Some("wasapi:{in}".into()),
                name: "Microphone".into(),
            }],
            outputs: vec![Device {
                id: Some("wasapi:{out}".into()),
                name: "Speakers".into(),
            }],
        };
        let choices = input_choices(found);
        assert_eq!(
            choices[0],
            ("wasapi:{in}".to_owned(), "Microphone".to_owned())
        );
        #[cfg(windows)]
        assert_eq!(
            choices[1],
            (
                "loopback:wasapi:{out}".to_owned(),
                "Speakers (what it plays)".to_owned()
            )
        );
        #[cfg(not(windows))]
        assert_eq!(choices.len(), 1);
    }

    #[test]
    fn record_takes_a_stream_and_asks_where_to_save() {
        let port = |text: &str, direction: &str| -> Option<String> {
            assert!(orange_sdk::NodeDefinition::described(text).is_ok());
            let file: Json = serde_json::from_str(text).unwrap();
            file["ports"]
                .as_array()?
                .iter()
                .find(|port| port["port_id"] == "stream" && port["direction"] == direction)
                .map(|port| port["type"].as_str().unwrap_or_default().to_owned())
        };
        assert_eq!(
            port(&record_definition(), "input").as_deref(),
            Some(STREAM_TYPE)
        );
        let (_, record) = orange_sdk::NodeDefinition::described(&record_definition()).unwrap();
        assert!(record.paths[0].save && !record.paths[0].folder);
    }

    /// Captures what the default output plays (WASAPI loopback), first while this test
    /// plays nothing, then while it plays a quiet 0.6 s tone. Makes a faint sound, so
    /// it is not part of the normal run: `cargo test -p orange-audio --lib -- --ignored`.
    #[test]
    #[ignore = "needs an audio output device and makes a faint sound"]
    fn what_the_default_output_plays_is_captured_by_loopback() {
        let input = InputSession(
            spawn_input(&settings(serde_json::json!({"device": LOOPBACK_PREFIX}))).unwrap(),
        );
        let ring = input.0.wait_ring(Some(OPEN_WAIT), &|| false).unwrap();
        println!("loopback {} Hz, {} channels", ring.rate, ring.channels);
        let before = ring.written();
        let mut idle_cursor = Cursor::new(Arc::clone(&ring));
        thread::sleep(Duration::from_millis(500));
        let idle = ring.written() - before;
        let mut heard = vec![0.0f32; 96_000];
        let read = idle_cursor.read(&mut heard);
        let idle_loudest = heard[..read]
            .iter()
            .fold(0.0f32, |most, s| most.max(s.abs()));
        let output = Output::open(None).unwrap();
        let frames = usize::try_from(output.rate).unwrap() * 6 / 10;
        let channels = usize::from(output.channels);
        let tone: Arc<[f32]> = (0..frames * channels)
            .map(|n| if n / channels % 100 < 50 { 0.05 } else { -0.05 })
            .collect();
        let mut cursor = Cursor::new(Arc::clone(&ring));
        let id = output.play(tone, 1.0, false).unwrap();
        let mut loudest = 0.0f32;
        let mut block = vec![0.0f32; 8_192];
        let started = Instant::now();
        while started.elapsed() < Duration::from_millis(900) {
            let read = cursor.read(&mut block);
            loudest = block[..read]
                .iter()
                .fold(loudest, |most, s| most.max(s.abs()));
            thread::sleep(POLL);
        }
        assert_eq!(output.wait(id, || false), Ended::Done);
        println!(
            "idle: {idle} samples in 500 ms, loudest {idle_loudest}; loudest while playing: {loudest}"
        );
        // Other playback on the machine is captured too, so only sound is checked, not
        // that it is this tone.
        assert!(
            idle > 0 && loudest.max(idle_loudest) > 0.01,
            "nothing was captured"
        );
    }

    /// Records one second of the default microphone through Audio in's ring.
    #[test]
    #[ignore = "needs an audio input device"]
    fn the_default_microphone_is_recorded_to_a_file() {
        let input = InputSession(spawn_input(&Map::new()).unwrap());
        let ring = input.0.wait_ring(Some(OPEN_WAIT), &|| false).unwrap();
        let path = std::env::temp_dir().join("orange-audio-microphone.wav");
        let seconds = record(Cursor::new(Arc::clone(&ring)), &path, 1.0, &|| false).unwrap();
        let reader = hound::WavReader::open(&path).unwrap();
        println!(
            "{} Hz, {} channels, {seconds} s, {} samples",
            ring.rate,
            ring.channels,
            reader.len()
        );
        assert_eq!(
            u64::from(reader.len()),
            u64::from(ring.rate) * u64::from(ring.channels)
        );
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
