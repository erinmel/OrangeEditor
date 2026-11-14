//! `orange-editor`: the desktop window of OrangeEditor.
//!
//! It owns the window and the host, and shows the editor interface of
//! `orange-editor-ui` in it. Everything the editor does goes through the host; this
//! file adds only what a desktop program has: a window, files named on the command
//! line or chosen in the system's file dialogs, and the built-in service packages.
mod picture;

use orange_editor_ui::canvas::Canvas;
use orange_editor_ui::{Editor, FileRequest};
use orange_host::{GraphView, Host, HostEvent, HostOptions};
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::time::Duration;

/// Where the application keeps its credentials in the operating system's store.
const VAULT_SERVICE: &str = "orange";
/// Frames drawn before a `--screenshot` is taken, so the layout has settled.
const SETTLE_FRAMES: u32 = 12;

const USAGE: &str = "\
usage:
  orange-editor [graph.json] [--canvas-only] [--screenshot <file.png>]

Opens the editor, with a graph if one is named. `--canvas-only` shows the graph alone,
as an application with its own panels embeds it. `--screenshot` saves a picture of the
window once it has drawn and then closes it.";

/// A picture to take of the window, and whether it has been written.
struct Shot {
    path: String,
    frames: u32,
    asked: bool,
    written: Arc<AtomicBool>,
}

struct Desktop {
    host: Host,
    events: Receiver<HostEvent>,
    editor: Editor,
    /// With `--canvas-only`: the graph and its canvas, shown without the editor.
    bare: Option<(GraphView, Canvas)>,
    shot: Option<Shot>,
}

