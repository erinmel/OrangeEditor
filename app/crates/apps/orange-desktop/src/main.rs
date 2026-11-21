//! `orange-editor`: the desktop window of OrangeEditor.
//!
//! It owns the window and the host, and shows the editor interface of
//! `orange-editor-ui` in it. Everything the editor does goes through the host; this
//! file adds only what a desktop program has: a window, files named on the command
//! line or chosen in the system's file dialogs, and the built-in service packages.
#[cfg(test)]
mod automation;
#[cfg(test)]
mod gallery;
mod keyboard;
mod logbook;
mod picture;

include!(concat!(env!("OUT_DIR"), "/locales.rs"));

use orange_editor_ui::canvas::Canvas;
use orange_editor_ui::i18n::{self, t, tf};
use orange_editor_ui::{Editor, FileRequest};
use orange_host::{GraphView, Host, HostOptions};
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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
    keyboard: keyboard::Keyboard,
    /// Messages Log message nodes wrote, waiting for the editor's Log.
    logbook: logbook::LogBook,
    host: Host,
    events: orange_host::Events,
    editor: Editor,
    /// With `--canvas-only`: the graph and its canvas, shown without the editor.
    bare: Option<(GraphView, Canvas)>,
    shot: Option<Shot>,
    title: String,
}

fn preferences_path() -> Option<std::path::PathBuf> {
    let base = if cfg!(target_os = "windows") {
        std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME")
            .map(|home| std::path::PathBuf::from(home).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".config"))
            })
    };
    base.map(|base| base.join("OrangeEditor").join("editor-preferences.json"))
}

impl Drop for Desktop {
    fn drop(&mut self) {
        let Some(path) = preferences_path() else {
            return;
        };
        let Some(parent) = path.parent() else { return };
        let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
        let result = std::fs::create_dir_all(parent)
            .and_then(|()| std::fs::write(&temporary, self.editor.preferences().to_string()))
            .and_then(|()| std::fs::rename(&temporary, &path));
        if let Err(error) = result {
            eprintln!("Could not save editor preferences: {error}");
        }
    }
}

