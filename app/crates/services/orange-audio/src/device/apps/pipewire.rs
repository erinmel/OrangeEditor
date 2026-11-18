//! A PipeWire playback-stream capture using the system's pw-dump and pw-record.
use super::{Capturing, Levels, Meter, PREFIX};
use crate::device::InputDriver;
use crate::stream::Ring;
use serde_json::Value as Json;
use std::io::{ErrorKind, Read};
use std::num::NonZeroU64;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const RATE: u32 = 48_000;
const CHANNELS: u16 = 2;
const QUERY_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_DUMP: u64 = 8 * 1024 * 1024;
const MAX_HELP: u64 = 16 * 1024;
const PIPEWIRE: &str = "Needs PipeWire with pw-dump and pw-record utilities.";
const NO_SERVER: &str = "No PipeWire session is available for application capture.";
const OUTDATED: &str = "The installed pw-record does not support raw audio capture.";
const STALE: &str = "That application stream has closed. Refresh the list and choose it again.";
const RECORDING: &str = "The PipeWire recording process could not start.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Route {
    cookie: i64,
    serial: NonZeroU64,
}

impl Route {
    fn parse(setting: &str) -> Option<Self> {
        let (cookie, serial) = setting.strip_prefix("pipewire:")?.split_once(':')?;
        Some(Self {
            cookie: cookie.parse().ok()?,
            serial: serial.parse().ok()?,
        })
    }

    fn setting(self) -> String {
        format!("{PREFIX}pipewire:{}:{}", self.cookie, self.serial)
    }
}

/// Reads one finite tool response without letting a stalled PipeWire server or a
/// large session block the picker indefinitely. A pipe reader drains while the
/// caller waits on the child so neither waits for the other's full pipe buffer.
fn query(program: &str, args: &[&str], limit: u64) -> Result<Vec<u8>, &'static str> {
    let mut child = Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| PIPEWIRE)?;
    let Some(stdout) = child.stdout.take() else {
        terminate(&mut child);
        return Err(NO_SERVER);
    };
    let reader = thread::Builder::new()
        .name("orange-audio-pipewire-query".into())
        .spawn(move || {
            let mut bytes = Vec::new();
            stdout.take(limit + 1).read_to_end(&mut bytes)?;
            Ok::<_, std::io::Error>(bytes)
        });
    let Ok(reader) = reader else {
        terminate(&mut child);
        return Err(NO_SERVER);
    };
    let started = Instant::now();
    let success = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if started.elapsed() < QUERY_TIMEOUT => {
                thread::sleep(Duration::from_millis(5))
            }
            _ => {
                terminate(&mut child);
                break false;
            }
        }
    };
    let bytes = reader
        .join()
        .map_err(|_| NO_SERVER)?
        .map_err(|_| NO_SERVER)?;
    if !success || !u64::try_from(bytes.len()).is_ok_and(|size| size <= limit) {
        return Err(NO_SERVER);
    }
    Ok(bytes)
}

fn terminate(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn apps(bytes: &[u8]) -> Result<Vec<(Route, String)>, &'static str> {
    let graph: Vec<Json> = serde_json::from_slice(bytes).map_err(|_| NO_SERVER)?;
    let cookie = graph
        .iter()
        .find(|object| object["type"] == "PipeWire:Interface:Core")
        .and_then(|core| core["info"]["cookie"].as_i64())
        .ok_or(NO_SERVER)?;
    let mut applications: Vec<_> = graph
        .iter()
        .filter_map(|object| {
            let properties = &object["info"]["props"];
            if object["type"] != "PipeWire:Interface:Node"
                || properties["media.class"] != "Stream/Output/Audio"
            {
                return None;
            }
            let serial = properties["object.serial"]
                .as_u64()
                .or_else(|| properties["object.serial"].as_str()?.parse().ok())
                .and_then(NonZeroU64::new)?;
            let name = properties["application.name"]
                .as_str()
                .or_else(|| properties["node.description"].as_str())
                .or_else(|| properties["node.name"].as_str())?;
            let route = Route { cookie, serial };
            let title = properties["media.name"]
                .as_str()
                .filter(|title| !title.is_empty() && *title != name);
            let label = match title {
                Some(title) => format!("{name} · {title} (app, {serial})"),
                None => format!("{name} (app, {serial})"),
            };
            Some((route, label))
        })
        .collect();
    applications
        .sort_unstable_by(|(a, name), (b, other)| name.cmp(other).then(a.serial.cmp(&b.serial)));
    applications.dedup_by_key(|(route, _)| route.serial);
    Ok(applications)
}

