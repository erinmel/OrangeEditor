//! Live audio nodes and stream-wired entries. Only source identity, format and
//! measured values cross the run boundary. Sample blocks remain in the shared ring.
use crate::{
    Analyzing, BAD_INPUT, Call, Control, DEVICE_CHECK, Definition, EmitError, Emitter,
    INPUT_ADAPTER, InputShared, KernelFault, Lifecycle, Listening, Map, OUTPUT_ADAPTER,
    OutputSession, PACKAGE, RecordRepr, SILENCE_DB, STREAM_TYPE, ServiceNode, ServicePackage,
    ServiceSource, Session, SourceControl, Stop, Value, input_of, number_setting, play_stream,
    source, stream_gain, stream_input, stream_value, transient, transient_settings, unit_gain,
    watch_windows, when_finished,
};
use orange_sdk::{Arguments, DefinitionError, LiveSource, Publisher};
use serde_json::{Value as Json, json};
use std::sync::Arc;
use std::thread;

/// Publishes an input connection's shared audio stream.
pub const AUDIO_SOURCE: &str = "org.orange.audio/audio-source";
/// Reports the stream's negotiated rate and number of channels.
pub const AUDIO_FORMAT: &str = "org.orange.audio/audio-format";
/// Continuous frequency plot and stable, user-defined band outputs.
pub const MULTIBAND_SPECTRUM: &str = "org.orange.audio/multiband-spectrum";
/// Full-stream RMS, threshold state and a configurable trigger.
pub const NOISE_LEVEL: &str = "org.orange.audio/noise-level";
/// Attenuates quiet samples on a derived audio stream.
pub const NOISE_GATE: &str = "org.orange.audio/noise-gate";
/// Turns loud parts of a stream down by a ratio above a threshold.
pub const COMPRESSOR: &str = "org.orange.audio/compressor";
/// Keeps a stream's peaks under a ceiling.
pub const LIMITER: &str = "org.orange.audio/limiter";
/// Makes a stream louder or quieter by a number of decibels.
pub const GAIN: &str = "org.orange.audio/gain";
/// Keeps the low, high or middle frequencies of a stream.
pub const FILTER: &str = "org.orange.audio/filter";
/// The record output of Audio format, with U64 fields `rate` and `channels`.
pub const FORMAT_TYPE: &str = "org.orange.audio/format@1";
/// Measures a stream's level in dBFS continuously.
pub const LIVE_LEVEL: &str = "org.orange.audio/live-level";
/// Fires when a stream has a hit in a selected band.
pub const TRANSIENT: &str = "org.orange.audio/transient";
/// Fires when the wired file source plays to its end.
pub const SOURCE_FINISHED: &str = "org.orange.audio/source-finished";
/// Controls how a wired file source plays.
pub const SOURCE_CONTROL: &str = "org.orange.audio/source-control";
/// Controls what plays on an output connection.
pub const SOUND_CONTROL: &str = "org.orange.audio/sound-control";

const LEVEL_WINDOW_MS: f64 = 20.0;
const F64: &str = "core/f64@1";
const SOURCE_COMMAND: &str = "org.orange.audio/source-command@1";
const SOUND_COMMAND: &str = "org.orange.audio/sound-command@1";

// Tags are this schema order. Keeping the table beside dispatch makes a change to
// either a deliberate contract edit.
const SOURCE_COMMANDS: &[(&str, Option<&str>)] = &[
    ("play", None),
    ("pause", None),
    ("resume", None),
    ("stop", None),
    ("jump", None),
];
const SOUND_COMMANDS: &[(&str, Option<&str>)] = &[
    ("stop", None),
    ("pause", None),
    ("resume", None),
    ("jump", None),
    ("volume", None),
];

fn input_stream(arguments: &Arguments) -> Result<Arc<InputShared>, String> {
    source(
        arguments
            .get("stream")
            .ok_or("the entry needs an audio stream")?,
    )
    .map_err(|fault| fault.message.to_string())
}

