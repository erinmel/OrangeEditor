//! Stream processors: each reads a PCM stream on a worker, outside capture callbacks,
//! and publishes another routable stream. The Noise Gate, Compressor, Limiter, Gain
//! and Filter share the reading loop and differ only in what they do to a block.
use crate::{
    Analyzing, Cursor, Definition, EmitError, InputShared, Lifecycle, Map, SILENCE_DB, Value,
    analysis::number, number_setting, stream::Ring, stream_value,
};
use orange_sdk::{Arguments, Publisher};
use serde_json::{Value as Json, json};
use std::{
    f64::consts::TAU,
    sync::{Arc, atomic::Ordering},
    thread,
    time::Duration,
};

/// What a processor does to the samples of one stream.
pub(crate) trait Process: Send + 'static {
    /// Changes `samples`, interleaved `channels` at a time, in place.
    fn process(&mut self, samples: &mut [f32], channels: u16);
}

/// Builds a processor for a stream's rate and channel count, once per stream and again
/// after samples were lost, so no state from before the gap is applied after it.
pub(crate) type Make = Box<dyn Fn(u32, u16) -> Box<dyn Process> + Send>;

/// A number setting: its key, range and default.
#[derive(Clone, Copy)]
pub(crate) struct Setting(pub &'static str, pub f64, pub f64, pub f64);

impl Setting {
    pub(crate) fn add(self, definition: Definition) -> Definition {
        definition
            .setting(self.0, number_setting(self.1, self.2, self.3))
            .setting_control(self.0, orange_sdk::Control::slider().number())
    }

    pub(crate) fn read(self, config: &Map<String, Json>) -> Result<f64, String> {
        let Self(key, min, max, default) = self;
        let value = number(config, key, default)?;
        if !(min..=max).contains(&value) {
            return Err(format!("`{key}` is outside its range"));
        }
        Ok(value)
    }
}

/// A wireable duration with a default and bounds expressed in milliseconds.
struct Timing(&'static str, i32, i32, i32);

impl Timing {
    fn add(&self, definition: Definition) -> Definition {
        let nanos = i64::from(self.3) * 1_000_000;
        let ty = "core/duration@1";
        definition.input_default(self.0, ty, json!({"type": ty, "value": nanos.to_string()}))
    }

    fn millis(&self, arguments: &Arguments) -> Result<f64, String> {
        let Some(value) = arguments.get(self.0) else {
            return Ok(f64::from(self.3));
        };
        let Value::Duration(nanos) = value else {
            return Err(format!("`{}` needs a duration", self.0));
        };
        let bounds = i64::from(self.1) * 1_000_000..=i64::from(self.2) * 1_000_000;
        if !bounds.contains(nanos) {
            return Err(format!(
                "`{}` must be between {}ms and {}ms",
                self.0, self.1, self.2
            ));
        }
        #[expect(
            clippy::cast_precision_loss,
            reason = "bounded audio timings fit exactly in binary64 nanoseconds"
        )]
        Ok(*nanos as f64 / 1_000_000.0)
    }
}

/// Power, as a mean of squared samples, of a level in dBFS.
fn power_of(db: f64) -> f64 {
    10.0_f64.powf(db / 10.0)
}

/// Amplitude factor of a gain in dB.
fn factor_of(db: f64) -> f64 {
    10.0_f64.powf(db / 20.0)
}

/// The per-frame step that moves a smoothed value most of the way in `ms`.
fn smoothing(ms: f64, rate: u32) -> f64 {
    let frames = ms * f64::from(rate) / 1_000.0;
    if frames <= 1.0 {
        1.0
    } else {
        1.0 - (-1.0 / frames).exp()
    }
}

/// Mean power of one frame, with non-finite samples silenced.
fn frame_power(frame: &mut [f32]) -> f64 {
    let mut power = 0.0;
    for sample in &mut *frame {
        if !sample.is_finite() {
            *sample = 0.0;
        }
        power += f64::from(*sample).powi(2);
    }
    #[expect(clippy::cast_precision_loss, reason = "at most a few channels")]
    let channels = frame.len() as f64;
    power / channels
}

