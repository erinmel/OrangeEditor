//! Opens operating-system audio devices and connects them to the mixer and the meter.
//!
//! The device named [`NULL_DEVICE`] is not hardware: its output runs the mixer on a
//! timer and discards the samples, and its input hears silence. It is what a machine
//! without a sound device, such as a build server, uses.
use crate::level::{Gate, Meter, SILENCE_DB};
use crate::mixer::{self, Command, MAX_VOICES, Mixer, Voice};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample, StreamConfig};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Device name that selects the built-in silent device instead of hardware.
pub const NULL_DEVICE: &str = "null";
const NULL_RATE: u32 = 48_000;
const NULL_CHANNELS: u16 = 2;
const NULL_BLOCK: Duration = Duration::from_millis(10);
/// Samples converted per step when a device does not take `f32`.
const SCRATCH: usize = 16_384;
/// How long past its length a sound may take before the device counts as stalled.
const STALL: Duration = Duration::from_secs(2);
const POLL: Duration = Duration::from_millis(5);

/// The input and output device names the operating system reports.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Devices {
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
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
        if device.supports_input() {
            found.inputs.push(description.name().to_owned());
        }
        if device.supports_output() {
            found.outputs.push(description.name().to_owned());
        }
    }
    Ok(found)
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
    host.devices()
        .map_err(|error| error.to_string())?
        .find(|device| {
            let usable = if input {
                device.supports_input()
            } else {
                device.supports_output()
            };
            usable
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

fn output_stream<T>(
    device: &cpal::Device,
    config: StreamConfig,
    mut mixer: Mixer,
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
        // A stream error stops callbacks; a playing node then reports a stalled device.
        |_| {},
        None,
    )
}

impl Output {
    /// Opens the default device, the device with this name, or the silent device.
    pub(crate) fn open(device: Option<&str>) -> Result<Self, String> {
        let (mixer, commands, retired) = mixer::mixer();
        let (rate, channels, driver) = if device == Some(NULL_DEVICE) {
            (NULL_RATE, NULL_CHANNELS, null_output(mixer)?)
        } else {
            let device = find(device, false)?;
            let supported = device
                .default_output_config()
                .map_err(|error| error.to_string())?;
            let config = supported.config();
            let (rate, channels) = (config.sample_rate, config.channels);
            let stream = match supported.sample_format() {
                SampleFormat::F32 => output_stream::<f32>(&device, config, mixer),
                SampleFormat::F64 => output_stream::<f64>(&device, config, mixer),
                SampleFormat::I16 => output_stream::<i16>(&device, config, mixer),
                SampleFormat::U16 => output_stream::<u16>(&device, config, mixer),
                SampleFormat::I32 => output_stream::<i32>(&device, config, mixer),
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
            driver,
        })
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

    /// Starts a sound. `None` when [`MAX_VOICES`] are already playing.
    pub(crate) fn play(&self, sound: Arc<[f32]>, gain: f32) -> Option<u64> {
        self.collect();
        let mut playing = self.active.lock().unwrap_or_else(PoisonError::into_inner);
        if *playing >= MAX_VOICES {
            return None;
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let voice = Voice {
            id,
            sound,
            at: 0,
            gain,
        };
        self.commands.try_send(Command::Play(voice)).ok()?;
        *playing += 1;
        Some(id)
    }

    /// Waits until the sound ends. `cancelled` stops it early.
    pub(crate) fn wait(&self, id: u64, samples: usize, cancelled: impl Fn() -> bool) -> Ended {
        let frames = samples / usize::from(self.channels.max(1));
        let length = Duration::from_secs_f64(
            u32::try_from(frames).map_or(f64::MAX, f64::from) / f64::from(self.rate.max(1)),
        );
        let deadline = Instant::now() + length + STALL;
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
            if Instant::now() >= deadline {
                return Ended::Stalled;
            }
            thread::sleep(POLL);
        }
    }

    /// Stops every sound playing on this output.
    pub(crate) fn stop_all(&self) -> bool {
        self.commands.try_send(Command::StopAll).is_ok()
    }
}

fn null_output(mut mixer: Mixer) -> Result<Driver, String> {
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

/// What the capture callback publishes: the latest level and each threshold crossing.
#[derive(Debug)]
pub(crate) struct Levels {
    latest: AtomicU64,
    fired: AtomicU64,
    fired_level: AtomicU64,
}
impl Levels {
    pub(crate) fn new() -> Self {
        Self {
            latest: AtomicU64::new(SILENCE_DB.to_bits()),
            fired: AtomicU64::new(0),
            fired_level: AtomicU64::new(SILENCE_DB.to_bits()),
        }
    }
    /// Records one measured window.
    pub(crate) fn record(&self, gate: &mut Gate, level: f64) {
        self.latest.store(level.to_bits(), Ordering::Release);
        if gate.step(level) {
            self.fired_level.store(level.to_bits(), Ordering::Release);
            let _ = self.fired.fetch_add(1, Ordering::AcqRel);
        }
    }
    pub(crate) fn latest(&self) -> f64 {
        f64::from_bits(self.latest.load(Ordering::Acquire))
    }
    /// How many times the threshold was crossed, and the level of the latest crossing.
    pub(crate) fn fired(&self) -> (u64, f64) {
        let count = self.fired.load(Ordering::Acquire);
        (
            count,
            f64::from_bits(self.fired_level.load(Ordering::Acquire)),
        )
    }
}

fn input_stream<T>(
    device: &cpal::Device,
    config: StreamConfig,
    mut meter: Meter,
    mut gate: Gate,
    levels: Arc<Levels>,
) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            meter.push(
                data.iter().map(|sample| f32::from_sample(*sample)),
                |level| levels.record(&mut gate, level),
            );
        },
        // A stream error stops callbacks; the level then stays at its last value.
        |_| {},
        None,
    )
}

