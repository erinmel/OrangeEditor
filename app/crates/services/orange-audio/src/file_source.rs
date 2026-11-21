//! A sound file as an audio source: an input connection whose settings name a `file`
//! or HTTP(S) URL instead of a device.
//!
//! The source owns its transport. Play, Pause, Resume, Stop and Jump to time act on it
//! alone, and On finished fires when it reaches its end. While it plays, its samples go
//! into the source's ring at the pace of real time, exactly as a microphone's capture
//! would, so Band level, On band level and On transient listen to the file being
//! played. With `monitor` the source is also heard on an output; without it, the file
//! is analysed silently. Nothing plays until a Play node, or `autoplay`, starts it.

use crate::device::Output;
use crate::level::Meter;
use crate::stream::Ring;
use crate::{InputShared, wav};
use serde_json::{Map, Value as Json};
use std::io::{self, Read};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

/// Rate and channels a file source plays at; the decoder converts every file to them.
pub(crate) const RATE: u32 = 48_000;
pub(crate) const CHANNELS: u16 = 2;
/// The step at which a playing file is handed to its ring.
const BLOCK: Duration = Duration::from_millis(10);
/// Ten minutes of stereo PCM at 48 kHz, shared by playback, looping and seeking.
const MAX_REMOTE_SAMPLES: usize = 57_600_000;

/// What an input connection's settings say about a file source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileSettings {
    pub location: Location,
    /// Starts again at the end until stopped.
    pub looped: bool,
    /// Starts playing as soon as a flow uses the source.
    pub autoplay: bool,
    /// Also heard on an output: `Some(None)` the default one, `Some(Some(name))` a named one.
    pub monitor: Option<Option<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Location {
    Path(PathBuf),
    Url(String),
}

impl FileSettings {
    /// The file settings of a connection, or `None` when it names a device.
    pub(crate) fn of(settings: &Map<String, Json>) -> Result<Option<Self>, String> {
        let Some(file) = settings.get("file") else {
            return Ok(None);
        };
        let path = file
            .as_str()
            .filter(|path| !path.trim().is_empty())
            .ok_or("`file` must be a sound file path or HTTP(S) URL")?;
        if settings
            .get("device")
            .is_some_and(|device| !device.is_null())
        {
            return Err("a source names a `device` or a `file`, not both".into());
        }
        let flag = |name: &str| match settings.get(name) {
            None | Some(Json::Null) => Ok(false),
            Some(Json::Bool(on)) => Ok(*on),
            Some(_) => Err(format!("`{name}` must be true or false")),
        };
        let monitor = if flag("monitor")? {
            Some(
                settings
                    .get("monitor_device")
                    .and_then(Json::as_str)
                    .filter(|name| !name.is_empty())
                    .map(str::to_owned),
            )
        } else {
            None
        };
        Ok(Some(Self {
            location: if path.starts_with("http://") || path.starts_with("https://") {
                Location::Url(path.to_owned())
            } else {
                Location::Path(PathBuf::from(path))
            },
            looped: flag("loop")?,
            autoplay: flag("autoplay")?,
            monitor,
        }))
    }
}

/// Where a file source is in its sound.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum State {
    #[default]
    Stopped,
    Playing,
    Paused,
}

/// A file source's transport: its state and position in samples.
#[derive(Debug, Default)]
pub(crate) struct Transport {
    pub state: State,
    pub at: usize,
}

impl Transport {
    pub(crate) fn play(&mut self) {
        self.state = State::Playing;
        self.at = 0;
    }
    pub(crate) fn pause(&mut self) {
        if self.state == State::Playing {
            self.state = State::Paused;
        }
    }
    pub(crate) fn resume(&mut self) {
        if self.state == State::Paused {
            self.state = State::Playing;
        }
    }
    pub(crate) fn stop(&mut self) {
        self.state = State::Stopped;
        self.at = 0;
    }
    /// Moves to `seconds` from the start, keeping the state.
    pub(crate) fn seek(&mut self, seconds: f64, rate: u32, channels: u16) {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "callers check the range; the product saturates"
        )]
        let frames = (seconds.max(0.0) * f64::from(rate)) as usize;
        self.at = frames.saturating_mul(usize::from(channels));
    }
}

