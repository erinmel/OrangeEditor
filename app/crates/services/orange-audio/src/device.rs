//! Opens operating-system audio devices and connects them to the mixer and the meter.
//!
//! The device named [`NULL_DEVICE`] is not hardware: its output runs the mixer on a
//! timer and discards the samples, and its input captures silence on a timer. It is
//! what a machine without a sound device, such as a build server, uses.
//!
//! An input setting that starts with [`LOOPBACK_PREFIX`] names an output device whose
//! playback is captured, where the system offers that (WASAPI loopback on Windows).
use crate::level::{Meter, SILENCE_DB};
use crate::mixer::{self, Command, MAX_VOICES, Mixer, Voice};
use crate::stream::{Live, Ring};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample, StreamConfig};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Device name that selects the built-in silent device instead of hardware.
pub const NULL_DEVICE: &str = "null";
/// Prefix of an input `device` setting that captures what an output device plays:
/// `loopback:` and the output's setting. Offered only where the system supports it.
pub const LOOPBACK_PREFIX: &str = "loopback:";
/// Whether this system can capture what an output device plays. Only WASAPI is
/// offered: cpal 0.18.2 also implements CoreAudio loopback through an aggregate
/// device, which is not verified here, and ALSA has none (10).
pub const LOOPBACK_OFFERED: bool = cfg!(windows);
const NULL_RATE: u32 = 48_000;
const NULL_CHANNELS: u16 = 2;
const NULL_BLOCK: Duration = Duration::from_millis(10);
/// Samples converted per step when a device does not take `f32`.
const SCRATCH: usize = 16_384;
/// How long a device may go without asking for samples before it counts as stalled.
const STALL: Duration = Duration::from_secs(2);
const POLL: Duration = Duration::from_millis(5);

/// One device the operating system reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    /// The operating system's identifier of the endpoint, as `<host>:<id>` (on Windows
    /// `wasapi:` and the endpoint ID, on macOS `coreaudio:` and the device UID, on Linux
    /// `alsa:` and the PCM name). It survives renaming and tells apart devices with
    /// the same name. `None` when the system gives none.
    pub id: Option<String>,
    /// What the system calls it, which two devices can share.
    pub name: String,
}
impl Device {
    /// What a connection's `device` setting holds to name this device: its identifier
    /// where the system has one, else its name.
    pub fn setting(&self) -> &str {
        self.id.as_deref().unwrap_or(&self.name)
    }
}

/// The input and output devices the operating system reports.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Devices {
    pub inputs: Vec<Device>,
    pub outputs: Vec<Device>,
}

/// Lists the devices a connection's `device` setting can name. [`NULL_DEVICE`] is
/// always accepted and is not listed.
pub fn devices() -> Result<Devices, String> {
    let host = cpal::default_host();
    let mut found = Devices::default();
    for device in host.devices().map_err(|error| error.to_string())? {
        let Ok(description) = device.description() else {
            continue;
        };
        let listed = Device {
            id: device.id().ok().map(|id| id.to_string()),
            name: description.name().to_owned(),
        };
        if device.supports_input() {
            found.inputs.push(listed.clone());
        }
        if device.supports_output() {
            found.outputs.push(listed);
        }
    }
    Ok(found)
}

/// How an input is captured: from a capture endpoint, or from what an output plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Capture {
    Input,
    Loopback,
}

/// The input device a setting names, and how it is captured.
fn find_input(name: Option<&str>) -> Result<(cpal::Device, Capture), String> {
    match name.and_then(|name| name.strip_prefix(LOOPBACK_PREFIX)) {
        Some(_) if !LOOPBACK_OFFERED => {
            Err("recording what an output plays is not offered on this system".to_owned())
        }
        // An empty rest is the default output.
        Some(output) => Ok((
            find(Some(output).filter(|output| !output.is_empty()), false)?,
            Capture::Loopback,
        )),
        None => Ok((find(name, true)?, Capture::Input)),
    }
}