/// Starts measuring an input device. The returned stream measures until dropped;
/// `None` is the silent device, which never measures anything.
pub(crate) fn open_input(
    device: Option<&str>,
    window: Duration,
    gate: Gate,
    levels: Arc<Levels>,
) -> Result<Option<cpal::Stream>, String> {
    if device == Some(NULL_DEVICE) {
        return Ok(None);
    }
    let device = find(device, true)?;
    let supported = device
        .default_input_config()
        .map_err(|error| error.to_string())?;
    let config = supported.config();
    let samples =
        u128::from(config.sample_rate) * u128::from(config.channels) * window.as_millis() / 1_000;
    let meter = Meter::new(usize::try_from(samples).unwrap_or(usize::MAX));
    let stream = match supported.sample_format() {
        SampleFormat::F32 => input_stream::<f32>(&device, config, meter, gate, levels),
        SampleFormat::F64 => input_stream::<f64>(&device, config, meter, gate, levels),
        SampleFormat::I16 => input_stream::<i16>(&device, config, meter, gate, levels),
        SampleFormat::U16 => input_stream::<u16>(&device, config, meter, gate, levels),
        SampleFormat::I32 => input_stream::<i32>(&device, config, meter, gate, levels),
        other => {
            return Err(format!(
                "the device's sample format {other} is not supported"
            ));
        }
    }
    .map_err(|error| error.to_string())?;
    stream.play().map_err(|error| error.to_string())?;
    Ok(Some(stream))
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
        let id = output.play(Arc::clone(&short), 1.0).unwrap();
        let started = Instant::now();
        assert_eq!(output.wait(id, short.len(), || false), Ended::Done);
        assert!(started.elapsed() >= Duration::from_millis(30));

        // 60 s of sound is cut short by cancelling.
        let long: Arc<[f32]> = vec![0.1; rate * 60].into();
        let id = output.play(Arc::clone(&long), 1.0).unwrap();
        let started = Instant::now();
        assert_eq!(output.wait(id, long.len(), || true), Ended::Done);
        assert!(started.elapsed() < Duration::from_secs(10));

        // The voice limit refuses, and stopping all frees every voice again.
        let ids: Vec<u64> = (0..MAX_VOICES)
            .map(|_| output.play(Arc::clone(&long), 1.0).unwrap())
            .collect();
        assert_eq!(output.play(Arc::clone(&long), 1.0), None);
        assert!(output.stop_all());
        for id in ids {
            assert_eq!(output.wait(id, long.len(), || false), Ended::Done);
        }
        assert!(output.play(short, 1.0).is_some());
    }

    #[test]
    fn levels_record_the_latest_window_and_each_crossing() {
        let levels = Levels::new();
        let mut gate = Gate::new(-20.0, -30.0);
        assert_eq!(levels.latest(), SILENCE_DB);
        levels.record(&mut gate, -40.0);
        assert_eq!((levels.latest(), levels.fired().0), (-40.0, 0));
        levels.record(&mut gate, -10.0);
        levels.record(&mut gate, -5.0);
        assert_eq!(levels.fired(), (1, -10.0));
        assert_eq!(levels.latest(), -5.0);
    }
}