impl eframe::App for Desktop {
    fn ui(&mut self, ui: &mut eframe::egui::Ui, _frame: &mut eframe::Frame) {
        // What the graph logged, between frames, then the run's own events.
        let (notes, dropped) = self.logbook.take();
        for (level, text) in &notes {
            self.editor.note(*level, text);
        }
        self.editor.notes_dropped(dropped);
        if self.editor.take_events(&self.events) || !notes.is_empty() {
            // A bounded poll may leave a backlog, including events from retired flows.
            ui.ctx().request_repaint();
        }
        if let Some(request) = self.editor.take_request() {
            self.carry_out(request);
        }
        if self.bare.is_none() {
            let title = window_title(self.editor.path());
            if self.title != title {
                self.title = title.into_owned();
                ui.ctx()
                    .send_viewport_cmd(eframe::egui::ViewportCommand::Title(self.title.clone()));
            }
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
        let (events, focused) = ui.input(|input| (input.events.clone(), input.focused));
        let editing = ui.ctx().text_edit_focused() || ui.ctx().any_popup_open();
        self.keyboard.handle(&events, focused, |key, modifiers| {
            !editing && self.editor.accepts_graph_key(key, modifiers)
        });
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

impl Desktop {
    /// Asks the person for the file or folder a node's input holds, starting where the
    /// input points now, and puts it on the input.
    fn choose_path(
        &mut self,
        node: orange_host::NodeId,
        port: &str,
        (folder, save): (bool, bool),
        extensions: &[String],
        current: &str,
    ) {
        let folder_connection = self.editor.file_picker_folder(node);
        let folder_name = folder_connection
            .flatten()
            .map(|connection| connection.name.as_str());
        let root = match folder_connection {
            Some(connection) => match connection
                .and_then(|connection| connection.settings["path"].as_str())
                .filter(|root| !root.trim().is_empty())
                .and_then(|root| std::fs::canonicalize(root).ok())
                .filter(|root| root.is_dir())
            {
                Some(root) => Some(root),
                None => {
                    let message = folder_name.map_or_else(
                        || t("desktop-configure-folder").to_owned(),
                        |name| tf("desktop-configure-named-folder", &[("folder", name.into())]),
                    );
                    trouble(t("desktop-choose-file"), &message);
                    return;
                }
            },
            None => None,
        };
        let current = root.as_ref().map_or_else(
            || Path::new(current).to_path_buf(),
            |root| root.join(current),
        );
        let start = if folder {
            Some(current.as_path())
        } else {
            current.parent()
        };
        let mut dialog = rfd::FileDialog::new();
        if let Some(start) = start.filter(|start| start.is_dir()).or(root.as_deref()) {
            dialog = dialog.set_directory(start);
        }
        let chosen = if folder {
            dialog.set_title(t("desktop-choose-folder")).pick_folder()
        } else {
            if !extensions.is_empty() {
                dialog = dialog.add_filter(extensions.join(", "), extensions);
            }
            if save {
                if let Some(name) = current.file_name() {
                    dialog = dialog.set_file_name(name.to_string_lossy());
                }
                dialog.set_title(t("desktop-save-as")).save_file()
            } else {
                dialog.set_title(t("desktop-choose-file")).pick_file()
            }
        };
        if let Some(path) = chosen {
            let path = if let Some(root) = &root {
                let Some(relative) = relative_selection(root, &path, save) else {
                    let message = folder_name.map_or_else(
                        || t("desktop-outside-folder").to_owned(),
                        |name| tf("desktop-outside-named-folder", &[("folder", name.into())]),
                    );
                    trouble(t("desktop-choose-file"), &message);
                    return;
                };
                relative
            } else {
                path.display().to_string()
            };
            let _ = self.editor.choose_path(&mut self.host, node, port, &path);
        }
    }
}

fn relative_selection(root: &Path, selected: &Path, save: bool) -> Option<String> {
    let resolved = match std::fs::canonicalize(selected) {
        Ok(path) => path,
        Err(error) if save && error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::canonicalize(selected.parent()?)
                .ok()?
                .join(selected.file_name()?)
        }
        Err(_) => return None,
    };
    resolved
        .strip_prefix(root)
        .ok()?
        .iter()
        .map(std::ffi::OsStr::to_str)
        .collect::<Option<Vec<_>>>()
        .map(|parts| parts.join("/"))
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
    fn import_package(&mut self) {
        use std::io::Read;

        let Some(path) = rfd::FileDialog::new()
            .set_title(t("import-package"))
            .add_filter(t("desktop-package-filter"), &["orangeplug"])
            .pick_file()
        else {
            return;
        };
        // Match the host archive limit, including the header.
        const MAX_ARCHIVE_BYTES: u64 = 256 << 20;
        let mut bytes = Vec::new();
        let read = std::fs::File::open(&path)
            .and_then(|file| file.take(MAX_ARCHIVE_BYTES + 1).read_to_end(&mut bytes));
        if let Err(error) = read {
            trouble(t("desktop-package-error"), &error.to_string());
            return;
        }
        if bytes.len() as u64 > MAX_ARCHIVE_BYTES {
            trouble(t("desktop-package-error"), t("desktop-package-too-large"));
            return;
        }
        let name = path
            .file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy();
        if let Err(error) = self
            .editor
            .review_package(&self.host, name.into_owned(), bytes)
        {
            trouble(t("desktop-package-error"), &error.to_string());
        }
    }

    fn carry_out(&mut self, request: FileRequest) {
        if let FileRequest::ExportPackage { name, archive } = request {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter(t("desktop-package-filter"), &["orangeplug"])
                .set_file_name(name)
                .save_file()
                && let Err(error) = std::fs::write(path, archive)
            {
                trouble(t("desktop-not-saved"), &error.to_string());
            }
            return;
        }
        if request == FileRequest::ImportPackage {
            self.import_package();
            return;
        }
        if let FileRequest::ChooseConnectionFolder { current } = &request {
            let mut dialog = rfd::FileDialog::new().set_title(t("desktop-choose-folder"));
            if Path::new(current).is_dir() {
                dialog = dialog.set_directory(current);
            }
            if let Some(path) = dialog.pick_folder() {
                let _ = self
                    .editor
                    .choose_connection_folder(&path.to_string_lossy());
            }
            return;
        }
        if let FileRequest::ChoosePath {
            node,
            port,
            folder,
            save,
            extensions,
            current,
        } = &request
        {
            self.choose_path(*node, port, (*folder, *save), extensions, current);
            return;
        }
        // Edits that are not saved are not dropped without asking.
        if matches!(request, FileRequest::New | FileRequest::Open) && self.editor.is_unsaved() {
            let answer = rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Warning)
                .set_title(t("desktop-unsaved-title"))
                .set_description(t("desktop-unsaved-question"))
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
        let mut dialog = rfd::FileDialog::new().add_filter(t("desktop-graph-filter"), &["json"]);
        if let Some(folder) = folder {
            dialog = dialog.set_directory(folder);
        }
        match request {
            FileRequest::New => match orange_host::new_document() {
                Ok(text) => self.editor.open_untitled(&mut self.host, text),
                Err(error) => trouble(t("desktop-no-new-graph"), &error.to_string()),
            },
            FileRequest::Open => {
                let Some(path) = dialog.set_title(t("desktop-open-graph")).pick_file() else {
                    return;
                };
                match std::fs::read_to_string(&path) {
                    Ok(text) => self
                        .editor
                        .open(&mut self.host, path.display().to_string(), text),
                    Err(error) => trouble(
                        t("desktop-not-opened"),
                        &format!("{}: {error}", path.display()),
                    ),
                }
            }
            FileRequest::ChoosePath { .. }
            | FileRequest::ChooseConnectionFolder { .. }
            | FileRequest::ImportPackage
            | FileRequest::ExportPackage { .. } => {}
            FileRequest::SaveAs => {
                let name = current
                    .and_then(Path::file_name)
                    .map_or_else(|| "graph.json".into(), |name| name.to_string_lossy());
                let Some(path) = dialog
                    .set_title(t("desktop-save-graph-as"))
                    .set_file_name(name)
                    .save_file()
                else {
                    return;
                };
                if let Err(message) = self.editor.save_as(path.display().to_string()) {
                    trouble(t("desktop-not-saved"), &message);
                }
            }
        }
    }
}

fn window_title(path: Option<&str>) -> std::borrow::Cow<'_, str> {
    path.and_then(|path| Path::new(path).file_name())
        .map_or_else(|| "OrangeEditor".into(), |name| name.to_string_lossy())
}

fn built_in_services(
    keyboard: &keyboard::Keyboard,
    logbook: &logbook::LogBook,
) -> Result<Vec<orange_sdk::ServicePackage>, String> {
    Ok(vec![
        keyboard.package().map_err(|error| error.to_string())?,
        logbook.package().map_err(|error| error.to_string())?,
        orange_net::package(),
        orange_files::package(),
        orange_audio::package(),
    ])
}

fn start(
    path: Option<String>,
    canvas_only: bool,
    screenshot: Option<String>,
) -> Result<Desktop, String> {
    let options = HostOptions {
        secrets: Some(Arc::new(orange_vault::OsVault::new(VAULT_SERVICE))),
        package_dir: preferences_path()
            .and_then(|path| path.parent().map(|dir| dir.join("packages"))),
        ..HostOptions::default()
    };
    let keyboard = keyboard::Keyboard::default();
    let logbook = logbook::LogBook::default();
    let services = built_in_services(&keyboard, &logbook)?;
    let (mut host, events) =
        Host::with_services(options, services).map_err(|error| error.to_string())?;
    let mut editor = Editor::new();
    // The system's language until the person chooses one.
    if let Some(system) = sys_locale::get_locale() {
        editor.set_language(&i18n::nearest_language(&system));
    }
    if let Some(saved) = preferences_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
    {
        editor.restore_preferences(&saved);
    }
    editor.file_dialogs = true;
    let mut bare = None;
    let title = window_title(path.as_deref()).into_owned();
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
        keyboard,
        logbook,
        host,
        events,
        editor,
        bare,
        title,
        shot: screenshot.map(|path| Shot {
            path,
            frames: 0,
            asked: false,
            written: Arc::new(AtomicBool::new(false)),
        }),
    })
}