fn find(name: Option<&str>, input: bool) -> Result<cpal::Device, String> {
    let host = cpal::default_host();
    let Some(name) = name else {
        let device = if input {
            host.default_input_device()
        } else {
            host.default_output_device()
        };
        return device.ok_or_else(|| "there is no default audio device".to_owned());
    };
    let usable = |device: &cpal::Device| {
        if input {
            device.supports_input()
        } else {
            device.supports_output()
        }
    };
    // An identifier names one endpoint, whatever it is called now; a setting that is
    // not one, or names an endpoint that is gone, is matched as a name.
    if let Ok(id) = name.parse::<cpal::DeviceId>()
        && let Some(device) = host.device_by_id(&id)
        && usable(&device)
    {
        return Ok(device);
    }
    host.devices()
        .map_err(|error| error.to_string())?
        .find(|device| {
            usable(device)
                && device
                    .description()
                    .is_ok_and(|description| description.name() == name)
        })
        .ok_or_else(|| format!("there is no audio device named `{name}`"))
}

enum Driver {
    Stream(#[expect(dead_code, reason = "kept so the device keeps playing")] Mutex<cpal::Stream>),
    Null {
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
    },
}

/// How a played sound ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ended {
    /// It played to its end or was stopped.
    Done,
    /// The device stopped asking for samples.
    Stalled,
}

/// An open output device and the control side of its mixer.
pub(crate) struct Output {
    pub rate: u32,
    pub channels: u16,
    commands: SyncSender<Command>,
    retired: Mutex<Receiver<Voice>>,
    ended: Mutex<BTreeSet<u64>>,
    active: Mutex<usize>,
    next: AtomicU64,
    /// Blocks the device has asked for.
    blocks: Arc<AtomicU64>,
    /// Set by the stream's error callback: the device has gone or changed, and a new
    /// output must be opened.
    broken: Arc<AtomicBool>,
    /// The silent device's loudest sample of its latest block, as `f32` bits.
    #[cfg_attr(not(test), expect(dead_code, reason = "read by tests of live sound"))]
    heard: Arc<AtomicU32>,
    driver: Driver,
}
impl std::fmt::Debug for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Output")
            .field("rate", &self.rate)
            .field("channels", &self.channels)
            .field("active", &self.active)
            .finish_non_exhaustive()
    }
}
impl Drop for Output {
    fn drop(&mut self) {
        if let Driver::Null { stop, thread } = &mut self.driver {
            stop.store(true, Ordering::Release);
            if let Some(thread) = thread.take() {
                let _ = thread.join();
            }
        }
    }
}

/// Whether a stream error means the stream plays or records no more and the device
/// must be opened again. An overrun or underrun (`Xrun`), a followed route change or a
/// refused real-time mode leaves the stream running. Runs in the error callback, so it
/// only looks at the kind.
fn fatal(error: &cpal::Error) -> bool {
    use cpal::ErrorKind::{DeviceNotAvailable, HostUnavailable, StreamInvalidated};
    matches!(
        error.kind(),
        DeviceNotAvailable | HostUnavailable | StreamInvalidated
    )
}

fn output_stream<T>(
    device: &cpal::Device,
    config: StreamConfig,
    mut mixer: Mixer,
    broken: Arc<AtomicBool>,
) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample + FromSample<f32>,
{
    // Allocated here, before the first callback.
    let mut scratch = vec![0.0f32; SCRATCH];
    device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            for chunk in data.chunks_mut(SCRATCH) {
                let Some(block) = scratch.get_mut(..chunk.len()) else {
                    continue;
                };
                mixer.fill(block);
                for (out, sample) in chunk.iter_mut().zip(block.iter()) {
                    *out = T::from_sample(*sample);
                }
            }
        },
        // A fatal stream error stops callbacks; a playing node then reports a stalled device
        // and the next one opens the device again.
        move |error| {
            if fatal(&error) {
                broken.store(true, Ordering::Release);
            }
        },
        None,
    )
}

