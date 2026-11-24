use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    time::Duration,
};

use super::{
    advanced::{Glyph, icon_button},
    playback::Playback,
};
use egui::Ui;
use orange_ui_controls::{
    Divider, UiTokens, components, navigation,
    patterns::{self, CheckState},
};

fn playback_gain(db: f32) -> f32 {
    if db <= -60.0 {
        0.0
    } else {
        10.0_f32.powf(db.min(0.0) / 20.0)
    }
}
#[cfg(test)]
#[test]
fn playback_fader_has_silence_unity_and_equal_decibel_steps() {
    assert_eq!(playback_gain(-60.0), 0.0);
    assert_eq!(playback_gain(0.0), 1.0);
    assert!((playback_gain(-20.0) - 0.1).abs() < 1e-6);
    assert!((playback_gain(-40.0) - 0.01).abs() < 1e-6);
}

struct Clip {
    name: String,
    metadata: String,
    seconds: f32,
    waveform: Vec<f32>,
    playback: Option<(Arc<[f32]>, u32)>,
}

impl Clip {
    fn fixture() -> Self {
        Self {
            name: "rehearsal.wav".into(),
            metadata: "24-bit · 48 kHz · 01:42 · Example".into(),
            seconds: 102.0,
            playback: None,
            waveform: (0..480u16)
                .map(|i| {
                    let t = f32::from(i);
                    let envelope = (t * 0.025).sin().abs() * (0.8 + 0.2 * (t * 0.43).sin());
                    envelope * (t * 1.49).sin()
                })
                .collect(),
        }
    }

    fn read(path: &Path) -> Result<Self, String> {
        let mut file = hound::WavReader::open(path).map_err(|error| error.to_string())?;
        let spec = file.spec();
        let length = file.len();
        if length == 0 || spec.sample_rate == 0 {
            return Err("The WAV has no audio samples.".into());
        }
        let channels = usize::from(spec.channels);
        if channels == 0 || length as usize / channels > 16_000_000 {
            return Err("The WAV exceeds the preview limit of 16 million frames.".into());
        }
        const BUCKETS: usize = 384;
        let mut peaks = [0.0_f32; BUCKETS];
        let mut audio = Vec::with_capacity(length as usize / channels);
        let mut sum = 0.0;
        let mut push = |index: usize, sample: f32| {
            let sample = if sample.is_finite() {
                sample.clamp(-1.0, 1.0)
            } else {
                0.0
            };
            sum += sample;
            if index % channels == channels - 1 {
                audio.push(sum / f32::from(spec.channels));
                sum = 0.0;
            }
            let slot = index.saturating_mul(BUCKETS) / length as usize;
            if let Some(peak) = peaks.get_mut(slot) {
                *peak = peak.max(sample.abs().clamp(0.0, 1.0));
            }
        };
        match spec.sample_format {
            hound::SampleFormat::Float => {
                for (index, sample) in file.samples::<f32>().enumerate() {
                    push(index, sample.map_err(|error| error.to_string())?);
                }
            }
            hound::SampleFormat::Int => {
                let scale = 2.0_f32.powi(i32::from(spec.bits_per_sample) - 1);
                for (index, sample) in file.samples::<i32>().enumerate() {
                    push(
                        index,
                        sample.map_err(|error| error.to_string())? as f32 / scale,
                    );
                }
            }
        }
        let seconds = file.duration() as f32 / spec.sample_rate as f32;
        let whole = seconds as u32;
        Ok(Self {
            name: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            metadata: format!(
                "{}-bit · {:.1} kHz · {:02}:{:02}",
                spec.bits_per_sample,
                spec.sample_rate as f32 / 1000.0,
                whole / 60,
                whole % 60
            ),
            seconds,
            waveform: peaks.into_iter().flat_map(|peak| [peak, -peak]).collect(),
            playback: (!audio.is_empty()).then(|| (audio.into(), spec.sample_rate)),
        })
    }
}

pub(super) struct Files {
    clip: Option<Clip>,
    loading: Option<mpsc::Receiver<Result<Clip, String>>>,
    error: Option<String>,
    position: f32,
    playing: bool,
    tick: Option<f64>,
    playback: Option<Playback>,
    volume: f32,
    checked: [bool; 3],
    folder: PathBuf,
    browsed_folder: bool,
    folders: HashMap<PathBuf, Option<Arc<[FolderEntry]>>>,
    pending_folder: Option<(PathBuf, DirectoryReply)>,
    file_filter: String,
}

