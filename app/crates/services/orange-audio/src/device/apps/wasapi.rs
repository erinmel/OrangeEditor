//! One application's sound on Windows: WASAPI process loopback (Windows 10 version
//! 2004 and later) captures what one process and its children play, before Windows
//! mixes it with other applications. The choices are the processes that have an audio
//! session now.
//!
//! The only unsafe code of this crate: COM and Win32 calls, each block saying why it
//! holds.
#![allow(unsafe_code)]
use super::{Capturing, Levels, Meter, PREFIX};
use crate::device::InputDriver;
use crate::stream::Ring;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;
use windows::Win32::Foundation::{CloseHandle, E_FAIL, HANDLE, S_OK, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
    AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_LOOPBACK, AUDIOCLIENT_ACTIVATION_PARAMS,
    AUDIOCLIENT_ACTIVATION_PARAMS_0, AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
    AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS, ActivateAudioInterfaceAsync, AudioSessionStateExpired,
    DEVICE_STATE_ACTIVE, IActivateAudioInterfaceAsyncOperation,
    IActivateAudioInterfaceCompletionHandler, IActivateAudioInterfaceCompletionHandler_Impl,
    IAudioCaptureClient, IAudioClient, IAudioSessionControl2, IAudioSessionManager2,
    IMMDeviceEnumerator, MMDeviceEnumerator, PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
    VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, WAVEFORMATEX, eRender,
};
use windows::Win32::Media::Multimedia::WAVE_FORMAT_IEEE_FLOAT;
use windows::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
};
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::{
    BLOB, CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemAlloc,
    CoUninitialize,
};
use windows::Win32::System::Threading::{
    CreateEventW, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    QueryFullProcessImageNameW, WaitForSingleObject,
};
use windows::Win32::System::Variant::VT_BLOB;
use windows::core::{HSTRING, Interface, PWSTR, Ref, implement};

const RATE: u32 = 48_000;
const CHANNELS: u16 = 2;
/// How long Windows may take to grant a capture.
const ACTIVATION: Duration = Duration::from_secs(5);
/// Shared-mode buffer, in 100 ns units: 200 ms.
const BUFFER: i64 = 2_000_000;
/// How often the capture thread looks for a stop when no audio arrives.
const IDLE_MS: u32 = 100;

const STALE: &str = "That application is no longer running. Refresh the list and choose it again.";
const CAPTURE: &str =
    "Windows did not start capturing that application. It needs Windows 10 version 2004 or later.";
const LIST: &str = "Windows did not list the applications playing sound.";

/// One process with an audio session: its ID and executable name. The name guards
/// against a process ID that Windows has given to another program since.
#[derive(Debug, Clone, PartialEq, Eq)]
struct App {
    pid: u32,
    name: String,
}
impl App {
    /// What follows `app:` in a saved device setting.
    fn setting(&self) -> String {
        format!("{PREFIX}{}:{}", self.pid, self.name)
    }
    fn parse(text: &str) -> Option<Self> {
        let (pid, name) = text.split_once(':')?;
        Some(Self {
            pid: pid.parse().ok().filter(|pid| *pid != 0)?,
            name: name.to_owned(),
        })
    }
}

/// Initializes COM for the calling thread in the multithreaded apartment, and
/// uninitializes it when dropped if this call initialized it.
struct Com(bool);
impl Com {
    fn new() -> Self {
        // SAFETY: called once per thread before any COM call on it; a thread already in
        // another apartment keeps it (RPC_E_CHANGED_MODE) and is not uninitialized here.
        Self(unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok())
    }
}
impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: balances the successful CoInitializeEx of this thread.
            unsafe { CoUninitialize() };
        }
    }
}

/// The executable name of process `pid`, without its folder or extension.
fn process_name(pid: u32) -> Option<String> {
    let path = process_path(pid)?;
    std::path::Path::new(&path)
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .filter(|stem| !stem.is_empty())
}