impl Output {
    /// Opens the default device, the device with this name, or the silent device.
    pub(crate) fn open(device: Option<&str>) -> Result<Self, String> {
        let (mixer, commands, retired, blocks) = mixer::mixer();
        let broken = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&broken);
        let heard = Arc::new(AtomicU32::new(0));
        let (rate, channels, driver) = if device == Some(NULL_DEVICE) {
            (
                NULL_RATE,
                NULL_CHANNELS,
                null_output(mixer, Arc::clone(&heard))?,
            )
        } else {
            let device = find(device, false)?;
            let supported = device
                .default_output_config()
                .map_err(|error| error.to_string())?;
            let config = supported.config();
            let (rate, channels) = (config.sample_rate, config.channels);
            let stream = match supported.sample_format() {
                SampleFormat::F32 => output_stream::<f32>(&device, config, mixer, flag),
                SampleFormat::F64 => output_stream::<f64>(&device, config, mixer, flag),
                SampleFormat::I16 => output_stream::<i16>(&device, config, mixer, flag),
                SampleFormat::U16 => output_stream::<u16>(&device, config, mixer, flag),
                SampleFormat::I32 => output_stream::<i32>(&device, config, mixer, flag),
                other => {
                    return Err(format!(
                        "the device's sample format {other} is not supported"
                    ));
                }
            }
            .map_err(|error| error.to_string())?;
            stream.play().map_err(|error| error.to_string())?;
            (rate, channels, Driver::Stream(Mutex::new(stream)))
        };
        Ok(Self {
            rate,
            channels,
            commands,
            retired: Mutex::new(retired),
            ended: Mutex::new(BTreeSet::new()),
            active: Mutex::new(0),
            next: AtomicU64::new(0),
            blocks,
            broken,
            heard,
            driver,
        })
    }

    /// The device reported an error and plays no more.
    pub(crate) fn broken(&self) -> bool {
        self.broken.load(Ordering::Acquire)
    }

    /// Acts as the stream's error callback does.
    #[cfg(test)]
    pub(crate) fn fail(&self) {
        self.broken.store(true, Ordering::Release);
    }

    /// Takes ended sounds back from the callback and frees them here.
    fn collect(&self) {
        let retired = self.retired.lock().unwrap_or_else(PoisonError::into_inner);
        let mut ended = self.ended.lock().unwrap_or_else(PoisonError::into_inner);
        for voice in retired.try_iter() {
            let mut playing = self.active.lock().unwrap_or_else(PoisonError::into_inner);
            *playing = playing.saturating_sub(1);
            let _ = ended.insert(voice.id);
        }
        // IDs nobody waited for (a stalled device that later recovered).
        while ended.len() > MAX_VOICES * 4 {
            let _ = ended.pop_first();
        }
    }

    /// Starts a sound, repeating until stopped when `looped`. `None` when
    /// [`MAX_VOICES`] are already playing.
    pub(crate) fn play(&self, sound: Arc<[f32]>, gain: f32, looped: bool) -> Option<u64> {
        self.start(|id| Voice {
            id,
            sound,
            at: 0,
            gain,
            looped,
            live: None,
        })
    }

    /// Starts playing what an input captures, converted to this output's rate and
    /// channels, until it is stopped or the input closes. `None` when [`MAX_VOICES`]
    /// are already playing.
    pub(crate) fn play_live(&self, ring: Arc<Ring>, gain: f32) -> Option<u64> {
        // The reader is made here so the callback only plays it.
        let live = Box::new(Live::new(ring, self.rate, self.channels));
        self.start(|id| Voice {
            id,
            sound: Arc::from([]),
            at: 0,
            gain,
            looped: false,
            live: Some(live),
        })
    }

    fn start(&self, voice: impl FnOnce(u64) -> Voice) -> Option<u64> {
        self.collect();
        let mut playing = self.active.lock().unwrap_or_else(PoisonError::into_inner);
        if *playing >= MAX_VOICES {
            return None;
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        self.commands.try_send(Command::Play(voice(id))).ok()?;
        *playing += 1;
        Some(id)
    }

    /// The loudest sample the silent device was given in its latest block: what would
    /// have been heard. Always 0 on a real device.
    #[cfg(test)]
    pub(crate) fn heard(&self) -> f32 {
        f32::from_bits(self.heard.load(Ordering::Acquire))
    }

    /// Waits until the sound ends, however long it is paused or repeats. `cancelled`
    /// stops it early. A device that stops asking for samples is reported as stalled.
    pub(crate) fn wait(&self, id: u64, cancelled: impl Fn() -> bool) -> Ended {
        let mut seen = self.blocks.load(Ordering::Relaxed);
        let mut deadline = Instant::now() + STALL;
        let mut stopped = false;
        loop {
            self.collect();
            if self
                .ended
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&id)
            {
                return Ended::Done;
            }
            if !stopped && cancelled() {
                stopped = self.commands.try_send(Command::Stop(id)).is_ok();
            }
            let blocks = self.blocks.load(Ordering::Relaxed);
            if blocks != seen {
                seen = blocks;
                deadline = Instant::now() + STALL;
            } else if Instant::now() >= deadline {
                return Ended::Stalled;
            }
            thread::sleep(POLL);
        }
    }

    /// Holds every sound on this output where it is, or lets them continue.
    pub(crate) fn pause(&self, paused: bool) -> bool {
        let command = if paused {
            Command::Pause
        } else {
            Command::Resume
        };
        self.commands.try_send(command).is_ok()
    }

    /// Sets the gain of the whole output.
    pub(crate) fn volume(&self, gain: f32) -> bool {
        self.commands.try_send(Command::Volume(gain)).is_ok()
    }

    /// Moves every sound on this output to `seconds` from its start.
    pub(crate) fn seek(&self, seconds: f64) -> bool {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "the caller checked the range; the product saturates"
        )]
        let frames = (seconds * f64::from(self.rate)) as usize;
        let offset = frames.saturating_mul(usize::from(self.channels));
        self.commands.try_send(Command::Seek(offset)).is_ok()
    }

    /// Stops every sound playing on this output.
    pub(crate) fn stop_all(&self) -> bool {
        self.commands.try_send(Command::StopAll).is_ok()
    }
}

