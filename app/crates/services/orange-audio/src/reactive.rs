//! Full-stream and multi-band controls built on continuous audio windows.
use crate::{
    Analyzing, BandMeter, Cursor, EmitError, Emitter, InputShared, Lifecycle, Listening, Map,
    Value,
    analysis::{Band, number},
    level::{ChannelMeter, ChannelWindow},
    live_nodes::CHANNEL_LEVELS_TYPE,
    spectrum,
    stream::Ring,
    stream_value, watch_window_measurements,
};
use orange_sdk::{ListRepr, PortKey, RecordRepr};
use serde_json::Value as Json;
use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

pub(crate) const MAX_BANDS: u16 = 12;
const WINDOW: Duration = Duration::from_millis(25);
const PLOT_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Clone, Copy)]
struct Rule {
    threshold: f64,
    margin: f64,
    hold: Duration,
    below: bool,
}

impl Rule {
    fn parse(settings: &Map<String, Json>, below: bool) -> Result<Self, String> {
        let threshold = number(settings, "threshold_db", -35.0)?;
        let margin = number(settings, "reset_db", 4.0)?;
        let hold = number(settings, "hold_ms", 150.0)?;
        if !(-120.0..=0.0).contains(&threshold)
            || !(0.0..=24.0).contains(&margin)
            || !(0.0..=10_000.0).contains(&hold)
        {
            return Err("Threshold, reset margin or hold time is outside its range".into());
        }
        Ok(Self {
            threshold,
            margin,
            hold: Duration::from_secs_f64(hold / 1_000.0),
            below,
        })
    }
}

#[derive(Default)]
struct Crossing {
    elapsed: Duration,
    active: bool,
    pending: bool,
}

impl Crossing {
    fn update(&mut self, rule: Rule, level: f64) {
        let difference = if rule.below {
            rule.threshold - level
        } else {
            level - rule.threshold
        };
        if self.active {
            if difference < 0.0 && difference <= -rule.margin {
                *self = Self::default();
            }
        } else if difference >= 0.0 {
            self.elapsed += WINDOW;
            if self.elapsed >= rule.hold {
                self.active = true;
                self.pending = true;
            }
        } else {
            self.elapsed = Duration::ZERO;
        }
    }

    fn send(&mut self, emit: impl FnOnce() -> Result<(), EmitError>) -> bool {
        if self.pending {
            match emit() {
                Ok(()) => self.pending = false,
                Err(EmitError::Busy) => {}
                Err(EmitError::Closed) => return false,
            }
        }
        true
    }
}

fn port(name: &str) -> Result<PortKey, String> {
    name.parse()
        .map_err(|_| format!("Invalid analyzer port {name}"))
}

pub(crate) fn noise(
    input: Arc<InputShared>,
    settings: &Map<String, Json>,
    emitter: Emitter,
) -> Result<Box<dyn Send>, String> {
    let below = match settings.get("direction").and_then(Json::as_str) {
        None | Some("above") => false,
        Some("below") => true,
        Some(_) => return Err("Trigger direction must be above or below".into()),
    };
    let rule = Rule::parse(settings, below)?;
    let level_port = port("level")?;
    let state_port = port("condition_met")?;
    let channels_port = port("channel_levels")?;
    let channels_type = CHANNEL_LEVELS_TYPE
        .parse()
        .map_err(|_| "Invalid channel levels type")?;
    let trigger = port("then")?;
    let mut gate = Crossing::default();
    watch_window_measurements(
        input,
        WINDOW,
        |ring| {
            let listening = Listening {
                low_hz: 0.0,
                high_hz: 0.0,
                window: WINDOW,
            };
            ChannelMeter::new(listening.frames(ring.rate), ring.channels)
        },
        |meter, samples, report| meter.push(samples, report),
        move |window: Option<ChannelWindow>| {
            let level = window.as_ref().map(|window| window.level);
            if let Some(level) = level {
                gate.update(rule, level);
            } else {
                gate = Crossing::default();
            }
            emitter.publish(&level_port, level.map(Value::F64)).is_ok()
                && emitter
                    .publish(
                        &channels_port,
                        window.map(|window| {
                            let list = |repr| Some(Value::List(Arc::new(repr)));
                            Value::Record(Arc::new(RecordRepr {
                                ty: Clone::clone(&channels_type),
                                fields: Box::new([
                                    list(ListRepr::F64(window.rms)),
                                    list(ListRepr::F64(window.peak)),
                                    list(ListRepr::Bool(window.clipped)),
                                ]),
                            }))
                        }),
                    )
                    .is_ok()
                && emitter
                    .publish(&state_port, Some(Value::Bool(gate.active)))
                    .is_ok()
                && gate.send(|| emitter.emit_on(&trigger))
        },
    )
}

