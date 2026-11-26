//! Embed the canvas in an existing egui app and let the app own its chrome.
use std::{error::Error, sync::Arc, time::Duration};

use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily};
use orange_editor_ui::{Editor, FileRequest, i18n, icons};
use orange_host::{Events, Host, HostOptions};

const STARTER: &str = include_str!("../../orange-cli/graphs/power-with-count.json");

fn configure_appearance(ctx: &egui::Context, font: Option<Vec<u8>>) {
    // Keep the stock font chain as a fallback for characters the brand face lacks.
    if let Some(font) = font {
        let mut fonts = FontDefinitions::default();
        fonts
            .font_data
            .insert("brand".into(), Arc::new(FontData::from_owned(font)));
        if let Some(family) = fonts.families.get_mut(&FontFamily::Proportional) {
            family.insert(0, "brand".into());
        }
        ctx.set_fonts(fonts);
    }

    ctx.set_theme(egui::Theme::Dark);
    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();
    style.visuals.panel_fill = Color32::from_rgb(20, 31, 45);
    style.visuals.extreme_bg_color = Color32::from_rgb(13, 22, 33);
    style.visuals.selection.bg_fill = Color32::from_rgb(32, 102, 110);
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style
        .text_styles
        .insert(egui::TextStyle::Body, egui::FontId::proportional(15.0));
    ctx.set_style_of(egui::Theme::Dark, style);

    icons::set_command_painter(
        ctx,
        Some(Arc::new(|painter, rect, icon, color| {
            if icon != icons::PROPERTIES {
                return false;
            }
            painter.circle_stroke(
                rect.center(),
                rect.width().min(rect.height()) * 0.25,
                (1.5, color),
            );
            true
        })),
    );
}

struct App {
    editor: Editor,
    host: Host,
    events: Events,
    show_inspector: bool,
    notice: String,
}

impl App {
    fn handle_request(&mut self, request: FileRequest) -> Result<(), Box<dyn Error>> {
        let dialog = || rfd::FileDialog::new().add_filter("Orange graph", &["json"]);
        match request {
            FileRequest::New => self
                .editor
                .open_untitled(&mut self.host, orange_host::new_document()?),
            FileRequest::Open => {
                if let Some(path) = dialog().pick_file() {
                    let text = std::fs::read_to_string(&path)?;
                    self.editor
                        .open(&mut self.host, path.to_string_lossy(), text);
                }
            }
            FileRequest::SaveAs => {
                if let Some(path) = dialog().save_file() {
                    self.editor.save_as(path.to_string_lossy())?;
                }
            }
            _ => self.notice = "This example supports New, Open and Save as.".into(),
        }
        Ok(())
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.editor.poll_events(&self.events);
        // Poll at a modest rate in this sample. A larger host can use an event-driven wakeup.
        ui.ctx().request_repaint_after(Duration::from_millis(50));
        self.editor
            .ui_with_layout(ui, &mut self.host, |editor, ui, host| {
                egui::Panel::top("app-header").show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.heading("Signal workspace");
                        ui.checkbox(&mut self.show_inspector, "Inspector");
                        for (label, request) in [
                            ("New", FileRequest::New),
                            ("Open", FileRequest::Open),
                            ("Save as", FileRequest::SaveAs),
                        ] {
                            if ui.button(label).clicked() {
                                editor.ask(request);
                            }
                        }
                    });
                    if !self.notice.is_empty() {
                        ui.label(&self.notice);
                    }
                });
                if self.show_inspector {
                    egui::Panel::right("app-inspector").show(ui, |ui| editor.right_panel(ui, host));
                }
                egui::CentralPanel::default().show(ui, |ui| editor.canvas_panel(ui, host));
            });
        if let Some(request) = self.editor.take_request() {
            self.notice.clear();
            if let Err(error) = self.handle_request(request) {
                self.notice = error.to_string();
            }
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let font = std::env::args_os().nth(1).map(std::fs::read).transpose()?;
    let _ = i18n::add_application_catalogs(&[
        ("en", "properties = Inspector"),
        ("es", "properties = Inspector"),
    ]);
    let (mut host, events) = Host::new(HostOptions::default())?;
    let mut editor = Editor::new();
    editor.open_untitled(&mut host, STARTER.into());
    eframe::run_native(
        "OrangeEditor embedding example",
        eframe::NativeOptions::default(),
        Box::new(move |creation| {
            configure_appearance(&creation.egui_ctx, font);
            Ok(Box::new(App {
                editor,
                host,
                events,
                show_inspector: true,
                notice: String::new(),
            }))
        }),
    )?;
    Ok(())
}
