#[path = "gallery/advanced.rs"]
mod advanced;
#[path = "gallery/audio.rs"]
mod audio;
#[path = "gallery/catalog.rs"]
mod catalog;
#[path = "gallery/files.rs"]
mod files;
#[path = "gallery/playback.rs"]
mod playback;
#[cfg(test)]
#[path = "gallery/tests.rs"]
mod tests;

use catalog::{ENTRIES, Fixture, State};
use eframe::egui::{self, Frame, RichText, Ui};
use orange_ui_controls::{Density, Divider, ThemeMode, UiTokens, components, theme::Theme};

const PALETTES: [(&str, Theme); 9] = [
    ("Light", Theme::LIGHT),
    ("Dark", Theme::DARK),
    ("Monokai", Theme::MONOKAI),
    ("Bubblegum", Theme::BUBBLEGUM),
    ("High Contrast Light", Theme::high_contrast_light()),
    ("High Contrast Dark", Theme::high_contrast_dark()),
    ("Spruce", Theme::SPRUCE),
    ("Parchment", Theme::PARCHMENT),
    ("Midnight Contrast", Theme::MIDNIGHT_CONTRAST),
];

/// Keeps the scroll extent stable while its measured content is outside the clip.
#[derive(Default, Clone, Copy)]
struct PreviewHeight(Option<(u32, Density, f32)>);

impl PreviewHeight {
    fn show(&mut self, ui: &mut Ui, tokens: UiTokens, contents: impl FnOnce(&mut Ui)) -> bool {
        let width = ui.available_width();
        let key = (width.to_bits(), tokens.density);
        if let Some((measured_width, density, height)) = self.0
            && (measured_width, density) == key
            && !ui.is_rect_visible(egui::Rect::from_min_size(
                ui.next_widget_position(),
                egui::vec2(width, height),
            ))
        {
            ui.allocate_space(egui::vec2(width, height));
            return false;
        }
        let bounds = ui.scope(contents).response.rect;
        self.0 = Some((key.0, key.1, bounds.height()));
        true
    }
}

struct Gallery {
    fixtures: Vec<[Fixture; 2]>,
    entry_heights: Vec<PreviewHeight>,
    audio_height: PreviewHeight,
    patterns_height: PreviewHeight,
    palette: usize,
    density: Density,
    audio: audio::Preview,
    shell: advanced::Shell,
    patterns: advanced::Catalog,
    files: files::Files,
    shot: Option<Shot>,
}

/// With `ORANGE_GALLERY_SHOT=<file.bmp>` the gallery saves one picture of its window once
/// it has settled, then closes, so the controls can be checked without someone watching.
/// `ORANGE_GALLERY_SIZE=<width>x<height>` sets the window size for it.
struct Shot {
    path: String,
    frames: u32,
    asked: bool,
    written: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Shot {
    fn from_env() -> Option<Self> {
        Some(Self {
            path: std::env::var("ORANGE_GALLERY_SHOT").ok()?,
            frames: 0,
            asked: false,
            written: std::sync::Arc::default(),
        })
    }

    fn take(&mut self, ui: &Ui) {
        use std::sync::atomic::Ordering;
        self.frames += 1;
        if self.written.load(Ordering::Acquire) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        } else if self.frames >= 30 && !self.asked {
            self.asked = true;
            let (path, written) = (self.path.clone(), std::sync::Arc::clone(&self.written));
            ui.ctx().request_screenshot(move |image| {
                let [width, height] = image.size;
                if let Err(error) = std::fs::write(&path, bmp(width, height, &image.pixels)) {
                    eprintln!("{path}: {error}");
                }
                written.store(true, Ordering::Release);
            });
        }
        ui.ctx().request_repaint();
    }
}

/// A top-down 32-bit BMP, which needs no image library.
fn bmp(width: usize, height: usize, pixels: &[egui::Color32]) -> Vec<u8> {
    let size = 54 + pixels.len() * 4;
    let le32 = |value: usize| u32::try_from(value).unwrap_or(u32::MAX).to_le_bytes();
    let mut out = Vec::with_capacity(size);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&le32(size));
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&le32(54));
    out.extend_from_slice(&le32(40));
    out.extend_from_slice(&le32(width));
    out.extend_from_slice(&(-i32::try_from(height).unwrap_or(i32::MAX)).to_le_bytes());
    out.extend_from_slice(&[1, 0, 32, 0]);
    out.extend_from_slice(&[0; 24]);
    for pixel in pixels {
        let [r, g, b, a] = pixel.to_array();
        out.extend_from_slice(&[b, g, r, a]);
    }
    out
}