/// Plays `file` into `shared`'s ring until the connection closes. Called from the
/// source's own thread once a flow has started it.
pub(crate) fn run(shared: &Arc<InputShared>, file: &FileSettings) {
    let (sound, rate, channels) = match &file.location {
        Location::Path(path) => match wav::load(path, RATE, CHANNELS) {
            Ok(samples) => (Sound::Local(samples), RATE, CHANNELS),
            Err(_) => {
                shared.set_failure(Some("the sound file cannot be read".to_owned()));
                while shared.rest_ended(Duration::from_secs(1)) {}
                return;
            }
        },
        Location::Url(url) => loop {
            match remote_sound(shared, url) {
                Ok((samples, rate, channels)) => break (Sound::Remote(samples), rate, channels),
                Err(reason) => shared.set_failure(Some(reason)),
            }
            if !shared.rest_ended(Duration::from_secs(1)) {
                return;
            }
        },
    };
    let ring = Ring::new(rate, channels);
    shared.replace_ring(Some(Arc::clone(&ring)));
    shared.set_failure(None);
    // Heard on an output through the same ring the analysers read.
    let monitor = file.monitor.as_ref().and_then(|device| {
        let output = Arc::new(Output::open(device.as_deref()).ok()?);
        output.play_live(Arc::clone(&ring), 1.0)?;
        Some(output)
    });
    if file.autoplay {
        shared.transport().play();
    }
    let block = usize::try_from(u128::from(rate) * BLOCK.as_millis() / 1_000).unwrap_or(480)
        * usize::from(channels);
    let window = usize::try_from(u128::from(rate) * shared.window.as_millis() / 1_000)
        .unwrap_or(2_400)
        * usize::from(channels);
    let mut meter = Meter::new(window);
    let started = Instant::now();
    let mut blocks = 0u32;
    while !shared.stopped() {
        let mut finished = false;
        {
            let mut transport = shared.transport();
            if transport.state == State::Playing {
                let mut consumed = 0;
                let complete = sound.read_and_visit(transport.at, block, |part| {
                    consumed = part.len();
                    ring.push(part.iter().copied());
                    meter.push(part.iter().copied(), |level| shared.levels.record(level));
                });
                let rest = std::iter::repeat_n(0.0, block.saturating_sub(consumed));
                ring.push(rest.clone());
                meter.push(rest, |level| shared.levels.record(level));
                transport.at = transport.at.saturating_add(consumed);
                if complete {
                    if sound.failed() {
                        transport.stop();
                        shared.set_failure(Some(
                            "the remote audio file could not finish decoding".into(),
                        ));
                    } else if file.looped {
                        transport.at = 0;
                    } else {
                        transport.stop();
                        finished = true;
                    }
                }
            } else {
                let silence = std::iter::repeat_n(0.0, block);
                ring.push(silence.clone());
                meter.push(silence, |level| shared.levels.record(level));
            }
        }
        if finished {
            // The entry has no payload: every listener fires with none.
            for emitter in shared.finished.emitters() {
                let _ = emitter.emit(None);
            }
        }
        blocks = blocks.wrapping_add(1);
        let due = BLOCK * blocks;
        if let Some(wait) = due.checked_sub(started.elapsed()) {
            let _ = shared.rest_ended(wait);
        }
    }
    ring.close();
    drop(monitor);
}

enum Sound {
    Local(Arc<[f32]>),
    Remote(Arc<Mutex<RemoteSound>>),
}

impl Sound {
    fn read_and_visit(&self, start: usize, count: usize, visit: impl FnOnce(&[f32])) -> bool {
        let read = |samples: &[f32], complete: bool| {
            let end = start.saturating_add(count).min(samples.len());
            visit(samples.get(start..end).unwrap_or_default());
            complete && end == samples.len()
        };
        match self {
            Self::Local(samples) => read(samples, true),
            Self::Remote(sound) => {
                let sound = sound.lock().unwrap_or_else(PoisonError::into_inner);
                read(&sound.samples, sound.done.is_some())
            }
        }
    }

    fn failed(&self) -> bool {
        matches!(self, Self::Remote(sound) if sound.lock().unwrap_or_else(PoisonError::into_inner)
            .done.as_ref().is_some_and(Result::is_err))
    }
}

#[derive(Default)]
struct RemoteSound {
    samples: Vec<f32>,
    format: Option<(u32, u16)>,
    done: Option<Result<(), String>>,
}

/// Demuxers can treat a read error as EOF. Preserve the transport outcome so a
/// disconnected server cannot produce a successful Source Finished event.
struct CheckedRead<'a, R> {
    reader: R,
    source: &'a InputShared,
    failed: &'a mut bool,
}