type DirectoryReply = mpsc::Receiver<Option<Arc<[FolderEntry]>>>;
struct FolderEntry {
    path: PathBuf,
    name: String,
    directory: bool,
}

impl Default for Files {
    fn default() -> Self {
        Self {
            clip: Some(Clip::fixture()),
            loading: None,
            error: None,
            position: 0.0,
            playing: false,
            tick: None,
            playback: None,
            volume: -2.0,
            checked: [true, false, false],
            folder: PathBuf::from("Projects/Audio/Session 01"),
            browsed_folder: false,
            folders: HashMap::new(),
            pending_folder: None,
            file_filter: String::new(),
        }
    }
}

impl Files {
    pub(super) fn seek_to(&mut self, seconds: f32) {
        if let Some(clip) = &self.clip {
            self.position = (seconds / clip.seconds).clamp(0.0, 1.0);
            if let Some(playback) = &self.playback {
                playback.seek(self.position);
            }
        }
    }

    fn load(&mut self, ctx: &egui::Context, path: PathBuf) {
        if self.loading.is_some() {
            return;
        }
        let ctx = ctx.clone();
        let (sender, receiver) = mpsc::channel();
        self.loading = Some(receiver);
        self.error = None;
        std::thread::spawn(move || {
            let result = Clip::read(&path);
            let _ = sender.send(result);
            ctx.request_repaint();
        });
    }

