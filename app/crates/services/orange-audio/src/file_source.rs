//! A sound file as an audio source: an input connection whose settings name a `file`
//! instead of a device.
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
use std::path::PathBuf;
use std::sync::{Arc, PoisonError};
use std::time::{Duration, Instant};

/// Rate and channels a file source plays at; the decoder converts every file to them.
pub(crate) const RATE: u32 = 48_000;
pub(crate) const CHANNELS: u16 = 2;
/// The step at which a playing file is handed to its ring.
const BLOCK: Duration = Duration::from_millis(10);

/// What an input connection's settings say about a file source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileSettings {
    pub path: PathBuf,
    /// Starts again at the end until stopped.
    pub looped: bool,
    /// Starts playing as soon as a flow uses the source.
    pub autoplay: bool,
    /// Also heard on an output: `Some(None)` the default one, `Some(Some(name))` a named one.
    pub monitor: Option<Option<String>>,
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
            .ok_or("`file` must be the path of a sound file")?;
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
            path: PathBuf::from(path),
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
    pub(crate) fn seek(&mut self, seconds: f64) {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "callers check the range; the product saturates"
        )]
        let frames = (seconds.max(0.0) * f64::from(RATE)) as usize;
        self.at = frames.saturating_mul(usize::from(CHANNELS));
    }
}

/// Plays `file` into `shared`'s ring until the connection closes. Called from the
/// source's own thread once a flow has started it.
pub(crate) fn run(shared: &InputShared, file: &FileSettings) {
    let sound = match wav::load(&file.path, RATE, CHANNELS) {
        Ok(sound) => sound,
        Err(_) => {
            shared.set_failure(Some("the sound file cannot be read".to_owned()));
            while shared.rest_ended(Duration::from_secs(1)) {}
            return;
        }
    };
    let ring = Ring::new(RATE, CHANNELS);
    *shared.ring.lock().unwrap_or_else(PoisonError::into_inner) = Some(Arc::clone(&ring));
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
    let block = usize::try_from(u128::from(RATE) * BLOCK.as_millis() / 1_000).unwrap_or(480)
        * usize::from(CHANNELS);
    let window = usize::try_from(u128::from(RATE) * shared.window.as_millis() / 1_000)
        .unwrap_or(2_400)
        * usize::from(CHANNELS);
    let mut meter = Meter::new(window);
    let started = Instant::now();
    let mut blocks = 0u32;
    while !shared.stopped() {
        let mut finished = false;
        {
            let mut transport = shared.transport();
            if transport.state == State::Playing {
                let end = transport.at.saturating_add(block).min(sound.len());
                let part = sound.get(transport.at..end).unwrap_or_default();
                ring.push(part.iter().copied());
                meter.push(part.iter().copied(), |level| shared.levels.record(level));
                let rest = std::iter::repeat_n(0.0, block.saturating_sub(part.len()));
                ring.push(rest.clone());
                meter.push(rest, |level| shared.levels.record(level));
                transport.at = end;
                if transport.at >= sound.len() {
                    if file.looped {
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
        transport.seek(0.5);
        assert_eq!(transport.at, 48_000);
        transport.resume();
        assert_eq!(transport.state, State::Playing);
        transport.stop();
        assert_eq!((transport.state, transport.at), (State::Stopped, 0));
    }
}