fn current_apps() -> Result<Vec<(Route, String)>, &'static str> {
    apps(&query("pw-dump", &[], MAX_DUMP)?)
}

pub(super) fn choices() -> Result<Vec<(String, String)>, &'static str> {
    let help = query("pw-record", &["--help"], MAX_HELP)?;
    if !String::from_utf8_lossy(&help).contains("--raw") {
        return Err(OUTDATED);
    }
    let apps = current_apps()?;
    if apps.is_empty() {
        return Err("PipeWire is available. Start playing audio in an app and refresh this list.");
    }
    Ok(apps
        .into_iter()
        .map(|(route, label)| (route.setting(), label))
        .collect())
}

pub(super) fn open(
    setting: &str,
    meter: impl Fn(u32, u16) -> Meter,
    levels: Arc<Levels>,
) -> Result<Capturing, &'static str> {
    let route = Route::parse(setting).ok_or(STALE)?;
    if !current_apps()?
        .into_iter()
        .any(|(current, _)| current == route)
    {
        return Err(STALE);
    }
    let serial = route.serial.to_string();
    let mut child = Command::new("pw-record")
        .args([
            "--raw",
            "--format=f32",
            "--rate=48000",
            "--channels=2",
            "--latency=20ms",
            "--target",
            &serial,
            "--properties",
            r#"{"node.dont-fallback":true,"node.dont-reconnect":true,"node.dont-move":true}"#,
            "-",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| {
            if error.kind() == ErrorKind::NotFound {
                PIPEWIRE
            } else {
                RECORDING
            }
        })?;
    let Some(stdout) = child.stdout.take() else {
        terminate(&mut child);
        return Err(RECORDING);
    };
    let ring = Ring::new(RATE, CHANNELS);
    let writer = Arc::clone(&ring);
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = Arc::clone(&stop);
    let meter = meter(RATE, CHANNELS);
    let worker = thread::Builder::new()
        .name("orange-audio-pipewire-capture".into())
        .spawn(move || {
            read_samples(stdout, meter, &levels, &writer);
            writer.close();
            if !stopping.load(Ordering::Acquire) {
                levels.mark_broken();
            }
        });
    let Ok(worker) = worker else {
        terminate(&mut child);
        return Err(RECORDING);
    };
    Ok(Capturing {
        ring,
        driver: InputDriver::App(Recording {
            child,
            stop,
            worker: Some(worker),
        }),
    })
}

