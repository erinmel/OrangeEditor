//! Applies a linked-channel gate on a worker, producing another routable PCM stream.
use crate::{
    Analyzing, Cursor, Definition, EmitError, InputShared, Lifecycle, Map, SILENCE_DB, Value,
    analysis::number, number_setting, stream::Ring, stream_value,
};
use orange_sdk::Publisher;
use serde_json::Value as Json;
use std::{
    sync::{Arc, atomic::Ordering},
    thread,
    time::Duration,
};

#[derive(Clone, Copy)]
struct Setting(&'static str, f64, f64, f64);

const THRESHOLD: Setting = Setting("threshold_db", SILENCE_DB, 0.0, -40.0);
const RESET: Setting = Setting("reset_db", 0.0, 24.0, 4.0);
const ATTACK: Setting = Setting("attack_ms", 0.0, 250.0, 5.0);
const HOLD: Setting = Setting("hold_ms", 0.0, 2_000.0, 100.0);
const RELEASE: Setting = Setting("release_ms", 0.0, 2_000.0, 80.0);

pub(crate) fn settings(definition: Definition) -> Definition {
    [THRESHOLD, RESET, ATTACK, HOLD, RELEASE].into_iter().fold(
        definition,
        |definition, Setting(key, min, max, default)| {
            definition.setting(key, number_setting(min, max, default))
        },
    )
}

#[derive(Clone, Copy)]
struct Settings {
    open_power: f64,
    close_power: f64,
    attack_ms: f64,
    hold_ms: f64,
    release_ms: f64,
}

impl Settings {
    fn read(config: &Map<String, Json>) -> Result<Self, String> {
        let read = |Setting(key, min, max, default)| {
            let value = number(config, key, default)?;
            if !(min..=max).contains(&value) {
                return Err(format!("`{key}` is outside its range"));
            }
            Ok(value)
        };
        let threshold = read(THRESHOLD)?;
        let margin = read(RESET)?;
        Ok(Self {
            open_power: 10.0_f64.powf(threshold / 10.0),
            close_power: 10.0_f64.powf((threshold - margin).max(SILENCE_DB) / 10.0),
            attack_ms: read(ATTACK)?,
            hold_ms: read(HOLD)?,
            release_ms: read(RELEASE)?,
        })
    }
}

struct Gate {
    settings: Settings,
    power: f64,
    smoothing: f64,
    open: bool,
    hold_frames: f64,
    remaining: f64,
    gain: f64,
    up: f64,
    down: f64,
}

impl Gate {
    fn new(settings: Settings, rate: u32) -> Self {
        let frames = |ms: f64| ms * f64::from(rate) / 1_000.0;
        let step = |ms| 1.0 / frames(ms).max(1.0);
        Self {
            settings,
            power: 0.0,
            smoothing: 1.0 - (-step(10.0)).exp(),
            open: false,
            hold_frames: frames(settings.hold_ms).ceil(),
            remaining: 0.0,
            gain: 0.0,
            up: step(settings.attack_ms),
            down: step(settings.release_ms),
        }
    }

    #[expect(clippy::cast_possible_truncation, reason = "bounded PCM gain in 0..=1")]
    fn process(&mut self, samples: &mut [f32], channels: u16) {
        for frame in samples.chunks_exact_mut(usize::from(channels)) {
            let mut power = 0.0;
            for sample in &mut *frame {
                if !sample.is_finite() {
                    *sample = 0.0;
                }
                power += f64::from(*sample).powi(2);
            }
            power /= f64::from(channels);
            self.power += (power - self.power) * self.smoothing;

            if self.power > self.settings.open_power
                || (self.open && self.power > self.settings.close_power)
            {
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
            for sample in frame {
                *sample *= self.gain as f32;
            }
        }
    }
}

struct Output(Arc<InputShared>);

impl Drop for Output {
    fn drop(&mut self) {
        self.0.stop.store(true, Ordering::Release);
        self.0.replace_ring(None);
    }
}

pub(crate) fn start(
    input: Arc<InputShared>,
    config: &Map<String, Json>,
    publisher: Publisher,
) -> Result<Box<dyn Send>, String> {
    let settings = Settings::read(config)?;
    let output = Output(InputShared::register(None, None, Duration::from_millis(25)));
    let value = stream_value(output.0.id).map_err(|fault| fault.message.to_owned())?;
    let life = Arc::new(Lifecycle::new());
    let running = Arc::clone(&life);
    thread::Builder::new()
        .name("orange-audio-noise-gate".into())
        .spawn(move || process(&input, &output, settings, &running, &publisher, value))
        .map_err(|error| error.to_string())?;
    Ok(Box::new(Analyzing(life)))
}

fn process(
    input: &InputShared,
    output: &Output,
    settings: Settings,
    life: &Lifecycle,
    publisher: &Publisher,
    value: Value,
) {
    let mut block = vec![0.0; 8192];
    let mut announced = false;
    while let Ok(ring) = input.wait_ring(None, &|| life.stopped()) {
        let channels = ring.channels;
        let mut cursor = Cursor::new(Arc::clone(&ring));
        let gated = Ring::new(ring.rate, channels);
        output.0.replace_ring(Some(Arc::clone(&gated)));
        let mut gate = Gate::new(settings, ring.rate);
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
                gate = Gate::new(settings, ring.rate);
            }
            let samples = &mut block[..read];
            gate.process(samples, channels);
            gated.push(samples.iter().copied());
        }
        output.0.replace_ring(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gates_quiet_pcm_and_uses_the_same_gain_for_both_channels() {
        let mut gate = Gate::new(Settings::read(&Map::new()).unwrap(), 1_000);
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
}