/// What a program calls itself: the file description of its version information,
/// such as "Microsoft Edge" for msedge.exe, when it has one.
fn description(path: &str) -> Option<String> {
    let file = HSTRING::from(path);
    // SAFETY: the buffer is as large as Windows says the version information is, and
    // the strings read from it are within the length VerQueryValueW returns.
    unsafe {
        let size = GetFileVersionInfoSizeW(&file, None);
        if size == 0 {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        GetFileVersionInfoW(&file, None, size, data.as_mut_ptr().cast()).ok()?;
        let block = data.as_ptr().cast();
        let query = |key: &str| -> Option<(*mut std::ffi::c_void, u32)> {
            let mut found = std::ptr::null_mut();
            let mut length = 0u32;
            VerQueryValueW(block, &HSTRING::from(key), &raw mut found, &raw mut length)
                .as_bool()
                .then_some((found, length))
                .filter(|(found, length)| !found.is_null() && *length > 0)
        };
        // The first language the file lists, else US English in Unicode.
        let language = query("\\VarFileInfo\\Translation")
            .filter(|(_, length)| *length >= 4)
            .map_or_else(
                || "040904b0".to_owned(),
                |(found, _)| {
                    let pair = found.cast::<u16>();
                    format!("{:04x}{:04x}", *pair, *pair.add(1))
                },
            );
        let (text, length) = query(&format!("\\StringFileInfo\\{language}\\FileDescription"))?;
        let chars = std::slice::from_raw_parts(text.cast::<u16>(), length as usize);
        let end = chars.iter().position(|c| *c == 0).unwrap_or(chars.len());
        let name = String::from_utf16_lossy(&chars[..end]).trim().to_owned();
        (!name.is_empty()).then_some(name)
    }
}

/// The full path of process `pid`'s executable.
fn process_path(pid: u32) -> Option<String> {
    // SAFETY: the handle is checked and closed; the buffer outlives the call and its
    // length is passed in and updated by Windows.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buffer = [0u16; 1024];
        let mut size = buffer.len() as u32;
        let named = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &raw mut size,
        );
        let _ = CloseHandle(process);
        named.ok()?;
        Some(String::from_utf16_lossy(&buffer[..size as usize]))
    }
}

/// The processes with a live audio session on any active output, each once.
fn sessions() -> windows::core::Result<Vec<App>> {
    let _com = Com::new();
    let mut apps: Vec<App> = Vec::new();
    // SAFETY: plain COM calls on interfaces this function owns; each result is checked.
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let devices = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
        for d in 0..devices.GetCount()? {
            let Ok(manager) = devices
                .Item(d)
                .and_then(|device| device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None))
            else {
                continue;
            };
            let sessions = manager.GetSessionEnumerator()?;
            for s in 0..sessions.GetCount()? {
                let Ok(control) = sessions
                    .GetSession(s)
                    .and_then(|session| session.cast::<IAudioSessionControl2>())
                else {
                    continue;
                };
                if control.IsSystemSoundsSession() == S_OK
                    || control
                        .GetState()
                        .is_ok_and(|state| state == AudioSessionStateExpired)
                {
                    continue;
                }
                let Some(pid) = control.GetProcessId().ok().filter(|pid| *pid != 0) else {
                    continue;
                };
                if let Some(name) = process_name(pid) {
                    let app = App { pid, name };
                    if !apps.contains(&app) {
                        apps.push(app);
                    }
                }
            }
        }
    }
    apps.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.pid.cmp(&b.pid))
    });
    Ok(apps)
}

/// Each application playing sound now, as (setting, label): the name the program
/// gives itself, such as "Microsoft Edge", or its executable name. Two processes of
/// one program are told apart by their process ID.
pub(super) fn choices() -> Result<Vec<(String, String)>, &'static str> {
    // On its own thread, so the caller's COM apartment never matters.
    let apps = thread::Builder::new()
        .name("orange-audio-apps".into())
        .spawn(sessions)
        .map_err(|_| LIST)?
        .join()
        .map_err(|_| LIST)?
        .map_err(|_| LIST)?;
    Ok(apps
        .iter()
        .map(|app| {
            let twin = apps.iter().filter(|other| other.name == app.name).count() > 1;
            let name = process_path(app.pid)
                .and_then(|path| description(&path))
                .unwrap_or_else(|| app.name.clone());
            let label = if twin {
                format!("{name} ({})", app.pid)
            } else {
                name
            };
            (app.setting(), label)
        })
        .collect())
}

/// Tells the capture thread that Windows finished activating the loopback client.
/// Objects made with `implement` are agile already, as Windows requires of this
/// handler, which it calls from any thread.
#[implement(IActivateAudioInterfaceCompletionHandler)]
struct Activated(mpsc::SyncSender<()>);
impl IActivateAudioInterfaceCompletionHandler_Impl for Activated_Impl {
    // The Windows COM trait requires this exact spelling.
    //noinspection RsMethodNaming
    fn ActivateCompleted(
        &self,
        _operation: Ref<IActivateAudioInterfaceAsyncOperation>,
    ) -> windows::core::Result<()> {
        let _ = self.0.try_send(());
        Ok(())
    }
}

