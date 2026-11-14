//! `orange-editor`: the desktop window of OrangeEditor.
//!
//! It owns the window and the host, and shows the editor interface of
//! `orange-editor-ui` in it. Everything the editor does goes through the host; this
//! file adds only what a desktop program has: a window, a file named on the command
//! line and the built-in service packages.
use orange_editor_ui::Editor;
use orange_host::{Host, HostEvent, HostOptions};
use std::process::ExitCode;
use std::sync::mpsc::Receiver;
use std::time::Duration;

/// Where the application keeps its credentials in the operating system's store.
const VAULT_SERVICE: &str = "orange";

struct Desktop {
    host: Host,
    events: Receiver<HostEvent>,
    editor: Editor,
}

impl eframe::App for Desktop {
    fn ui(&mut self, ui: &mut eframe::egui::Ui, _frame: &mut eframe::Frame) {
        let _ = self.editor.take_events(&self.events);
        self.editor.ui(ui, &mut self.host);
        // Flows report on their own threads: look again shortly while one runs.
        if self.editor.is_running() {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
    }
}

fn start(path: Option<String>) -> Result<Desktop, String> {
    let options = HostOptions {
        secrets: Some(std::sync::Arc::new(orange_vault::OsVault::new(
            VAULT_SERVICE,
        ))),
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
    if let Some(path) = path {
        let text = std::fs::read_to_string(&path).map_err(|error| format!("{path}: {error}"))?;
        editor.open(&mut host, path, text);
    }
    Ok(Desktop {
        host,
        events,
        editor,
    })
}

fn main() -> ExitCode {
    let desktop = match start(std::env::args().nth(1)) {
        Ok(desktop) => desktop,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::FAILURE;
        }
    };
    let shown = eframe::run_native(
        "OrangeEditor",
        eframe::NativeOptions::default(),
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