impl eframe::App for Desktop {
    fn ui(&mut self, ui: &mut eframe::egui::Ui, _frame: &mut eframe::Frame) {
        let _ = self.editor.take_events(&self.events);
        if let Some(request) = self.editor.take_request() {
            self.carry_out(request);
        }
        match &mut self.bare {
            // All an embedding application does: show the canvas where it wants it.
            Some((graph, canvas)) => {
                let _ = eframe::egui::CentralPanel::default()
                    .frame(eframe::egui::Frame::NONE)
                    .show(ui, |ui| canvas.show(ui, graph, &[]));
            }
            None => self.editor.ui(ui, &mut self.host),
        }
        // Flows report on their own threads: look again shortly while one runs.
        if self.editor.is_running() {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
        if let Some(shot) = &mut self.shot {
            shot.frames += 1;
            if shot.written.load(Ordering::Acquire) {
                ui.ctx()
                    .send_viewport_cmd(eframe::egui::ViewportCommand::Close);
            } else if shot.frames >= SETTLE_FRAMES && !shot.asked {
                shot.asked = true;
                let (path, written) = (shot.path.clone(), Arc::clone(&shot.written));
                ui.ctx().request_screenshot(move |image| {
                    let rgba: Vec<u8> = image
                        .pixels
                        .iter()
                        .flat_map(eframe::egui::Color32::to_array)
                        .collect();
                    let saved = picture::png(image.size[0], image.size[1], &rgba)
                        .ok_or_else(|| "the picture could not be encoded".to_owned())
                        .and_then(|file| {
                            std::fs::write(&path, file).map_err(|error| error.to_string())
                        });
                    if let Err(message) = saved {
                        eprintln!("{path}: {message}");
                    }
                    written.store(true, Ordering::Release);
                });
            }
            ui.ctx().request_repaint();
        }
    }
}

/// Tells the person something went wrong, in a dialog of the system.
fn trouble(title: &str, message: &str) {
    let _ = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title(title)
        .set_description(message)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

impl Desktop {
    /// Carries out a file action of the editor with the system's dialogs. The dialogs
    /// wait for the person, as a desktop program's dialogs do.
    fn carry_out(&mut self, request: FileRequest) {
        // Edits that are not saved are not dropped without asking.
        if matches!(request, FileRequest::New | FileRequest::Open) && self.editor.is_unsaved() {
            let answer = rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Warning)
                .set_title("Changes not saved")
                .set_description("The graph has changes that are not saved. Leave them?")
                .set_buttons(rfd::MessageButtons::YesNo)
                .show();
            if answer != rfd::MessageDialogResult::Yes {
                return;
            }
        }
        let current = self.editor.path().map(Path::new);
        let folder = current
            .and_then(Path::parent)
            .filter(|folder| !folder.as_os_str().is_empty())
            .map(Path::to_path_buf);
        let mut dialog = rfd::FileDialog::new().add_filter("OrangeEditor graph", &["json"]);
        if let Some(folder) = folder {
            dialog = dialog.set_directory(folder);
        }
        match request {
            FileRequest::New => match orange_host::new_document() {
                Ok(text) => self.editor.open_untitled(&mut self.host, text),
                Err(error) => trouble("No new graph", &error.to_string()),
            },
            FileRequest::Open => {
                let Some(path) = dialog.set_title("Open a graph").pick_file() else {
                    return;
                };
                match std::fs::read_to_string(&path) {
                    Ok(text) => self
                        .editor
                        .open(&mut self.host, path.display().to_string(), text),
                    Err(error) => trouble("Not opened", &format!("{}: {error}", path.display())),
                }
            }
            FileRequest::SaveAs => {
                let name = current
                    .and_then(Path::file_name)
                    .map_or_else(|| "graph.json".into(), |name| name.to_string_lossy());
                let Some(path) = dialog
                    .set_title("Save the graph as")
                    .set_file_name(name)
                    .save_file()
                else {
                    return;
                };
                if let Err(message) = self.editor.save_as(path.display().to_string()) {
                    trouble("Not saved", &message);
                }
            }
        }
    }
}

fn start(
    path: Option<String>,
    canvas_only: bool,
    screenshot: Option<String>,
) -> Result<Desktop, String> {
    let options = HostOptions {
        secrets: Some(Arc::new(orange_vault::OsVault::new(VAULT_SERVICE))),
        ..HostOptions::default()
    };
    let services = vec![
        orange_net::package(),
        orange_files::package(),
        orange_audio::package(),
    ];
    let (mut host, events) =
        Host::with_services(options, services).map_err(|error| error.to_string())?;
    let mut editor = Editor::new();
    editor.file_dialogs = true;
    let mut bare = None;
    if let Some(path) = path {
        let text = std::fs::read_to_string(&path).map_err(|error| format!("{path}: {error}"))?;
        if canvas_only {
            let graph = host
                .view(&text)
                .map_err(|error| format!("{path}: {error}"))?;
            let mut canvas = Canvas::new();
            canvas.load(&graph);
            bare = Some((graph, canvas));
        } else {
            editor.open(&mut host, path, text);
        }
    } else {
        // Nothing named: a new graph, which asks where to go when it is first saved.
        let text = orange_host::new_document().map_err(|error| error.to_string())?;
        editor.open_untitled(&mut host, text);
    }
    // A picture shows more with a node selected.
    if screenshot.is_some() {
        let first = editor
            .graph()
            .and_then(|graph| graph.nodes.last().map(|node| node.id));
        editor.canvas().select(first);
    }
    Ok(Desktop {
        host,
        events,
        editor,
        bare,
        shot: screenshot.map(|path| Shot {
            path,
            frames: 0,
            asked: false,
            written: Arc::new(AtomicBool::new(false)),
        }),
    })
}

fn main() -> ExitCode {
    let (mut graph, mut screenshot, mut canvas_only) = (None, None, false);
    let mut words = std::env::args().skip(1);
    while let Some(word) = words.next() {
        match word.as_str() {
            "--screenshot" => match words.next() {
                Some(path) if path.ends_with(".png") => screenshot = Some(path),
                _ => {
                    eprintln!("--screenshot needs a file ending in .png\n{USAGE}");
                    return ExitCode::from(2);
                }
            },
            "--canvas-only" => canvas_only = true,
            option if option.starts_with("--") => {
                eprintln!("unknown option {option}\n{USAGE}");
                return ExitCode::from(2);
            }
            _ if graph.is_none() => graph = Some(word),
            _ => {
                eprintln!("one graph at a time\n{USAGE}");
                return ExitCode::from(2);
            }
        }
    }
    let desktop = match start(graph, canvas_only, screenshot) {
        Ok(desktop) => desktop,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::FAILURE;
        }
    };
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]),
        ..eframe::NativeOptions::default()
    };
    let shown = eframe::run_native(
        "OrangeEditor",
        options,
        Box::new(move |_creation| Ok(Box::new(desktop))),
    );
    match shown {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("the window could not be opened: {error}");
            ExitCode::FAILURE
        }
    }
}