#[expect(clippy::cast_possible_truncation, reason = "bounded PCM gain")]
fn scale(frame: &mut [f32], gain: f64) {
    for sample in frame {
        *sample = (f64::from(*sample) * gain) as f32;
    }
}

// --- Noise Gate -------------------------------------------------------------------

const GATE_THRESHOLD: Setting = Setting("threshold_db", SILENCE_DB, 0.0, -40.0);
const GATE_RESET: Setting = Setting("reset_db", 0.0, 24.0, 4.0);
const GATE_ATTACK: Timing = Timing("attack", 0, 250, 5);
const GATE_HOLD: Timing = Timing("hold", 0, 2_000, 100);
const GATE_RELEASE: Timing = Timing("release", 0, 2_000, 80);

pub(crate) fn gate_settings(definition: Definition) -> Definition {
    let definition = GATE_RESET.add(GATE_THRESHOLD.add(definition));
    [GATE_ATTACK, GATE_HOLD, GATE_RELEASE]
        .into_iter()
        .fold(definition, |definition, setting| setting.add(definition))
}

/// Mutes the stream while it is quieter than the threshold. Every channel gets the same
/// gain, so a stereo image does not move.
struct Gate {
    open_power: f64,
    close_power: f64,
    power: f64,
    smoothing: f64,
    open: bool,
    hold_frames: f64,
    remaining: f64,
    gain: f64,
    up: f64,
    down: f64,
}

pub(crate) fn gate(config: &Map<String, Json>, arguments: &Arguments) -> Result<Make, String> {
    let threshold = GATE_THRESHOLD.read(config)?;
    let margin = GATE_RESET.read(config)?;
    let attack = GATE_ATTACK.millis(arguments)?;
    let hold = GATE_HOLD.millis(arguments)?;
    let release = GATE_RELEASE.millis(arguments)?;
    Ok(Box::new(move |rate, _| {
        let frames = |ms: f64| ms * f64::from(rate) / 1_000.0;
        let step = |ms| 1.0 / frames(ms).max(1.0);
        Box::new(Gate {
            open_power: power_of(threshold),
            close_power: power_of((threshold - margin).max(SILENCE_DB)),
            power: 0.0,
            smoothing: 1.0 - (-step(10.0)).exp(),
            open: false,
            hold_frames: frames(hold).ceil(),
            remaining: 0.0,
            gain: 0.0,
            up: step(attack),
            down: step(release),
        })
    }))
}

impl Process for Gate {
    fn process(&mut self, samples: &mut [f32], channels: u16) {
        for frame in samples.chunks_exact_mut(usize::from(channels)) {
            let power = frame_power(frame);
            self.power += (power - self.power) * self.smoothing;
            if self.power > self.open_power || (self.open && self.power > self.close_power) {
                self.open = true;
                self.remaining = self.hold_frames;
            } else if self.remaining > 0.0 {
                self.remaining -= 1.0;
            } else {
                self.open = false;
            }
            self.gain = if self.open {
                (self.gain + self.up).min(1.0)
            } else {
                (self.gain - self.down).max(0.0)
            };
            scale(frame, self.gain);
        }
    }
}

// --- Compressor -------------------------------------------------------------------

const COMP_THRESHOLD: Setting = Setting("threshold_db", -60.0, 0.0, -18.0);
const COMP_RATIO: Setting = Setting("ratio", 1.0, 20.0, 4.0);
const COMP_ATTACK: Timing = Timing("attack", 0, 250, 10);
const COMP_RELEASE: Timing = Timing("release", 1, 2_000, 120);
const COMP_MAKEUP: Setting = Setting("makeup_db", 0.0, 24.0, 0.0);

pub(crate) fn compressor_settings(definition: Definition) -> Definition {
    let definition = COMP_MAKEUP.add(COMP_RATIO.add(COMP_THRESHOLD.add(definition)));
    COMP_RELEASE.add(COMP_ATTACK.add(definition))
}

