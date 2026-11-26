//! Browser-only controls example embedded in the OrangeEditor documentation.
#![cfg(target_arch = "wasm32")]

use eframe::egui::{self, Ui};
use eframe_web as eframe;
use orange_ui_controls::{
    Surface, UiTokens, apply_desktop_style, continuous_slider, knob, pill_segmented,
};
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

/// Keeps the egui runner alive while the example is visible.
#[wasm_bindgen]
pub struct ControlsDemo {
    runner: eframe::WebRunner,
}

#[wasm_bindgen]
impl ControlsDemo {
    /// Starts the controls example on the supplied canvas.
    pub async fn start(canvas: HtmlCanvasElement) -> Result<ControlsDemo, JsValue> {
        let runner = eframe::WebRunner::new();
        let started = runner
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(|context| {
                    apply_desktop_style(&context.egui_ctx);
                    context.egui_ctx.set_theme(egui::Theme::Dark);
                    Ok(Box::new(Gallery::default()))
                }),
            )
            .await;
        if let Err(error) = started {
            runner.destroy();
            return Err(error);
        }
        Ok(Self { runner })
    }

    /// Releases the canvas, callbacks and renderer.
    pub fn stop(&self) {
        self.runner.destroy();
    }
}

impl std::fmt::Debug for ControlsDemo {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ControlsDemo")
            .finish_non_exhaustive()
    }
}

struct Gallery {
    theme: egui::Theme,
    level: f32,
}

impl Default for Gallery {
    fn default() -> Self {
        Self {
            theme: egui::Theme::Dark,
            level: 55.0,
        }
    }
}

impl eframe::App for Gallery {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            let label = ui.label("Appearance");
            if pill_segmented(
                ui,
                UiTokens::resolve(ui.ctx()),
                &label,
                &mut self.theme,
                &[(egui::Theme::Light, "Light"), (egui::Theme::Dark, "Dark")],
            )
            .changed()
            {
                ui.ctx().set_theme(self.theme);
            }

            ui.add_space(8.0);
            ui.label(format!("Level  {:.0}%", self.level));
            continuous_slider(ui, Surface::Card, &mut self.level, 0.0..=100.0);
            knob(ui, Surface::Card, &mut self.level, 0.0..=100.0, None, &[]);
        });
    }
}