fn main() -> ExitCode {
    let _ = i18n::add_application_catalogs(CATALOGS);
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
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title(desktop.title.clone())
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([820.0, 560.0])
            .with_resizable(true)
            .with_decorations(true),
        ..eframe::NativeOptions::default()
    };
    let shown = eframe::run_native(
        "OrangeEditor",
        options,
        Box::new(move |creation| {
            creation
                .egui_ctx
                .set_theme(eframe::egui::ThemePreference::System);
            orange_editor_ui::apply_theme(&creation.egui_ctx);
            orange_editor_ui::icons::install(&creation.egui_ctx);
            Ok(Box::new(desktop))
        }),
    );
    match shown {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("the window could not be opened: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use orange_host::Catalogs;

    /// Every language has the desktop's English messages, and no others.
    #[test]
    fn the_desktop_text_is_in_every_language() {
        let catalogs = Catalogs::from_sources(super::CATALOGS).unwrap();
        let desktop = |tag| -> Vec<&str> {
            catalogs
                .ids(tag)
                .into_iter()
                .filter(|id| id.starts_with("desktop-"))
                .collect()
        };
        let english = desktop("en");
        assert!(!english.is_empty());
        for (tag, _) in catalogs.languages() {
            assert_eq!(desktop(tag), english, "{tag}");
        }
    }
}