pub(in crate::device) struct Recording {
    child: Child,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Recording {
    pub(in crate::device) fn stop(&mut self) {
        if let Some(worker) = self.worker.take() {
            self.stop.store(true, Ordering::Release);
            terminate(&mut self.child);
            let _ = worker.join();
        }
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        self.stop();
    }
}

// A pipe can yield partial samples or a partial stereo frame. Both the level meter
// and the shared ring consume exactly the same complete frames, in order.
fn read_samples(mut input: impl Read, mut meter: Meter, levels: &Levels, ring: &Ring) {
    const SAMPLE: usize = std::mem::size_of::<f32>();
    const FRAME: usize = SAMPLE * CHANNELS as usize;
    let mut buffer = [0u8; 8192];
    let mut pending = 0;
    loop {
        let got = match input.read(&mut buffer[pending..]) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        let total = pending + got;
        let complete = total - total % FRAME;
        if complete > 0 {
            let samples = || {
                buffer[..complete]
                    .as_chunks::<SAMPLE>()
                    .0
                    .iter()
                    .map(|bytes| f32::from_ne_bytes(*bytes))
            };
            meter.push(samples(), |db| levels.record(db));
            ring.push(samples());
        }
        buffer.copy_within(complete..total, 0);
        pending = total - complete;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_choice_identifies_one_app_stream_in_one_pipewire_session() {
        let dump = br#"
        [
            {"type":"PipeWire:Interface:Core","info":{"cookie":108}},
            {"type":"PipeWire:Interface:Node","info":{"props":{"media.class":"Audio/Source","object.serial":20,"node.name":"mic"}}},
            {"type":"PipeWire:Interface:Node","info":{"props":{"media.class":"Stream/Output/Audio","object.serial":21,"application.name":"Browser","media.name":"Music"}}},
            {"type":"PipeWire:Interface:Node","info":{"props":{"media.class":"Stream/Output/Audio","object.serial":22,"application.name":"Browser","media.name":"Voice"}}},
            {"type":"PipeWire:Interface:Node","info":{"props":{"media.class":"Stream/Input/Audio","object.serial":23,"application.name":"Recorder"}}}
        ]"#;
        let found = apps(dump).unwrap();
        assert_eq!(found.len(), 2);
        let (route, label) = &found[0];
        let setting = route.setting();
        assert_eq!(setting, "app:pipewire:108:21");
        assert_eq!(label, "Browser · Music (app, 21)");
        assert_eq!(
            Route::parse(setting.strip_prefix(PREFIX).unwrap()),
            Some(*route)
        );
        assert_ne!(
            Route::parse("pipewire:109:21"),
            Some(*route),
            "a restarted server can reuse a stream serial",
        );
        assert_eq!(Route::parse("pipewire:108:0"), None);
        assert_eq!(Route::parse("pipewire:108:--target"), None);
        assert!(apps(b"[]").is_err());
    }

    struct Fragmented<'a>(&'a [u8]);

    impl Read for Fragmented<'_> {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            (&mut self.0).read(&mut out[..3])
        }
    }

    #[test]
    fn fragmented_pcm_preserves_complete_stereo_frames_only() {
        let ring = Ring::new(RATE, CHANNELS);
        let levels = Levels::new();
        let samples = [0.5f32, -0.25, 0.25, -0.5, 0.75];
        let bytes: Vec<_> = samples.into_iter().flat_map(f32::to_ne_bytes).collect();
        read_samples(Fragmented(&bytes), Meter::new(4), &levels, &ring);
        let mut seen = [0.0; 6];
        assert_eq!(ring.recent(&mut seen), 4);
        assert_eq!(&seen[..4], &samples[..4]);
        assert!(levels.latest() > -12.0);
    }

    #[test]
    #[ignore = "needs Linux, a PipeWire/WirePlumber session and a selected app playing audio"]
    fn the_selected_application_feeds_the_capture_ring() {
        let selected = std::env::var("ORANGE_AUDIO_TEST_APP")
            .expect("set ORANGE_AUDIO_TEST_APP to a unique substring of the playing app's name");
        let choices = choices().expect("the PipeWire playback streams should be listed");
        let mut matching = choices.iter().filter(|(_, name)| name.contains(&selected));
        let (setting, _) = matching
            .next()
            .expect("the selected playback stream must exist");
        assert!(
            matching.next().is_none(),
            "use a substring unique to one offered stream"
        );

        let levels = Arc::new(Levels::new());
        let stream = open(
            setting.strip_prefix(PREFIX).unwrap(),
            |rate, channels| {
                Meter::new(usize::try_from(rate / 20).unwrap() * usize::from(channels))
            },
            Arc::clone(&levels),
        )
        .expect("open the requested playback stream");
        let ring = Arc::clone(stream.ring());
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(5) && levels.latest() <= -65.0 {
            assert!(!ring.closed(), "PipeWire ended the selected capture");
            thread::sleep(Duration::from_millis(20));
        }
        assert!(
            levels.latest() > -65.0,
            "the selected application did not produce audible samples"
        );
        assert!(ring.written() >= 2);
        drop(stream);
        assert!(ring.closed());
    }
}