fn null_output(mut mixer: Mixer, heard: Arc<AtomicU32>) -> Result<Driver, String> {
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = Arc::clone(&stop);
    let thread = thread::Builder::new()
        .name("orange-audio-null".into())
        .spawn(move || {
            let frames = usize::try_from(u128::from(NULL_RATE) * NULL_BLOCK.as_millis() / 1_000)
                .unwrap_or(480);
            let mut block = vec![0.0; frames * usize::from(NULL_CHANNELS)];
            let start = Instant::now();
            let mut blocks = 0u32;
            while !stopped.load(Ordering::Acquire) {
                mixer.fill(&mut block);
                let loudest = block
                    .iter()
                    .fold(0.0f32, |most, sample| most.max(sample.abs()));
                heard.store(loudest.to_bits(), Ordering::Release);
                blocks = blocks.wrapping_add(1);
                if let Some(wait) = (NULL_BLOCK * blocks).checked_sub(start.elapsed()) {
                    thread::sleep(wait);
                }
            }
        })
        .map_err(|error| error.to_string())?;
    Ok(Driver::Null {
        stop,
        thread: Some(thread),
    })
}

/// What the capture callback publishes: the latest level of the whole sound. Events are
/// made by analyzers reading the ring (`analysis`), never on the callback.
#[derive(Debug)]
pub(crate) struct Levels {
    latest: AtomicU64,
    /// Set by the stream's error callback: the device has gone and must be reopened.
    broken: AtomicBool,
}
impl Levels {
    pub(crate) fn new() -> Self {
        Self {
            latest: AtomicU64::new(SILENCE_DB.to_bits()),
            broken: AtomicBool::new(false),
        }
    }
    /// Takes the stream error flag: whether the device failed since the last call.
    pub(crate) fn take_broken(&self) -> bool {
        self.broken.swap(false, Ordering::AcqRel)
    }
    /// Records one measured window.
    pub(crate) fn record(&self, level: f64) {
        self.latest.store(level.to_bits(), Ordering::Release);
    }
    pub(crate) fn latest(&self) -> f64 {
        f64::from_bits(self.latest.load(Ordering::Acquire))
    }
}
fn input_stream<T>(
    device: &cpal::Device,
    config: StreamConfig,
    mut meter: Meter,
    levels: Arc<Levels>,
    ring: Arc<Ring>,
) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let failed = Arc::clone(&levels);
    device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            meter.push(
                data.iter().map(|sample| f32::from_sample(*sample)),
                |level| levels.record(level),
            );
            ring.push(data.iter().map(|sample| f32::from_sample(*sample)));
        },
        // A fatal stream error stops callbacks; the listening thread then opens it again.
        move |error| {
            if fatal(&error) {
                failed.broken.store(true, Ordering::Release);
            }
        },
        None,
    )
}