/// An audio client capturing what process `pid` and its children play.
fn activate(pid: u32) -> windows::core::Result<IAudioClient> {
    let params = AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: pid,
                ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
            },
        },
    };
    let mut variant = PROPVARIANT::default();
    // SAFETY: the variant is a VT_BLOB. Windows takes the blob and frees it with
    // CoTaskMemFree once activation ends (a blob on the stack corrupts the heap), so it
    // is allocated with CoTaskMemAlloc and not freed here.
    unsafe {
        let value = &mut *variant.Anonymous.Anonymous;
        value.vt = VT_BLOB;
        value.Anonymous.blob = BLOB {
            cbSize: size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
            pBlobData: {
                let block = CoTaskMemAlloc(size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>())
                    .cast::<AUDIOCLIENT_ACTIVATION_PARAMS>();
                if block.is_null() {
                    return Err(windows::core::Error::from(E_FAIL));
                }
                block.write(params);
                block.cast()
            },
        };
    }
    let (done, finished) = mpsc::sync_channel(1);
    let handler: IActivateAudioInterfaceCompletionHandler = Activated(done).into();
    // SAFETY: every pointer passed lives until the completion is received below.
    unsafe {
        let operation = ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some(&raw const variant),
            &handler,
        )?;
        finished
            .recv_timeout(ACTIVATION)
            .map_err(|_| windows::core::Error::from(E_FAIL))?;
        let mut result = S_OK;
        let mut activated = None;
        operation.GetActivateResult(&raw mut result, &raw mut activated)?;
        result.ok()?;
        activated
            .ok_or_else(|| windows::core::Error::from(E_FAIL))?
            .cast()
    }
}

/// The open stream of one capture: its client, capture service and wake-up event.
struct Stream {
    client: IAudioClient,
    capture: IAudioCaptureClient,
    event: HANDLE,
}
impl Drop for Stream {
    fn drop(&mut self) {
        // SAFETY: stops the client this stream started and closes its own event.
        unsafe {
            let _ = self.client.Stop();
            let _ = CloseHandle(self.event);
        }
    }
}

/// Starts `client` capturing 48 kHz stereo floats, which Windows converts to.
fn start(client: IAudioClient) -> windows::core::Result<Stream> {
    let format = WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_IEEE_FLOAT as u16,
        nChannels: CHANNELS,
        nSamplesPerSec: RATE,
        nAvgBytesPerSec: RATE * 8,
        nBlockAlign: 8,
        wBitsPerSample: 32,
        cbSize: 0,
    };
    // SAFETY: the format lives through Initialize; the event is owned by the stream.
    unsafe {
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_LOOPBACK
                | AUDCLNT_STREAMFLAGS_EVENTCALLBACK
                | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
            BUFFER,
            0,
            &raw const format,
            None,
        )?;
        let event = CreateEventW(None, false, false, None)?;
        let stream = Stream {
            capture: client.GetService()?,
            client,
            event,
        };
        stream.client.SetEventHandle(stream.event)?;
        stream.client.Start()?;
        Ok(stream)
    }
}

/// Moves every packet Windows has ready into the meter and the ring, until `stop`.
fn pump(
    stream: &Stream,
    stop: &AtomicBool,
    meter: &mut Meter,
    levels: &Levels,
    ring: &Ring,
) -> windows::core::Result<()> {
    let channels = usize::from(CHANNELS);
    while !stop.load(Ordering::Acquire) {
        // SAFETY: the event belongs to the open stream.
        if unsafe { WaitForSingleObject(stream.event, IDLE_MS) } != WAIT_OBJECT_0 {
            continue;
        }
        loop {
            // SAFETY: the buffer Windows lends is read only between GetBuffer and
            // ReleaseBuffer, and only as many samples as it says it holds.
            unsafe {
                if stream.capture.GetNextPacketSize()? == 0 {
                    break;
                }
                let mut data = std::ptr::null_mut();
                let mut frames = 0u32;
                let mut flags = 0u32;
                stream.capture.GetBuffer(
                    &raw mut data,
                    &raw mut frames,
                    &raw mut flags,
                    None,
                    None,
                )?;
                let count = frames as usize * channels;
                let silent = flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0;
                if silent || data.is_null() {
                    let quiet = || std::iter::repeat_n(0.0f32, count);
                    meter.push(quiet(), |db| levels.record(db));
                    ring.push(quiet());
                } else {
                    let samples = std::slice::from_raw_parts(data.cast::<f32>(), count);
                    meter.push(samples.iter().copied(), |db| levels.record(db));
                    ring.push(samples.iter().copied());
                }
                stream.capture.ReleaseBuffer(frames)?;
            }
        }
    }
    Ok(())
}