impl Default for Gallery {
    fn default() -> Self {
        Self {
            fixtures: (0..ENTRIES.len())
                .map(|_| std::array::from_fn(|_| Fixture::sample()))
                .collect(),
            entry_heights: vec![PreviewHeight::default(); ENTRIES.len()],
            audio_height: PreviewHeight::default(),
            patterns_height: PreviewHeight::default(),
            palette: 0,
            density: Density::Compact,
            audio: audio::Preview::default(),
            shell: advanced::Shell::default(),
            patterns: advanced::Catalog::default(),
            files: files::Files::default(),
            shot: Shot::from_env(),
        }
    }
}

impl eframe::App for Gallery {
    fn ui(&mut self, ui: &mut Ui, _: &mut eframe::Frame) {
        self.canvas(ui);
        if let Some(shot) = &mut self.shot {
            shot.take(ui);
        }
    }
}

impl Gallery {
    fn canvas(&mut self, ui: &mut Ui) {
        let tokens = UiTokens::resolve(ui.ctx());
        ui.visuals_mut().panel_fill = tokens.theme.surface;
        tokens.scope(ui, None, |ui, tokens| {
            let spacing = tokens.density.spacing();
            egui::Panel::top("gallery-header")
                .frame(
                    Frame::new()
                        .fill(tokens.theme.bg_surface())
                        .inner_margin(egui::Margin::same(8)),
                )
                .show(ui, |ui| self.shell.header(ui, tokens, &mut self.audio));
            egui::Panel::bottom("gallery-footer")
                .frame(
                    Frame::new()
                        .fill(tokens.theme.bg_surface())
                        .inner_margin(egui::Margin::same(7)),
                )
                .show(ui, |ui| self.shell.footer(ui, tokens));
            self.shell.sidebar(ui, tokens);
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    Frame::new()
                        .fill(tokens.theme.surface)
                        .inner_margin(egui::Margin::same((2.0 * spacing.md) as i8))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(RichText::new("Component gallery").size(27.0).strong());
                            self.shell.section(ui, 0, "Appearance");
                            self.appearance(ui, tokens);
                            Divider::Section.show(ui, tokens);

                            self.shell.section(ui, 1, "Core controls");
                            card_grid(ui, 2, 300.0, 2, |ui, index| self.entry(ui, tokens, index));
                            ui.add_space(spacing.md);
                            self.shell.section(ui, 2, "Audio tools");
                            if !self.audio_height.show(ui, tokens, |ui| {
                                components::card(ui, tokens, |ui| self.audio.show(ui, tokens));
                            }) {
                                self.audio.suspend_preview();
                            }
                            ui.add_space(spacing.md);
                            self.shell.section(ui, 3, "Files and previews");
                            components::card(ui, tokens, |ui| self.files.show(ui, tokens));
                            self.shell.section(ui, 4, "Interface patterns");
                            self.patterns_height.show(ui, tokens, |ui| {
                                self.patterns.show(ui, tokens);
                                components::card(ui, tokens, |ui| {
                                    self.patterns.selections(ui, tokens)
                                });
                            });
                            if let Some(seconds) = self.patterns.take_timestamp() {
                                self.files.seek_to(seconds);
                                self.shell.jump_to_files();
                                ui.ctx().request_repaint();
                            }
                            card_grid(ui, ENTRIES.len() - 2, 300.0, 2, |ui, index| {
                                self.entry(ui, tokens, index + 2);
                            });
                        });
                });
            self.patterns.toasts(ui, tokens);
        });
    }

    fn entry(&mut self, ui: &mut Ui, tokens: UiTokens, index: usize) {
        let entry = ENTRIES[index];
        self.entry_heights[index].show(ui, tokens, |ui| {
            components::card(ui, tokens, |ui| {
                ui.set_min_width(ui.available_width());
                ui.heading(entry.title);
                let width = ui.available_width();
                let [normal, alternate] = &mut self.fixtures[index];
                catalog::preview(ui, tokens, entry, State::Normal, width, normal);
                if let Some((label, state)) = entry.alternate() {
                    ui.add_space(tokens.density.spacing().sm);
                    ui.weak(label);
                    catalog::preview(ui, tokens, entry, state, width, alternate);
                }
            });
        });
    }

    fn appearance(&mut self, ui: &mut Ui, tokens: UiTokens) {
        let previous = (self.palette, self.density);
        let label = ui.strong("Density");
        orange_ui_controls::pill_segmented(
            ui,
            tokens,
            &label,
            &mut self.density,
            &[(Density::Compact, "Compact"), (Density::Comfy, "Comfy")],
        );
        ui.add_space(tokens.density.spacing().sm);
        ui.strong("Theme palettes");
        card_grid(
            ui,
            PALETTES.len() + 1,
            108.0,
            PALETTES.len() + 1,
            |ui, index| {
                let (label, palette) = index
                    .checked_sub(1)
                    .map_or(("System", None), |i| (PALETTES[i].0, Some(PALETTES[i].1)));
                if theme_tile(ui, tokens, label, palette, index == self.palette).clicked() {
                    self.palette = index;
                }
            },
        );
        if previous != (self.palette, self.density) {
            let mode = self
                .palette
                .checked_sub(1)
                .map_or(ThemeMode::System, |i| ThemeMode::Custom(PALETTES[i].1));
            UiTokens::install(ui.ctx(), mode, self.density);
        }
    }
}