struct BandSpec {
    listening: Listening,
    rule: Rule,
    level: Option<PortKey>,
    above: Option<PortKey>,
    below: Option<PortKey>,
    output: Option<BandOutput>,
}

struct BandOutput {
    port: PortKey,
    source: Arc<InputShared>,
}

impl Drop for BandOutput {
    fn drop(&mut self) {
        self.source
            .stop
            .store(true, std::sync::atomic::Ordering::Release);
        self.source.replace_ring(None);
    }
}

fn bands(settings: &Map<String, Json>, streams: bool) -> Result<Vec<BandSpec>, String> {
    let Some(value) = settings.get("bands") else {
        return Ok(Vec::new());
    };
    let bands = value.as_object().ok_or("Bands must be an object")?;
    if bands.len() > usize::from(MAX_BANDS) {
        return Err(format!("Spectrum supports up to {MAX_BANDS} bands"));
    }
    bands
        .iter()
        .map(|(id, band)| {
            let id = id.parse::<u16>().map_err(|_| "Invalid band identity")?;
            let band = band.as_object().ok_or("Invalid band settings")?;
            let mut listening = Listening::of(band, (0.0, 0.0, 25.0))?;
            if listening.low_hz < 20.0
                || listening.high_hz > 24_000.0
                || listening.high_hz <= listening.low_hz
            {
                return Err("Band needs two ordered frequencies from 20 to 24000 Hz".into());
            }
            listening.window = WINDOW;
            let route = |field: &str, base: &str| {
                band.get(field)
                    .map(|value| {
                        let route = value
                            .as_u64()
                            .and_then(|id| u16::try_from(id).ok())
                            .ok_or_else(|| format!("Invalid {field} for band {id}"))?;
                        port(&format!("{base}#{route}"))
                    })
                    .transpose()
            };
            Ok(BandSpec {
                listening,
                rule: Rule::parse(band, false)?,
                level: (!streams)
                    .then(|| port(&format!("band_level#{id}")))
                    .transpose()?,
                above: route("above_port", "band_above")?,
                below: route("below_port", "band_below")?,
                output: if streams {
                    Some(BandOutput {
                        port: port(&format!("band_stream#{id}"))?,
                        source: InputShared::register(None, None, WINDOW),
                    })
                } else {
                    None
                },
            })
        })
        .collect()
}

struct BandTracker<'a> {
    spec: &'a BandSpec,
    meter: BandMeter,
    above: Crossing,
    below: Crossing,
    below_armed: bool,
    filtered: Option<(Arc<Ring>, Vec<Band>)>,
}

impl BandTracker<'_> {
    fn push(&mut self, samples: &[f32], emitter: &Emitter) -> bool {
        if let Some((ring, filters)) = &mut self.filtered {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "PCM samples use the ring's f32 format"
            )]
            ring.push(
                samples
                    .chunks_exact(filters.len())
                    .flat_map(|frame| frame.iter())
                    .enumerate()
                    .map(|(i, &sample)| {
                        let channel = i % filters.len();
                        filters[channel].process(f64::from(sample)) as f32
                    }),
            );
        }
        let Some(level_port) = &self.spec.level else {
            return true;
        };
        let mut alive = true;
        self.meter.push(samples, |level| {
            if !alive {
                return;
            }
            let was_above = self.above.active;
            self.above.update(self.spec.rule, level);
            if !was_above && self.above.active {
                self.below = Crossing::default();
                self.below_armed = true;
            }
            if self.below_armed {
                self.below.update(
                    Rule {
                        below: true,
                        ..self.spec.rule
                    },
                    level,
                );
                self.below_armed = !self.below.active;
            }
            alive = emitter.publish(level_port, Some(Value::F64(level))).is_ok()
                && self.above.send(|| {
                    self.spec
                        .above
                        .as_ref()
                        .map_or(Ok(()), |port| emitter.emit_on(port))
                })
                && self.below.send(|| {
                    self.spec
                        .below
                        .as_ref()
                        .map_or(Ok(()), |port| emitter.emit_on(port))
                });
        });
        alive
    }
}