impl<R: Read> Read for CheckedRead<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let result = if self.source.stopped() {
            Err(io::Error::other("audio source stopped"))
        } else {
            self.reader.read(buffer)
        };
        if result
            .as_ref()
            .is_err_and(|error| error.kind() != io::ErrorKind::Interrupted)
        {
            *self.failed = true;
        }
        result
    }
}

fn remote_sound(
    shared: &Arc<InputShared>,
    url: &str,
) -> Result<(Arc<Mutex<RemoteSound>>, u32, u16), String> {
    use symphonia::core::io::ReadOnlySource;

    let sound = Arc::new(Mutex::new(RemoteSound::default()));
    let output = Arc::clone(&sound);
    let source = Arc::clone(shared);
    let url = url.to_owned();
    thread::Builder::new()
        .name("orange-remote-audio".into())
        .spawn(move || {
            let fetch = || {
                let body = orange_net::media_reader(&url, wav::MAX_FILE_BYTES)?;
                let mut read_failed = false;
                let reader = CheckedRead {
                    reader: body,
                    source: &source,
                    failed: &mut read_failed,
                };
                let decoded = wav::decode_packets(
                    Box::new(ReadOnlySource::new(reader)),
                    |rate, channels, packet| {
                        if source.stopped()
                            || !(8_000..=192_000).contains(&rate)
                            || !(1..=8).contains(&channels)
                        {
                            return Err(wav::WavError::Unreadable);
                        }
                        let mut output = output.lock().unwrap_or_else(PoisonError::into_inner);
                        if output
                            .samples
                            .len()
                            .saturating_add(packet.samples_interleaved())
                            > MAX_REMOTE_SAMPLES
                        {
                            return Err(wav::WavError::TooLarge);
                        }
                        wav::append_packet(&mut output.samples, rate, channels, packet)?;
                        if !output.samples.is_empty() {
                            output.format = Some((rate, channels));
                        }
                        Ok(())
                    },
                );
                if read_failed {
                    return Err("the remote audio transfer was interrupted".into());
                }
                decoded
                    .map(|_| ())
                    .map_err(|_| "the remote audio file cannot be decoded".into())
            };
            let result = fetch();
            output.lock().unwrap_or_else(PoisonError::into_inner).done = Some(result);
        })
        .map_err(|_| "the remote audio reader could not be started".to_owned())?;
    while !shared.stopped() {
        {
            let buffer = sound.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some((rate, channels)) = buffer.format {
                return Ok((Arc::clone(&sound), rate, channels));
            }
            if let Some(result) = &buffer.done {
                return Err(result
                    .as_ref()
                    .err()
                    .cloned()
                    .unwrap_or_else(|| "the audio response is empty".into()));
            }
        }
        shared.rest(BLOCK);
    }
    Err("the audio source stopped".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(json: Json) -> Map<String, Json> {
        match json {
            Json::Object(map) => map,
            _ => unreachable!("tests pass objects"),
        }
    }

    #[test]
    fn a_source_names_a_file_or_a_device() {
        assert_eq!(FileSettings::of(&settings(serde_json::json!({}))), Ok(None));
        let file = FileSettings::of(&settings(serde_json::json!({
            "file": "song.ogg", "loop": true, "monitor": true})))
        .unwrap()
        .unwrap();
        assert!(file.looped && !file.autoplay);
        assert_eq!(file.monitor, Some(None));
        for bad in [
            serde_json::json!({"file": ""}),
            serde_json::json!({"file": 3}),
            serde_json::json!({"file": "a.wav", "device": "mic"}),
            serde_json::json!({"file": "a.wav", "loop": "yes"}),
        ] {
            assert!(FileSettings::of(&settings(bad)).is_err());
        }
    }

    #[test]
    fn the_transport_plays_pauses_resumes_seeks_and_stops() {
        let mut transport = Transport::default();
        transport.pause();
        assert_eq!(transport.state, State::Stopped, "nothing to pause");
        transport.play();
        transport.at = 100;
        transport.pause();
        assert_eq!((transport.state, transport.at), (State::Paused, 100));
        transport.seek(0.5, RATE, CHANNELS);
        assert_eq!(transport.at, 48_000);
        transport.resume();
        assert_eq!(transport.state, State::Playing);
        transport.stop();
        assert_eq!((transport.state, transport.at), (State::Stopped, 0));
    }
}