fn theme_tile(
    ui: &mut Ui,
    tokens: UiTokens,
    label: &str,
    palette: Option<Theme>,
    selected: bool,
) -> egui::Response {
    use egui::{Rect, Role, Sense, Stroke, StrokeKind, WidgetInfo, pos2, vec2};
    let theme = tokens.theme;
    let height = tokens.density.row_height() + 65.0;
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    response.widget_info(|| {
        WidgetInfo::selected(Role::RadioButton, response.enabled(), selected, label)
    });
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let painter = ui.painter_at(rect);
    let border = if response.has_focus() || selected {
        theme.focus_ring
    } else if response.hovered() {
        theme.border_strong
    } else {
        theme.border
    };
    painter.rect(
        rect,
        theme.radius,
        theme.surface_elevated,
        border,
        StrokeKind::Inside,
    );
    let sample = Rect::from_min_max(
        rect.min + vec2(5.0, 5.0),
        pos2(rect.right() - 5.0, rect.top() + 58.0),
    );
    let light = orange_ui_controls::Theme::from_palette(Theme::LIGHT);
    let dark = orange_ui_controls::Theme::from_palette(Theme::DARK);
    let (first, split) = palette.map_or((light, Some(dark)), |theme| {
        (orange_ui_controls::Theme::from_palette(theme), None)
    });
    orange_ui_controls::paint_theme_preview(&painter, sample, first, split);
    let available = rect.width() - 10.0;
    let font = egui::TextStyle::Body.resolve(ui.style());
    let galley = painter.layout(label.to_owned(), font, theme.text, available);
    painter.galley(
        pos2(
            rect.center().x - galley.size().x / 2.0,
            rect.top() + 59.0 + (height - 59.0 - galley.size().y) / 2.0,
        ),
        galley,
        theme.text,
    );
    if selected {
        let y = rect.bottom() - 3.0;
        painter.hline(
            rect.center().x - 6.0..=rect.center().x + 6.0,
            y,
            Stroke::new(1.5, theme.primary),
        );
    }
    response.on_hover_text("Preview of the canvas, panels, slider and buttons.")
}

fn card_grid(
    ui: &mut Ui,
    count: usize,
    min_width: f32,
    max_columns: usize,
    mut show: impl FnMut(&mut Ui, usize),
) {
    let item_spacing = ui.spacing().item_spacing;
    let gap = item_spacing.x * 3.0;
    let width = ui.available_width();
    let columns = (1..=max_columns)
        .rev()
        .find(|&n| n as f32 * (min_width + gap) - gap <= width)
        .unwrap_or(1);
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(gap, gap);
        for first in (0..count).step_by(columns) {
            ui.push_id(first, |ui| {
                ui.columns(columns, |cells| {
                    for (offset, cell) in cells.iter_mut().take(count - first).enumerate() {
                        cell.spacing_mut().item_spacing = item_spacing;
                        cell.visuals_mut().indent_has_left_vline = false;
                        cell.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                            ui.set_width(ui.available_width());
                            show(ui, first + offset);
                        });
                    }
                });
            });
        }
    });
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(
                std::env::var("ORANGE_GALLERY_SIZE")
                    .ok()
                    .and_then(|size| {
                        let (width, height) = size.split_once('x')?;
                        Some([width.parse().ok()?, height.parse().ok()?])
                    })
                    .unwrap_or([920.0, 760.0]),
            )
            .with_min_inner_size([240.0, 320.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Orange UI gallery",
        options,
        Box::new(|creation| {
            UiTokens::install(&creation.egui_ctx, ThemeMode::System, Density::Compact);
            #[cfg(feature = "icons")]
            orange_ui_controls::icons::install(&creation.egui_ctx);
            Ok(Box::<Gallery>::default())
        }),
    )
}