/// Turns the level above the threshold down by the ratio: at 4:1, 8 dB over becomes
/// 2 dB over. The reduction follows the level with the attack and release times, and
/// makeup gain raises the result again.
struct Compressor {
    threshold: f64,
    slope: f64,
    makeup: f64,
    attack: f64,
    release: f64,
    power: f64,
    detect: f64,
    reduction: f64,
}

pub(crate) fn compressor(
    config: &Map<String, Json>,
    arguments: &Arguments,
) -> Result<Make, String> {
    let threshold = COMP_THRESHOLD.read(config)?;
    let ratio = COMP_RATIO.read(config)?;
    let attack = COMP_ATTACK.millis(arguments)?;
    let release = COMP_RELEASE.millis(arguments)?;
    let makeup = COMP_MAKEUP.read(config)?;
    Ok(Box::new(move |rate, _| {
        Box::new(Compressor {
            threshold,
            slope: 1.0 - 1.0 / ratio,
            makeup: factor_of(makeup),
            attack: smoothing(attack, rate),
            release: smoothing(release, rate),
            power: 0.0,
            detect: smoothing(5.0, rate),
            reduction: 0.0,
        })
    }))
}

impl Process for Compressor {
    fn process(&mut self, samples: &mut [f32], channels: u16) {
        for frame in samples.chunks_exact_mut(usize::from(channels)) {
            let power = frame_power(frame);
            self.power += (power - self.power) * self.detect;
            let level = if self.power > 0.0 {
                10.0 * self.power.log10()
            } else {
                SILENCE_DB
            };
            let target = (level - self.threshold).max(0.0) * self.slope;
            let speed = if target > self.reduction {
                self.attack
            } else {
                self.release
            };
            self.reduction += (target - self.reduction) * speed;
            scale(frame, factor_of(-self.reduction) * self.makeup);
        }
    }
}

// --- Limiter ----------------------------------------------------------------------

const LIMIT_CEILING: Setting = Setting("ceiling_db", -24.0, 0.0, -1.0);
const LIMIT_RELEASE: Timing = Timing("release", 1, 1_000, 50);

pub(crate) fn limiter_settings(definition: Definition) -> Definition {
    LIMIT_RELEASE.add(LIMIT_CEILING.add(definition))
}

/// Keeps every sample at or under the ceiling: a peak above it turns the gain down at
/// once, and the gain comes back up over the release time.
struct Limiter {
    ceiling: f64,
    release: f64,
    gain: f64,
}

pub(crate) fn limiter(config: &Map<String, Json>, arguments: &Arguments) -> Result<Make, String> {
    let ceiling = factor_of(LIMIT_CEILING.read(config)?);
    let release = LIMIT_RELEASE.millis(arguments)?;
    Ok(Box::new(move |rate, _| {
        Box::new(Limiter {
            ceiling,
            release: smoothing(release, rate),
            gain: 1.0,
        })
    }))
}

impl Process for Limiter {
    fn process(&mut self, samples: &mut [f32], channels: u16) {
        for frame in samples.chunks_exact_mut(usize::from(channels)) {
            let mut peak = 0.0_f64;
            for sample in &mut *frame {
                if !sample.is_finite() {
                    *sample = 0.0;
                }
                peak = peak.max(f64::from(sample.abs()));
            }
            let needed = if peak > self.ceiling {
                self.ceiling / peak
            } else {
                1.0
            };
            self.gain = if needed < self.gain {
                needed
            } else {
                self.gain + (needed - self.gain) * self.release
            };
            scale(frame, self.gain);
        }
    }
}

// --- Gain -------------------------------------------------------------------------

const GAIN: Setting = Setting("gain_db", -60.0, 24.0, 0.0);

pub(crate) fn gain_settings(definition: Definition) -> Definition {
    GAIN.add(definition)
}

/// Makes the stream louder or quieter by a fixed number of decibels.
struct Gain(f64);

pub(crate) fn gain(config: &Map<String, Json>) -> Result<Make, String> {
    let factor = factor_of(GAIN.read(config)?);
    Ok(Box::new(move |_, _| Box::new(Gain(factor))))
}