struct AudioSource;
impl LiveSource for AudioSource {
    fn start(
        &self,
        session: Option<Arc<dyn Session>>,
        _settings: &Map<String, Json>,
        _arguments: &Arguments,
        publisher: Publisher,
    ) -> Result<Box<dyn Send>, String> {
        let session = input_of(session.as_ref())?;
        let value = stream_value(session.id).map_err(|fault| fault.message.to_string())?;
        let life = Arc::new(Lifecycle::new());
        let running = Arc::clone(&life);
        thread::Builder::new()
            .name("orange-audio-source".into())
            .spawn(move || {
                if session.wait_ring(None, &|| running.stopped()).is_ok() && !running.stopped() {
                    let _ = publisher.publish(value);
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Box::new(Analyzing(life)))
    }
}

/// A stream processor: its worker's name and how its settings build it.
type MakeProcessor = fn(&Map<String, Json>, &Arguments) -> Result<crate::processors::Make, String>;
struct Processor(&'static str, MakeProcessor);
impl LiveSource for Processor {
    fn start(
        &self,
        _session: Option<Arc<dyn Session>>,
        settings: &Map<String, Json>,
        arguments: &Arguments,
        publisher: Publisher,
    ) -> Result<Box<dyn Send>, String> {
        let Self(name, make) = self;
        crate::processors::start(
            name,
            input_stream(arguments)?,
            make(settings, arguments)?,
            publisher,
        )
    }
}

struct LiveAudioOut;
impl ServiceSource for LiveAudioOut {
    fn start_with_arguments(
        &self,
        session: Option<Arc<dyn Session>>,
        _settings: &Map<String, Json>,
        arguments: &Arguments,
        emitter: Emitter,
    ) -> Result<Box<dyn Send>, String> {
        let playing = "playing"
            .parse()
            .map_err(|_| "invalid playback status port")?;
        let Some(Value::Bool(enabled)) = arguments.get("enabled") else {
            return Err("audio output needs an enabled value".to_owned());
        };
        let _ = emitter.publish(&playing, Some(Value::Bool(false)));
        if !*enabled {
            return Ok(Box::new(()));
        }
        let input = input_stream(arguments)?;
        let Some(Value::F64(gain)) = arguments.get("gain") else {
            return Err("audio output needs a gain value".to_owned());
        };
        let gain = stream_gain(*gain).map_err(|fault| fault.message.to_string())?;
        let session = session.ok_or("choose an audio output device")?;
        if !session.as_any().is::<OutputSession>() {
            return Err("choose an audio output device".to_owned());
        }
        let life = Arc::new(Lifecycle::new());
        let running = Arc::clone(&life);
        thread::Builder::new()
            .name("orange-live-audio-out".into())
            .spawn(move || {
                let Some(output) = session.as_any().downcast_ref::<OutputSession>() else {
                    return;
                };
                let result = play_stream(
                    output,
                    &input,
                    gain,
                    || running.stopped(),
                    |active| {
                        let _ = emitter.publish(&playing, Some(Value::Bool(active)));
                    },
                );
                if let Err(fault) = result {
                    let _ = emitter.fault(fault);
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Box::new(Analyzing(life)))
    }
}

struct AudioFormat;
impl LiveSource for AudioFormat {
    fn start(
        &self,
        _session: Option<Arc<dyn Session>>,
        _settings: &Map<String, Json>,
        arguments: &Arguments,
        publisher: Publisher,
    ) -> Result<Box<dyn Send>, String> {
        let input = input_stream(arguments)?;
        let ty = FORMAT_TYPE
            .parse()
            .map_err(|_| "invalid audio format type")?;
        let life = Arc::new(Lifecycle::new());
        let running = Arc::clone(&life);
        thread::Builder::new()
            .name("orange-audio-format".into())
            .spawn(move || {
                let mut previous = None;
                while !running.stopped() {
                    let Ok(ring) = input.wait_ring(None, &|| running.stopped()) else {
                        return;
                    };
                    let format = (ring.rate, ring.channels);
                    if previous != Some(format) {
                        let value = Value::Record(Arc::new(RecordRepr {
                            ty: Clone::clone(&ty),
                            fields: Box::new([
                                Some(Value::U64(u64::from(ring.rate))),
                                Some(Value::U64(u64::from(ring.channels))),
                            ]),
                        }));
                        if publisher.publish(value) == Err(EmitError::Closed) {
                            return;
                        }
                        previous = Some(format);
                    }
                    if !running.rest(DEVICE_CHECK) {
                        return;
                    }
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Box::new(Analyzing(life)))
    }
}

struct Meter {
    band: bool,
}

fn band_listening(
    settings: &Map<String, Json>,
    arguments: &Arguments,
    window: f64,
) -> Result<Listening, String> {
    let hertz = |name| match arguments.get(name) {
        Some(Value::F64(value)) if (0.0..=24_000.0).contains(value) => Ok(*value),
        _ => Err(format!("{name} must be between 0 and 24000 Hz")),
    };
    let (low_hz, high_hz) = (hertz("low_hz")?, hertz("high_hz")?);
    if high_hz > 0.0 && low_hz >= high_hz {
        return Err("the band needs low_hz < high_hz, or 0 to leave a side open".into());
    }
    let mut listening = Listening::of(settings, (0.0, 0.0, window))?;
    listening.low_hz = low_hz;
    listening.high_hz = high_hz;
    Ok(listening)
}

impl LiveSource for Meter {
    fn start(
        &self,
        _session: Option<Arc<dyn Session>>,
        settings: &Map<String, Json>,
        arguments: &Arguments,
        publisher: Publisher,
    ) -> Result<Box<dyn Send>, String> {
        let stream = input_stream(arguments)?;
        let listening = if self.band {
            band_listening(settings, arguments, LEVEL_WINDOW_MS)?
        } else {
            Listening::of(settings, (0.0, 0.0, LEVEL_WINDOW_MS))?
        };
        watch_windows(stream, listening, move |level| {
            publisher.publish(Value::F64(level.unwrap_or(SILENCE_DB))) != Err(EmitError::Closed)
        })
    }
}

struct ReactiveAudio {
    spectrum: bool,
    streams: bool,
}
impl ServiceSource for ReactiveAudio {
    fn start_with_arguments(
        &self,
        _session: Option<Arc<dyn Session>>,
        settings: &Map<String, Json>,
        arguments: &Arguments,
        emitter: Emitter,
    ) -> Result<Box<dyn Send>, String> {
        let stream = input_stream(arguments)?;
        if self.spectrum {
            crate::reactive::spectrum(stream, settings, emitter, self.streams)
        } else {
            crate::reactive::noise(stream, settings, emitter)
        }
    }
}

struct Transient;
impl ServiceSource for Transient {
    fn start_with_arguments(
        &self,
        _session: Option<Arc<dyn Session>>,
        settings: &Map<String, Json>,
        arguments: &Arguments,
        emitter: Emitter,
    ) -> Result<Box<dyn Send>, String> {
        transient(input_stream(arguments)?, settings, emitter)
    }
}

struct SourceFinished;
impl ServiceSource for SourceFinished {
    fn start_with_arguments(
        &self,
        _session: Option<Arc<dyn Session>>,
        _settings: &Map<String, Json>,
        arguments: &Arguments,
        emitter: Emitter,
    ) -> Result<Box<dyn Send>, String> {
        when_finished(input_stream(arguments)?, emitter)
    }
}

struct SourceTransport;
impl ServiceNode for SourceTransport {
    fn run(&self, call: &Call<'_>) -> Result<Vec<Value>, KernelFault> {
        let [stream, Value::Enum { tag, payload: None }, seconds] = call.inputs else {
            return Err(BAD_INPUT);
        };
        let control = match tag {
            0 => SourceControl::Play,
            1 => SourceControl::Pause,
            2 => SourceControl::Resume,
            3 => SourceControl::Stop,
            4 => SourceControl::Seek,
            _ => return Err(BAD_INPUT),
        };
        let input = source(stream)?;
        control.apply(&input, std::slice::from_ref(seconds))
    }
}

struct SoundControl;
impl ServiceNode for SoundControl {
    fn run(&self, call: &Call<'_>) -> Result<Vec<Value>, KernelFault> {
        let [Value::Enum { tag, payload: None }, seconds, gain] = call.inputs else {
            return Err(BAD_INPUT);
        };
        let (control, value) = match tag {
            0 => return Stop.run(call),
            1 => (Control::Pause, &[][..]),
            2 => (Control::Resume, &[][..]),
            3 => (Control::Seek, std::slice::from_ref(seconds)),
            4 => (Control::Volume, std::slice::from_ref(gain)),
            _ => return Err(BAD_INPUT),
        };
        control.run(&Call {
            inputs: value,
            session: call.session,
            lease: call.lease,
        })
    }
}

fn zero() -> Json {
    json!({"type": F64, "bits": "0000000000000000"})
}

fn choice(ty: &str, variant: &str) -> Json {
    json!({"type": ty, "value": {"variant": variant}})
}

fn live(name: &str, label: &str, description: &str) -> Definition {
    Definition::read(PACKAGE, name, label)
        .description(description)
        .category("audio", "Audio")
}

fn entry(name: &str, label: &str, description: &str) -> Definition {
    stream_input(
        Definition::entry(PACKAGE, name, label)
            .description(description)
            .category("audio", "Audio"),
    )
}

fn action(name: &str, label: &str, description: &str) -> Definition {
    Definition::node(PACKAGE, name, label)
        .description(description)
        .category("audio", "Audio")
}

fn meter(name: &str, label: &str, description: &str) -> Definition {
    stream_input(live(name, label, description))
        .setting("window_ms", number_setting(5.0, 1_000.0, LEVEL_WINDOW_MS))
        .live("level", F64)
}

pub(crate) fn add(package: ServicePackage) -> Result<ServicePackage, DefinitionError> {
    let audio_source = live("audio-source", "Audio source", "Gives the audio stream of a device or a sound file. Wire it to an analyzer, an output or a recorder.")
        .keywords("audio in, audio input, microphone, application, system audio, virtual cable, file")
        .uses(INPUT_ADAPTER)
        .live("stream", STREAM_TYPE);
    let audio_out = entry(
        "audio-out",
        "Audio out",
        "Plays the wired audio stream on the selected device while Enabled is on.",
    )
    .version(2)
    .uses(OUTPUT_ADAPTER)
    .keywords("speaker, monitor, output, listen, mute, toggle")
    .input_default(
        "enabled",
        "core/bool@1",
        json!({"type": "core/bool@1", "value": true}),
    )
    .input_default("gain", F64, unit_gain())
    .live("playing", "core/bool@1")
    .source_outputs("finished", None, 0);
    let audio_format = stream_input(live(
        "audio-format",
        "Audio format",
        "Reports the rate and channel count of a stream.",
    ))
    .live("format", FORMAT_TYPE);
    let frequency_splitter = entry(
        "multiband-spectrum",
        "Frequency Splitter",
        "Drag across the spectrum to create a filtered audio output for that frequency range.",
    )
    .version(3)
    .keywords("fft, bandpass, bass, multiband, frequency, audio spectrum, band trigger, audio filter, split audio")
    .setting("bands", json!({"type": "object"}))
    .live("spectrum", "core/list@1<core/f64@1>")
    .source_outputs("band_stream", Some(STREAM_TYPE), crate::reactive::MAX_BANDS)
    // A passive source has no execution outcome, even with no bands configured.
    .source_outputs("band_above", None, 0);
    let noise = entry(
        "noise-level",
        "Audio Level Trigger",
        "Measure overall volume and trigger after staying above or below a threshold.",
    )
    .keywords("microphone, noise level, loudness, volume, rms, dbfs, threshold")
    .then_label("Triggered")
    .live("level", F64)
    .live("condition_met", "core/bool@1")
    .setting(
        "direction",
        json!({"type": "string", "enum": ["above", "below"], "default": "above"}),
    )
    .setting("threshold_db", number_setting(-120.0, 0.0, -35.0))
    .setting("hold_ms", number_setting(0.0, 10_000.0, 150.0))
    .setting("reset_db", number_setting(0.0, 24.0, 4.0));
    let gate = crate::processors::gate_settings(stream_input(live(
        "noise-gate",
        "Noise Gate",
        "Mutes quiet audio and outputs a gated stream for playback or recording.",
    )))
    .keywords("microphone, noise gate, mute, silence, threshold, audio filter, background noise")
    .live("gated_stream", STREAM_TYPE);
    let compressor = crate::processors::compressor_settings(stream_input(live(
        "compressor",
        "Compressor",
        "Turns the loud parts of a stream down so its level stays even. Ratio sets how much, Attack and Release how fast, and Makeup raises the result again.",
    )))
    .keywords("compression, dynamics, loudness, even out, level, voice, audio effect")
    .live("compressed_stream", STREAM_TYPE);
    let limiter = crate::processors::limiter_settings(stream_input(live(
        "limiter",
        "Limiter",
        "Keeps a stream's peaks under the ceiling so it never clips.",
    )))
    .keywords("clip, clipping, peak, ceiling, protect, loudness, audio effect")
    .live("limited_stream", STREAM_TYPE);
    let gain = crate::processors::gain_settings(stream_input(live(
        "gain",
        "Gain",
        "Makes a stream louder or quieter by a number of decibels.",
    )))
    .keywords("volume, louder, quieter, amplify, boost, attenuate, decibels, audio effect")
    .live("adjusted_stream", STREAM_TYPE);
    let filter = crate::processors::filter_settings(stream_input(live(
        "filter",
        "Filter",
        "Keeps the low (low-pass), high (high-pass) or middle (band-pass) frequencies of a stream around the cutoff.",
    )))
    .keywords("eq, equalizer, low pass, high pass, band pass, bass, treble, frequency, audio effect")
    .live("filtered_stream", STREAM_TYPE);
    let level = meter(
        "live-level",
        "Level",
        "Measures a stream's level in dBFS as it changes.",
    )
    .keywords("volume, loudness, meter");
    let transient = transient_settings(
        entry(
            "transient",
            "Transient",
            "Fires at each hit in a stream's frequency band, such as a kick drum.",
        )
        .keywords("kick, beat, drum, hit, onset, rhythm")
        .payload(F64),
    );
    let finished = entry(
        "source-finished",
        "Source finished",
        "Fires when the wired sound file plays to its end.",
    )
    .keywords("end, done, track over");
    let transport = stream_input(action(
        "source-control",
        "Source control",
        "Plays, pauses, resumes, stops or moves the wired sound file to a position.",
    ))
    .input_default("command", SOURCE_COMMAND, choice(SOURCE_COMMAND, "play"))
    .input_default("seconds", F64, zero());
    let sound = action(
        "sound-control",
        "Sound control",
        "Stops, pauses, resumes, moves or changes the volume of sounds on an output.",
    )
    .uses(OUTPUT_ADAPTER)
    .input_default("command", SOUND_COMMAND, choice(SOUND_COMMAND, "stop"))
    .input_default("seconds", F64, zero())
    .input_default("gain", F64, unit_gain());
    package
        .record(
            "format",
            &[("rate", "core/u64@1"), ("channels", "core/u64@1")],
        )
        .enumeration("source-command", SOURCE_COMMANDS)
        .enumeration("sound-command", SOUND_COMMANDS)
        .live(&audio_source.build(), AudioSource)?
        .live(
            &gate.build(),
            Processor("noise-gate", crate::processors::gate),
        )?
        .live(
            &compressor.build(),
            Processor("compressor", crate::processors::compressor),
        )?
        .live(
            &limiter.build(),
            Processor("limiter", crate::processors::limiter),
        )?
        .live(
            &gain.build(),
            Processor("gain", |settings, _| crate::processors::gain(settings)),
        )?
        .live(
            &filter.build(),
            Processor("filter", |settings, _| crate::processors::filter(settings)),
        )?
        .source(&audio_out.build(), LiveAudioOut)?
        .live(&audio_format.build(), AudioFormat)?
        .live(&level.build(), Meter { band: false })?
        .source(
            &frequency_splitter.build(),
            ReactiveAudio {
                spectrum: true,
                streams: true,
            },
        )?
        .source(
            &noise.build(),
            ReactiveAudio {
                spectrum: false,
                streams: false,
            },
        )?
        .source(&transient.build(), Transient)?
        .source(&finished.build(), SourceFinished)?
        .node(&transport.build(), SourceTransport)?
        .node(&sound.build(), SoundControl)
}