/// A test input that captures a square wave of amplitude 0.5 (-6.02 dBFS) on the
/// silent device's timer.
#[cfg(test)]
pub(crate) const TONE_DEVICE: &str = "test-tone";

enum InputDriver {
    Stream(#[expect(dead_code, reason = "kept so the device keeps capturing")] cpal::Stream),
    Null {
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
    },
}

/// An input device being captured: measured, and its samples kept in [`Ring`] for
/// the consumers of its stream. Dropping it stops the device and closes the ring.
pub(crate) struct Capturing {
    ring: Arc<Ring>,
    driver: InputDriver,
}
impl Capturing {
    pub(crate) fn ring(&self) -> &Arc<Ring> {
        &self.ring
    }
}
impl Drop for Capturing {
    fn drop(&mut self) {
        if let InputDriver::Null { stop, thread } = &mut self.driver {
            stop.store(true, Ordering::Release);
            if let Some(thread) = thread.take() {
                let _ = thread.join();
            }
        }
        self.ring.close();
    }
}

/// The silent input: blocks of silence (or of the test tone) at the silent device's
/// rate, on a timer, measured and kept like a device's.
fn null_input(
    tone: bool,
    mut meter: Meter,
    levels: Arc<Levels>,
    ring: Arc<Ring>,
) -> Result<InputDriver, String> {
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = Arc::clone(&stop);
    let thread = thread::Builder::new()
        .name("orange-audio-null-input".into())
        .spawn(move || {
            let frames = usize::try_from(u128::from(NULL_RATE) * NULL_BLOCK.as_millis() / 1_000)
                .unwrap_or(480);
            let channels = usize::from(NULL_CHANNELS);
            let block: Vec<f32> = (0..frames * channels)
                .map(|n| match (tone, n / channels % 48 < 24) {
                    (false, _) => 0.0,
                    (true, true) => 0.5,
                    (true, false) => -0.5,
                })
                .collect();
            let start = Instant::now();
            let mut blocks = 0u32;
            while !stopped.load(Ordering::Acquire) {
                meter.push(block.iter().copied(), |level| levels.record(level));
                ring.push(block.iter().copied());
                blocks = blocks.wrapping_add(1);
                if let Some(wait) = (NULL_BLOCK * blocks).checked_sub(start.elapsed()) {
                    thread::sleep(wait);
                }
            }
        })
        .map_err(|error| error.to_string())?;
    Ok(InputDriver::Null {
        stop,
        thread: Some(thread),
    })
}

/// Starts capturing an input device: the default one, the one a setting names, what
/// an output plays (a [`LOOPBACK_PREFIX`] setting) or the silent device. Each window
/// of `window` is measured for the input's level; the samples go to a new ring
/// at the device's own rate and channels.
pub(crate) fn open_input(
    device: Option<&str>,
    window: Duration,
    levels: Arc<Levels>,
) -> Result<Capturing, String> {
    let meter = |rate: u32, channels: u16| {
        let samples = u128::from(rate) * u128::from(channels) * window.as_millis() / 1_000;
        Meter::new(usize::try_from(samples).unwrap_or(usize::MAX))
    };
    let null = device == Some(NULL_DEVICE);
    #[cfg(test)]
    let null = null || device == Some(TONE_DEVICE);
    if null {
        let ring = Ring::new(NULL_RATE, NULL_CHANNELS);
        #[cfg(test)]
        let tone = device == Some(TONE_DEVICE);
        #[cfg(not(test))]
        let tone = false;
        let driver = null_input(
            tone,
            meter(NULL_RATE, NULL_CHANNELS),
            levels,
            Arc::clone(&ring),
        )?;
        return Ok(Capturing { ring, driver });
    }
    let (device, capture) = find_input(device)?;
    // Loopback captures in the output's own format.
    let supported = match capture {
        Capture::Input => device.default_input_config(),
        Capture::Loopback => device.default_output_config(),
    }
    .map_err(|error| error.to_string())?;
    let config = supported.config();
    let ring = Ring::new(config.sample_rate, config.channels);
    let meter = meter(config.sample_rate, config.channels);
    let stream = {
        let ring = Arc::clone(&ring);
        match supported.sample_format() {
            SampleFormat::F32 => input_stream::<f32>(&device, config, meter, levels, ring),
            SampleFormat::F64 => input_stream::<f64>(&device, config, meter, levels, ring),
            SampleFormat::I16 => input_stream::<i16>(&device, config, meter, levels, ring),
            SampleFormat::U16 => input_stream::<u16>(&device, config, meter, levels, ring),
            SampleFormat::I32 => input_stream::<i32>(&device, config, meter, levels, ring),
            other => {
                return Err(format!(
                    "the device's sample format {other} is not supported"
                ));
            }
        }
    }
    .map_err(|error| error.to_string())?;
    stream.play().map_err(|error| error.to_string())?;
    Ok(Capturing {
        ring,
        driver: InputDriver::Stream(stream),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_silent_output_plays_stops_and_limits_sounds() {
        let output = Output::open(Some(NULL_DEVICE)).unwrap();
        let rate = usize::try_from(output.rate).unwrap() * usize::from(output.channels);
        // 50 ms of sound ends on its own.
        let short: Arc<[f32]> = vec![0.1; rate / 20].into();
        let id = output.play(Arc::clone(&short), 1.0, false).unwrap();
        let started = Instant::now();
        assert_eq!(output.wait(id, || false), Ended::Done);
        assert!(started.elapsed() >= Duration::from_millis(30));

        // 60 s of sound is cut short by cancelling.
        let long: Arc<[f32]> = vec![0.1; rate * 60].into();
        let id = output.play(Arc::clone(&long), 1.0, false).unwrap();
        let started = Instant::now();
        assert_eq!(output.wait(id, || true), Ended::Done);
        assert!(started.elapsed() < Duration::from_secs(10));

        // The voice limit refuses, and stopping all frees every voice again.
        let ids: Vec<u64> = (0..MAX_VOICES)
            .map(|_| output.play(Arc::clone(&long), 1.0, false).unwrap())
            .collect();
        assert_eq!(output.play(Arc::clone(&long), 1.0, false), None);
        assert!(output.stop_all());
        for id in ids {
            assert_eq!(output.wait(id, || false), Ended::Done);
        }
        assert!(output.play(short, 1.0, false).is_some());
    }

    #[test]
    fn levels_keep_the_latest_window() {
        let levels = Levels::new();
        assert_eq!(levels.latest(), SILENCE_DB);
        levels.record(-40.0);
        levels.record(-5.0);
        assert_eq!(levels.latest(), -5.0);
    }

    /// Asks the operating system, so it is not part of the normal run:
    /// `cargo test -p orange-audio -- --ignored`. Opens nothing and makes no sound.
    #[test]
    #[ignore = "needs the machine's audio devices"]
    fn every_listed_device_is_found_again_by_what_its_setting_holds() {
        let found = devices().unwrap();
        for (listed, input) in found
            .outputs
            .iter()
            .map(|device| (device, false))
            .chain(found.inputs.iter().map(|device| (device, true)))
        {
            assert!(listed.id.is_some(), "{listed:?} has no identifier");
            let again = find(Some(listed.setting()), input).unwrap();
            assert_eq!(again.id().unwrap().to_string(), listed.setting());
            println!("{} {listed:?}", if input { "input " } else { "output" });
        }
    }
}