    fn update(&mut self) {
        let Some(receiver) = &self.loading else {
            return;
        };
        match receiver.try_recv() {
            Ok(result) => {
                self.loading = None;
                match result {
                    Ok(clip) => {
                        self.clip = Some(clip);
                        self.position = 0.0;
                        self.playing = false;
                        self.playback = None;
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.loading = None;
                self.error = Some("The audio preview worker stopped.".into());
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
    }

    pub(super) fn show(&mut self, ui: &mut Ui, tokens: UiTokens) {
        self.update();
        let theme = tokens.theme;
        ui.heading("Audio files and folders");
        let dropzone = components::card(ui, tokens, |ui| {
            ui.set_min_width(ui.available_width());
            ui.strong("Drop a local WAV here");
            if self.loading.is_some() {
                patterns::progress(ui, tokens, None, "Loading WAV");
            }
            if let Some(error) = &self.error {
                ui.colored_label(theme.danger, error);
            }
            ui.horizontal_wrapped(|ui| {
                if ui
                    .button((Glyph::Browse.atom(), "Browse / Replace"))
                    .clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("WAV", &["wav"])
                        .pick_file()
                {
                    self.load(ui.ctx(), path);
                }
                if ui.button("Clear").clicked() {
                    self.clip = None;
                    self.playing = false;
                    self.playback = None;
                }
            });
        })
        .response;
        if ui.rect_contains_pointer(dropzone.rect)
            && let Some(path) = ui.input(|input| {
                input
                    .raw
                    .dropped_files
                    .iter()
                    .map(|file| file.path())
                    .find(|path| !path.as_os_str().is_empty())
                    .map(Path::to_path_buf)
            })
        {
            self.load(ui.ctx(), path);
        }

        if let Some(clip) = &self.clip {
            ui.strong(&clip.name);
            ui.weak(&clip.metadata);
            if clip.playback.is_none() {
                ui.weak("Waveform demo · Load a WAV for audio playback");
            }
            if let Some(playback) = &self.playback {
                self.position = playback.position();
                if playback.fault() {
                    self.error = Some("The audio output was interrupted.".into());
                    self.playback = None;
                    self.playing = false;
                } else if self.position >= 1.0 {
                    self.playback = None;
                    self.playing = false;
                }
            }
            let prior = self.position;
            super::audio::waveform(ui, tokens, &clip.waveform, Some(&mut self.position));
            if self.position != prior
                && let Some(playback) = &self.playback
            {
                playback.seek(self.position);
            }
            ui.horizontal_wrapped(|ui| {
                let now = ui.input(|input| input.time);
                if icon_button(
                    ui,
                    if self.playing {
                        Glyph::Pause
                    } else {
                        Glyph::Play
                    },
                    if self.playing {
                        "Pause preview"
                    } else {
                        "Play preview"
                    },
                )
                .clicked()
                {
                    self.playing = !self.playing;
                    if self.position >= 1.0 {
                        self.position = 0.0;
                    }
                    self.tick = None;
                    if self.playing
                        && let Some((audio, rate)) = &clip.playback
                    {
                        match Playback::start(
                            audio.clone(),
                            *rate,
                            self.position,
                            playback_gain(self.volume),
                        ) {
                            Ok(playback) => {
                                self.playback = Some(playback);
                                self.error = None;
                            }
                            Err(error) => {
                                self.error = Some(error);
                                self.playing = false;
                            }
                        }
                    } else if !self.playing {
                        self.playback = None;
                    }
                }
                if ui.button("Stop").clicked() {
                    self.playback = None;
                    self.playing = false;
                    self.position = 0.0;
                }
                for (label, seconds) in [("-5 s", -5.0), ("+5 s", 5.0)] {
                    if ui.button(label).clicked() {
                        self.position = (self.position + seconds / clip.seconds).clamp(0.0, 1.0);
                        if let Some(playback) = &self.playback {
                            playback.seek(self.position);
                        }
                    }
                }
                if self.playing && clip.playback.is_none() {
                    let dt = self
                        .tick
                        .replace(now)
                        .map_or(0.0, |then| (now - then).max(0.0));
                    self.position = (self.position + dt as f32 / clip.seconds.max(0.01)).min(1.0);
                    self.playing = self.position < 1.0;
                }
                if self.playing {
                    ui.ctx().request_repaint_after(Duration::from_millis(16));
                }
                let display = |time: f32| {
                    let time = time as u32;
                    format!("{:02}:{:02}", time / 60, time % 60)
                };
                ui.monospace(format!(
                    "{} / {}",
                    display(self.position * clip.seconds),
                    display(clip.seconds)
                ));
            });
            ui.vertical(|ui| {
                ui.set_max_width(ui.available_width().min(280.0));
                let label = ui.label(if self.volume <= -60.0 {
                    "Volume · muted".to_owned()
                } else {
                    format!("Volume · {:.1} dB", self.volume)
                });
                if orange_ui_controls::continuous_slider(
                    ui,
                    orange_ui_controls::Surface::Card,
                    &mut self.volume,
                    -60.0..=0.0,
                )
                .labelled_by(label.id)
                .changed()
                    && let Some(playback) = &self.playback
                {
                    playback.set_volume(playback_gain(self.volume));
                }
            });
        } else {
            patterns::empty_state(
                ui,
                tokens,
                "No audio loaded",
                "Drop a WAV to preview its waveform.",
            );
        }
        Divider::Section.show(ui, tokens);
        self.folder(ui, tokens);
    }

    fn folder(&mut self, ui: &mut Ui, tokens: UiTokens) {
        ui.horizontal_wrapped(|ui| {
            ui.strong("Project folder");
            if ui.button((Glyph::Browse.atom(), "Browse folder")).clicked()
                && let Some(path) = rfd::FileDialog::new().pick_folder()
            {
                self.folder = path;
                self.browsed_folder = true;
                self.folders.clear();
                self.pending_folder = None;
            }
            if self.browsed_folder && ui.button("Refresh tree").clicked() {
                self.folders.clear();
                self.pending_folder = None;
            }
        });
        let paths: Vec<_> = self
            .folder
            .ancestors()
            .take_while(|path| !path.as_os_str().is_empty())
            .collect();
        let names: Vec<_> = paths
            .iter()
            .rev()
            .map(|path| {
                path.file_name()
                    .unwrap_or(path.as_os_str())
                    .to_string_lossy()
            })
            .collect();
        let labels = names.iter().map(|name| name.as_ref()).collect::<Vec<_>>();
        if let Some(index) = navigation::breadcrumbs(ui, &labels) {
            self.folder = paths[paths.len() - 1 - index].to_path_buf();
        }
        if self.browsed_folder {
            super::advanced::search_field(
                ui,
                "tree-filter",
                &mut self.file_filter,
                "Filter files in expanded folders",
                ui.available_width(),
            );
            egui::ScrollArea::vertical()
                .max_height(220.0)
                .show(ui, |ui| {
                    self.path_tree(ui, &self.folder.clone(), 0);
                });
        } else {
            egui::CollapsingHeader::new("Session 01")
                .default_open(true)
                .show(ui, |ui| {
                    let count = self.checked.iter().filter(|&&checked| checked).count();
                    let mut state = match count {
                        0 => CheckState::Off,
                        3 => CheckState::On,
                        _ => CheckState::Mixed,
                    };
                    if patterns::tri_checkbox(ui, tokens, "Select all", &mut state).changed() {
                        self.checked.fill(state == CheckState::On);
                    }
                    for (index, name) in ["rehearsal.wav", "voice.wav", "room.wav"]
                        .into_iter()
                        .enumerate()
                    {
                        self.fixture_file(ui, index, name);
                    }
                });
        }
    }

    fn fixture_file(&mut self, ui: &mut Ui, index: usize, name: &str) {
        ui.horizontal(|ui| {
            let glyph = super::advanced::icon(ui, Glyph::Wave);
            glyph.on_hover_text("WAV audio");
            ui.checkbox(&mut self.checked[index], name);
            if icon_button(ui, Glyph::Play, &format!("Preview {name}")).clicked() {
                let mut clip = Clip::fixture();
                clip.name = name.into();
                self.clip = Some(clip);
                self.position = 0.0;
                self.playing = true;
                self.tick = None;
                self.playback = None;
            }
        });
    }

    fn path_tree(&mut self, ui: &mut Ui, directory: &Path, depth: usize) {
        if let Some((path, receiver)) = &self.pending_folder {
            match receiver.try_recv() {
                Ok(result) => {
                    self.folders.insert(path.clone(), result);
                    self.pending_folder = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.folders.insert(path.clone(), None);
                    self.pending_folder = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        let entries = match self.folders.get(directory) {
            Some(Some(entries)) => Arc::clone(entries),
            Some(None) => {
                ui.weak("Folder unavailable");
                return;
            }
            None => {
                if self.pending_folder.is_none() {
                    let path = directory.to_owned();
                    let (tx, rx) = mpsc::sync_channel(1);
                    self.pending_folder = Some((path.clone(), rx));
                    let ctx = ui.ctx().clone();
                    std::thread::spawn(move || {
                        let result = read_folder(&path);
                        let _ = tx.send(result);
                        ctx.request_repaint();
                    });
                }
                ui.weak("Loading folder…");
                return;
            }
        };
        for FolderEntry {
            path,
            name,
            directory,
        } in entries.iter()
        {
            if !directory && !name.contains(&self.file_filter) {
                continue;
            }
            ui.push_id(path, |ui| {
                if *directory && depth < 6 {
                    egui::CollapsingHeader::new(name)
                        .show(ui, |ui| self.path_tree(ui, path, depth + 1));
                } else if !directory {
                    ui.horizontal(|ui| {
                        super::advanced::icon(ui, Glyph::Wave);
                        let id = ui.scope_id().with("selected");
                        let mut selected = ui.ctx().data(|data| data.get_temp(id)).unwrap_or(false);
                        if ui.checkbox(&mut selected, name).changed() {
                            ui.ctx().data_mut(|data| data.insert_temp(id, selected));
                        }
                        if icon_button(ui, Glyph::Play, "Load audio preview").clicked() {
                            self.load(ui.ctx(), path.clone());
                        }
                    });
                }
            });
        }
    }
}

fn read_folder(directory: &Path) -> Option<Arc<[FolderEntry]>> {
    let entries = directory.read_dir().ok()?;
    let mut folders: Vec<_> = entries
        .take(512)
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let kind = entry.file_type().ok()?;
            let path = entry.path();
            let wav = kind.is_file()
                && path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("wav"));
            (kind.is_dir() || wav).then(|| FolderEntry {
                path,
                name: entry.file_name().to_string_lossy().into_owned(),
                directory: kind.is_dir(),
            })
        })
        .collect();
    folders.sort_by(|a, b| b.directory.cmp(&a.directory).then(a.name.cmp(&b.name)));
    Some(folders.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loaded_stereo_wav_provides_preview_peaks_and_mono_playback_frames() {
        let path =
            std::env::temp_dir().join(format!("orange-gallery-preview-{}.wav", std::process::id()));
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        for _ in 0..480 {
            writer.write_sample(16_384_i16).unwrap();
            writer.write_sample(0_i16).unwrap();
        }
        writer.finalize().unwrap();
        let loaded = Clip::read(&path);
        std::fs::remove_file(&path).unwrap();
        let clip = loaded.unwrap();
        assert!((clip.seconds - 0.01).abs() < f32::EPSILON);
        assert!(clip.waveform.contains(&0.5));
        let (playback, rate) = clip.playback.unwrap();
        assert_eq!(rate, 48_000);
        assert_eq!(playback.len(), 480);
        assert!(playback.iter().all(|&sample| sample == 0.25));
    }
}