pub(crate) fn spectrum(
    input: Arc<InputShared>,
    settings: &Map<String, Json>,
    emitter: Emitter,
    streams: bool,
    run_id: u64,
) -> Result<Box<dyn Send>, String> {
    let specs = bands(settings, streams)?;
    for output in specs.iter().filter_map(|spec| spec.output.as_ref()) {
        let value =
            stream_value(&output.source, run_id).map_err(|fault| fault.message.to_owned())?;
        emitter
            .publish(&output.port, Some(value))
            .map_err(|_| "Band output could not be published")?;
    }
    let plot = port("spectrum")?;
    let life = Arc::new(Lifecycle::new());
    let running = Arc::clone(&life);
    thread::Builder::new()
        .name("orange-audio-multiband".into())
        .spawn(move || {
            let mut transform = spectrum::Analyzer::new();
            let mut block = vec![0.0f32; 8192];
            let clear = || {
                emitter.publish(&plot, None).is_ok()
                    && specs
                        .iter()
                        .filter_map(|spec| spec.level.as_ref())
                        .all(|port| emitter.publish(port, None).is_ok())
            };
            while let Ok(ring) = input.wait_ring(None, &|| running.stopped()) {
                let channels = usize::from(ring.channels.max(1));
                let size = spectrum::FRAMES * channels;
                let mut samples = Vec::with_capacity(size);
                let mut last_plot = None;
                let new_trackers = || {
                    specs
                        .iter()
                        .filter(|spec| spec.listening.low_hz < f64::from(ring.rate) * 0.475)
                        .map(|spec| {
                            let filtered = spec.output.as_ref().map(|output| {
                                let filtered = Ring::new(ring.rate, ring.channels);
                                output.source.replace_ring(Some(Arc::clone(&filtered)));
                                let filter = Band::isolated(
                                    ring.rate,
                                    spec.listening.low_hz,
                                    spec.listening.high_hz,
                                );
                                let bands = vec![filter; channels];
                                (filtered, bands)
                            });
                            BandTracker {
                                spec,
                                meter: BandMeter::new(&ring, &spec.listening),
                                above: Crossing::default(),
                                below: Crossing::default(),
                                below_armed: false,
                                filtered,
                            }
                        })
                        .collect::<Vec<_>>()
                };
                let mut trackers = new_trackers();
                let mut cursor = Cursor::new(Arc::clone(&ring));
                while !running.stopped() {
                    let previous = cursor.dropped;
                    let read = cursor.read(&mut block);
                    if read == 0 {
                        if ring.closed() {
                            break;
                        }
                        let _ = running.rest(Duration::from_millis(5));
                        continue;
                    }
                    if previous != cursor.dropped {
                        if !clear() {
                            return;
                        }
                        trackers = new_trackers();
                        samples.clear();
                    }
                    if !trackers
                        .iter_mut()
                        .all(|band| band.push(&block[..read], &emitter))
                    {
                        return;
                    }
                    let mut remaining = &block[..read];
                    while !remaining.is_empty() {
                        let count = (size - samples.len()).min(remaining.len());
                        samples.extend_from_slice(&remaining[..count]);
                        remaining = &remaining[count..];
                        if samples.len() == size {
                            let now = Instant::now();
                            if last_plot.is_none_or(|last| now - last >= PLOT_INTERVAL) {
                                let values = transform.measure(&samples, channels, ring.rate);
                                if emitter
                                    .publish(
                                        &plot,
                                        Some(Value::List(Arc::new(ListRepr::F64(Box::new(
                                            values,
                                        ))))),
                                    )
                                    .is_err()
                                {
                                    return;
                                }
                                last_plot = Some(now);
                            }
                            samples.clear();
                        }
                    }
                }
                for output in specs.iter().filter_map(|spec| spec.output.as_ref()) {
                    output.source.replace_ring(None);
                }
                if !clear() {
                    return;
                }
            }
        })
        .map_err(|error| error.to_string())?;
    Ok(Box::new(Analyzing(life)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn below_uses_completed_windows_and_rearms_only_after_the_margin() {
        let rule = Rule {
            threshold: -35.0,
            margin: 4.0,
            hold: WINDOW * 2,
            below: true,
        };
        let mut gate = Crossing::default();
        let crossings: Vec<bool> = [
            -80.0, -25.0, -80.0, -80.0, -32.0, -80.0, -30.0, -80.0, -80.0,
        ]
        .into_iter()
        .map(|db| {
            gate.update(rule, db);
            let pending = gate.pending;
            assert!(gate.send(|| Ok(())));
            pending
        })
        .collect();
        assert_eq!(
            crossings,
            [false, false, false, true, false, false, false, false, true]
        );
    }
}
