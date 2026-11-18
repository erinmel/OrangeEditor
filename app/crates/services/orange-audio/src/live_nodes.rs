//! Live audio nodes and stream-wired entries. Only source identity, format and
//! measured values cross the run boundary. Sample blocks remain in the shared ring.
use crate::{
    Analyzing, BAD_INPUT, Call, Control, DEVICE_CHECK, Definition, EmitError, Emitter,
    INPUT_ADAPTER, InputShared, KernelFault, Lifecycle, Listening, Map, OUTPUT_ADAPTER, PACKAGE,
    RecordRepr, SILENCE_DB, STREAM_TYPE, ServiceNode, ServicePackage, ServiceSource, Session,
    SourceControl, Stop, Value, input_of, number_setting, source, stream_input, stream_value,
    transient, transient_settings, unit_gain, watch_windows, when_finished,
};
use orange_sdk::{Arguments, DefinitionError, LiveSource, Publisher};
use serde_json::{Value as Json, json};
use std::sync::Arc;
use std::thread;

/// Publishes an input connection's shared audio stream.
pub const AUDIO_SOURCE: &str = "org.orange.audio/audio-source";
/// Reports the stream's negotiated rate and number of channels.
pub const AUDIO_FORMAT: &str = "org.orange.audio/audio-format";
/// The record output of Audio format, with U64 fields `rate` and `channels`.
pub const FORMAT_TYPE: &str = "org.orange.audio/format@1";
/// Measures a stream's level in dBFS continuously.
pub const LIVE_LEVEL: &str = "org.orange.audio/live-level";
/// Measures a selected band of a stream in dBFS continuously.
pub const LIVE_BAND_LEVEL: &str = "org.orange.audio/live-band-level";
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
        publisher
            .publish(value)
            .map_err(|_| "the audio stream could not be published".to_owned())?;
        // The connection, not this reference, owns the capture.
        Ok(Box::new(()))
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
impl LiveSource for Meter {
    fn start(
        &self,
        _session: Option<Arc<dyn Session>>,
        settings: &Map<String, Json>,
        arguments: &Arguments,
        publisher: Publisher,
    ) -> Result<Box<dyn Send>, String> {
        let stream = input_stream(arguments)?;
        let hertz = |name| match arguments.get(name) {
            Some(Value::F64(value)) if (0.0..=24_000.0).contains(value) => Ok(*value),
            _ => Err(format!("{name} must be between 0 and 24000 Hz")),
        };
        let (low, high) = if self.band {
            (hertz("low_hz")?, hertz("high_hz")?)
        } else {
            (0.0, 0.0)
        };
        let mut listening = Listening::of(settings, (low, high, LEVEL_WINDOW_MS))?;
        // These borders are live arguments. Only the analyzer window is a setting.
        if high > 0.0 && low >= high {
            return Err("the band needs low_hz < high_hz, or 0 to leave a side open".into());
        }
        listening.low_hz = low;
        listening.high_hz = high;
        publisher
            .publish(Value::F64(SILENCE_DB))
            .map_err(|_| "the audio level could not be published".to_owned())?;
        watch_windows(stream, listening, move |level| {
            publisher.publish(Value::F64(level.unwrap_or(SILENCE_DB))) != Err(EmitError::Closed)
        })
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
        .keywords("microphone, application, system audio, virtual cable, file")
        .uses(INPUT_ADAPTER)
        .live("stream", STREAM_TYPE);
    let audio_format = stream_input(live(
        "audio-format",
        "Audio format",
        "Reports the rate and channel count of a stream.",
    ))
    .live("format", FORMAT_TYPE);
    let level = meter(
        "live-level",
        "Level",
        "Measures a stream's level in dBFS as it changes.",
    )
    .keywords("volume, loudness, meter");
    let band = meter(
        "live-band-level",
        "Band level",
        "Measures a band of a stream in dBFS as it changes. Leave a side at 0 to keep it open.",
    )
    .keywords("bass, frequency, loudness, meter")
    .input_default("low_hz", F64, zero())
    .input_default("high_hz", F64, zero());
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
        .live(&audio_format.build(), AudioFormat)?
        .live(&level.build(), Meter { band: false })?
        .live(&band.build(), Meter { band: true })?
        .source(&transient.build(), Transient)?
        .source(&finished.build(), SourceFinished)?
        .node(&transport.build(), SourceTransport)?
        .node(&sound.build(), SoundControl)
}