impl Process for Gain {
    fn process(&mut self, samples: &mut [f32], channels: u16) {
        for frame in samples.chunks_exact_mut(usize::from(channels)) {
            for sample in &mut *frame {
                if !sample.is_finite() {
                    *sample = 0.0;
                }
            }
            scale(frame, self.0);
        }
    }
}

// --- Filter -----------------------------------------------------------------------

const FILTER_CUTOFF: Setting = Setting("cutoff_hz", 20.0, 20_000.0, 1_000.0);
const FILTER_Q: Setting = Setting("q", 0.1, 10.0, std::f64::consts::FRAC_1_SQRT_2);
const FILTER_MODES: [&str; 4] = ["low-pass", "high-pass", "band-pass", "notch"];

pub(crate) fn filter_settings(definition: Definition) -> Definition {
    let definition = definition.setting(
        "mode",
        json!({"type": "string", "enum": FILTER_MODES, "default": "low-pass"}),
    );
    [FILTER_CUTOFF, FILTER_Q]
        .into_iter()
        .fold(definition, |definition, setting| setting.add(definition))
}

/// A second-order filter (RBJ biquad) with its own state for each channel.
struct Filter {
    b: [f64; 3],
    a: [f64; 2],
    state: Vec<[f64; 4]>,
}

pub(crate) fn filter(config: &Map<String, Json>) -> Result<Make, String> {
    let mode = match config.get("mode") {
        None | Some(Json::Null) => "low-pass".to_owned(),
        Some(Json::String(mode)) if FILTER_MODES.contains(&mode.as_str()) => mode.clone(),
        Some(_) => return Err("`mode` is low-pass, high-pass, band-pass or notch".to_owned()),
    };
    let cutoff = FILTER_CUTOFF.read(config)?;
    let q = FILTER_Q.read(config)?;
    Ok(Box::new(move |rate, channels| {
        // Keep the cutoff under the stream's Nyquist frequency.
        let cutoff = cutoff.min(f64::from(rate) * 0.45);
        let w = TAU * cutoff / f64::from(rate);
        let (sin, cos) = w.sin_cos();
        let alpha = sin / (2.0 * q);
        let b = match mode.as_str() {
            "high-pass" => [(1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0],
            "band-pass" => [alpha, 0.0, -alpha],
            "notch" => [1.0, -2.0 * cos, 1.0],
            _ => [(1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0],
        };
        let a0 = 1.0 + alpha;
        Box::new(Filter {
            b: b.map(|b| b / a0),
            a: [-2.0 * cos / a0, (1.0 - alpha) / a0],
            state: vec![[0.0; 4]; usize::from(channels)],
        })
    }))
}

impl Process for Filter {
    #[expect(clippy::cast_possible_truncation, reason = "a stable filter of PCM")]
    fn process(&mut self, samples: &mut [f32], channels: u16) {
        for frame in samples.chunks_exact_mut(usize::from(channels)) {
            for (sample, [x1, x2, y1, y2]) in frame.iter_mut().zip(&mut self.state) {
                let x = if sample.is_finite() {
                    f64::from(*sample)
                } else {
                    0.0
                };
                let y = self.b[0] * x + self.b[1] * *x1 + self.b[2] * *x2
                    - self.a[0] * *y1
                    - self.a[1] * *y2;
                (*x2, *x1, *y2, *y1) = (*x1, x, *y1, y);
                *sample = y as f32;
            }
        }
    }
}

// --- The shared reading loop ------------------------------------------------------

struct Output(Arc<InputShared>);

impl Drop for Output {
    fn drop(&mut self) {
        self.0.stop.store(true, Ordering::Release);
        self.0.replace_ring(None);
    }
}

/// Starts `make`'s processor on a worker named `name`, reading `input` and publishing
/// the processed stream.
pub(crate) fn start(
    name: &str,
    input: Arc<InputShared>,
    make: Make,
    publisher: Publisher,
    run_id: u64,
) -> Result<Box<dyn Send>, String> {
    let output = Output(InputShared::register(None, None, Duration::from_millis(25)));
    let value = stream_value(&output.0, run_id).map_err(|fault| fault.message.to_owned())?;
    let life = Arc::new(Lifecycle::new());
    let running = Arc::clone(&life);
    thread::Builder::new()
        .name(format!("orange-audio-{name}"))
        .spawn(move || run(&input, &output, &make, &running, &publisher, value))
        .map_err(|error| error.to_string())?;
    Ok(Box::new(Analyzing(life)))
}

fn run(
    input: &InputShared,
    output: &Output,
    make: &Make,
    life: &Lifecycle,
    publisher: &Publisher,
    value: Value,
) {
    let mut block = vec![0.0; 8192];
    let mut announced = false;
    while let Ok(ring) = input.wait_ring(None, &|| life.stopped()) {
        let channels = ring.channels;
        let mut cursor = Cursor::new(Arc::clone(&ring));
        let processed = Ring::new(ring.rate, channels);
        output.0.replace_ring(Some(Arc::clone(&processed)));
        let mut processor = make(ring.rate, channels);
        block.resize(block.len().max(usize::from(channels)), 0.0);
        while !life.stopped() {
            if !announced {
                match publisher.publish(value.clone()) {
                    Ok(()) => announced = true,
                    Err(EmitError::Closed) => return,
                    Err(EmitError::Busy) => {}
                }
            }
            let dropped = cursor.dropped;
            let read = cursor.read(&mut block);
            if read == 0 {
                if ring.closed() {
                    break;
                }
                let _ = life.rest(Duration::from_millis(5));
                continue;
            }
            if dropped != cursor.dropped {
                processor = make(ring.rate, channels);
            }
            let samples = &mut block[..read];
            processor.process(samples, channels);
            processed.push(samples.iter().copied());
        }
        output.0.replace_ring(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(json: Json) -> Map<String, Json> {
        json.as_object().unwrap().clone()
    }

    fn peak(samples: &[f32]) -> f32 {
        samples
            .iter()
            .fold(0.0, |peak, sample| peak.max(sample.abs()))
    }

    /// A sine of `hz` at amplitude `level`, `seconds` long, mono at `rate`.
    fn sine(hz: f64, level: f32, rate: u32, seconds: f64) -> Vec<f32> {
        #[expect(clippy::cast_possible_truncation, reason = "test lengths")]
        #[expect(clippy::cast_sign_loss, reason = "test lengths")]
        let frames = (f64::from(rate) * seconds) as usize;
        (0..frames)
            .map(|n| {
                #[expect(clippy::cast_precision_loss, reason = "test lengths")]
                let t = n as f64 / f64::from(rate);
                #[expect(clippy::cast_possible_truncation, reason = "test PCM")]
                let sample = ((TAU * hz * t).sin() as f32) * level;
                sample
            })
            .collect()
    }

    #[test]
    fn the_gate_mutes_quiet_pcm_and_uses_the_same_gain_for_both_channels() {
        let mut gate = gate(&Map::new(), &Arguments::default()).unwrap()(1_000, 2);
        let mut quiet = [0.001; 80];
        gate.process(&mut quiet, 2);
        assert_eq!(quiet, [0.0; 80]);

        let mut loud = [0.5, 0.25].repeat(16);
        gate.process(&mut loud, 2);
        assert!(loud[0] > 0.0 && loud[0] < 0.5);
        assert_eq!(&loud[30..], &[0.5, 0.25]);
        assert!(
            loud.as_chunks::<2>()
                .0
                .iter()
                .all(|frame| frame[0] == frame[1] * 2.0)
        );

        let mut held = [0.001; 200];
        gate.process(&mut held, 2);
        assert_eq!(held, [0.001; 200]);
        let mut tail = [0.001; 800];
        gate.process(&mut tail, 2);
        assert!(
            tail.iter()
                .any(|sample| (0.0..0.001).contains(sample) && *sample > 0.0)
        );
        assert_eq!(&tail[798..], &[0.0; 2]);
    }

    #[test]
    fn the_compressor_turns_loud_audio_down_by_its_ratio_and_leaves_quiet_audio() {
        let rate = 8_000;
        let timing = Arguments::new(vec![
            ("attack".into(), Value::Duration(1_000_000)),
            ("release".into(), Value::Duration(50_000_000)),
        ]);
        let make = compressor(&config(json!({"threshold_db": -20, "ratio": 4})), &timing).unwrap();
        // A sine at -6 dBFS peak is about -9 dB RMS: 11 dB over, 8.25 dB taken off.
        let mut loud = sine(200.0, 0.5, rate, 0.5);
        make(rate, 1).process(&mut loud, 1);
        let settled = peak(&loud[loud.len() / 2..]);
        let expected = 0.5 * factor_of(-8.25) as f32;
        assert!((settled - expected).abs() < 0.03, "{settled} vs {expected}");
        // Quiet audio is under the threshold and passes as it is.
        let quiet = sine(200.0, 0.01, rate, 0.2);
        let mut processed = quiet.clone();
        make(rate, 1).process(&mut processed, 1);
        assert_eq!(processed, quiet);
        // Makeup gain raises everything.
        let raised = compressor(&config(json!({"makeup_db": 6})), &Arguments::default()).unwrap();
        let mut processed = quiet.clone();
        raised(rate, 1).process(&mut processed, 1);
        assert!((peak(&processed) / peak(&quiet) - 2.0).abs() < 0.01);
    }

    #[test]
    fn the_limiter_keeps_every_sample_under_the_ceiling_and_recovers() {
        let rate = 8_000;
        let timing = Arguments::new(vec![("release".into(), Value::Duration(10_000_000))]);
        let make = limiter(&config(json!({"ceiling_db": -6})), &timing).unwrap();
        let mut limiter = make(rate, 2);
        let ceiling = factor_of(-6.0) as f32;
        let mut loud: Vec<f32> = sine(300.0, 1.0, rate, 0.2)
            .into_iter()
            .flat_map(|sample| [sample, sample * 0.5])
            .collect();
        limiter.process(&mut loud, 2);
        assert!(peak(&loud) <= ceiling + 1e-6);
        // Afterwards quiet audio comes back to its own level.
        let mut quiet: Vec<f32> = sine(300.0, 0.1, rate, 0.3)
            .into_iter()
            .flat_map(|sample| [sample, sample])
            .collect();
        limiter.process(&mut quiet, 2);
        assert!((peak(&quiet[quiet.len() / 2..]) - 0.1).abs() < 0.005);
    }

    #[test]
    fn gain_scales_by_decibels_and_silences_bad_samples() {
        let mut gain = gain(&config(json!({"gain_db": -6}))).unwrap()(8_000, 1);
        let mut samples = [0.5, -0.5, f32::NAN];
        gain.process(&mut samples, 1);
        let half = 0.5 * factor_of(-6.0) as f32;
        assert!((samples[0] - half).abs() < 1e-6);
        assert!((samples[1] + half).abs() < 1e-6);
        assert_eq!(samples[2], 0.0);
        assert!(gain_settings_reject(json!({"gain_db": 40})));
    }

    fn gain_settings_reject(settings: Json) -> bool {
        gain(&config(settings)).is_err()
    }

    #[test]
    fn filters_pass_their_band_and_cut_the_rest() {
        let rate = 48_000;
        let level = |mode: &str, hz: f64| {
            let make = filter(&config(json!({"mode": mode, "cutoff_hz": 1000}))).unwrap();
            let mut samples = sine(hz, 0.5, rate, 0.2);
            make(rate, 1).process(&mut samples, 1);
            peak(&samples[samples.len() / 2..])
        };
        assert!(level("low-pass", 100.0) > 0.45);
        assert!(level("low-pass", 10_000.0) < 0.01);
        assert!(level("high-pass", 10_000.0) > 0.45);
        assert!(level("high-pass", 100.0) < 0.01);
        assert!(level("band-pass", 1_000.0) > 0.45);
        assert!(level("band-pass", 100.0) < 0.1);
        assert!(level("band-pass", 10_000.0) < 0.1);
        assert!(level("notch", 1_000.0) < 0.01);
        assert!(level("notch", 100.0) > 0.45);
    }
}