/// A running capture of one application.
pub(in crate::device) struct Capture {
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Capture {
    pub(in crate::device) fn stop(&mut self) {
        if let Some(worker) = self.worker.take() {
            self.stop.store(true, Ordering::Release);
            let _ = worker.join();
        }
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Starts capturing the application `app` names (what follows `app:`).
pub(super) fn open(
    app: &str,
    meter: impl Fn(u32, u16) -> Meter,
    levels: Arc<Levels>,
) -> Result<Capturing, String> {
    let app = App::parse(app).ok_or(STALE)?;
    if process_name(app.pid).as_deref() != Some(app.name.as_str()) {
        return Err(STALE.to_owned());
    }
    let ring = Ring::new(RATE, CHANNELS);
    let writer = Arc::clone(&ring);
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = Arc::clone(&stop);
    let mut meter = meter(RATE, CHANNELS);
    let (started, start_result) = mpsc::sync_channel(1);
    let worker = thread::Builder::new()
        .name("orange-audio-app-capture".into())
        .spawn(move || {
            let _com = Com::new();
            let stream = match activate(app.pid).and_then(start) {
                Ok(stream) => stream,
                Err(_) => {
                    let _ = started.send(false);
                    return;
                }
            };
            let _ = started.send(true);
            let ended = pump(&stream, &stopping, &mut meter, &levels, &writer);
            drop(stream);
            writer.close();
            // A capture that ended without being asked to, such as when the
            // application closed, is broken and is opened again later.
            if ended.is_err() || !stopping.load(Ordering::Acquire) {
                levels.mark_broken();
            }
        })
        .map_err(|_| CAPTURE.to_owned())?;
    let mut capture = Capture {
        stop,
        worker: Some(worker),
    };
    if start_result.recv_timeout(ACTIVATION * 2) != Ok(true) {
        capture.stop();
        return Err(CAPTURE.to_owned());
    }
    Ok(Capturing {
        ring,
        driver: InputDriver::WindowsApp(capture),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_choice_names_the_process_and_refuses_a_malformed_one() {
        let app = App {
            pid: 4321,
            name: "Spotify".into(),
        };
        let setting = app.setting();
        assert_eq!(setting, "app:4321:Spotify");
        let parsed = App::parse(setting.strip_prefix(PREFIX).unwrap()).unwrap();
        assert_eq!(parsed, app);
        for bad in ["", "4321", "x:Spotify", "0:System"] {
            assert!(App::parse(bad).is_none(), "{bad}");
        }
        // A process ID that no longer runs the named program is refused.
        assert!(process_name(u32::MAX).is_none());
    }

    /// Plays a tone from this process on the default output and captures this
    /// process alone: what it plays must reach the capture, at its level.
    #[test]
    #[ignore = "needs a Windows output device"]
    fn captures_what_one_process_plays() {
        let tone = Ring::new(RATE, CHANNELS);
        let (writer, playing) = (Arc::clone(&tone), Arc::new(AtomicBool::new(true)));
        let still = Arc::clone(&playing);
        let source = thread::spawn(move || {
            let mut phase = 0.0f32;
            while still.load(Ordering::Acquire) {
                let block = (0..480).flat_map(|_| {
                    phase = (phase + 440.0 / RATE as f32).fract();
                    let sample = 0.5 * (phase * std::f32::consts::TAU).sin();
                    [sample, sample]
                });
                writer.push(block.collect::<Vec<_>>().into_iter());
                thread::sleep(Duration::from_millis(10));
            }
        });
        let output = crate::device::Output::open(None).unwrap();
        let _ = output.play_live(Arc::clone(&tone), 0.5);
        let me = std::process::id();
        let name = process_name(me).unwrap();
        let levels = Arc::new(Levels::new());
        let capturing = open(
            &format!("{me}:{name}"),
            |_, _| Meter::new(2_400),
            Arc::clone(&levels),
        )
        .unwrap();
        let start = std::time::Instant::now();
        while levels.latest() < -40.0 && start.elapsed() < Duration::from_secs(5) {
            thread::sleep(Duration::from_millis(20));
        }
        let heard = levels.latest();
        drop(capturing);
        playing.store(false, Ordering::Release);
        let _ = source.join();
        eprintln!("captured level: {heard:.1} dBFS");
        assert!(heard > -40.0, "heard {heard} dBFS");
    }

    /// Lists the applications playing sound on this machine; run it while some app
    /// plays audio.
    #[test]
    #[ignore = "needs a Windows audio session"]
    fn lists_the_applications_playing_sound() {
        let choices = choices().unwrap();
        eprintln!("{choices:#?}");
        assert!(
            choices
                .iter()
                .all(|(setting, _)| setting.starts_with(PREFIX))
        );
    }
}
