//! A reusable sRGB picker with straight alpha. The caller owns the committed color.
use egui::{
    Color32, Id, Popup, PopupCloseBehavior, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui,
    Vec2,
    ecolor::{Hsva, HsvaGamma, linear_u8_from_linear_f32},
    epaint::{Mesh, Vertex, WHITE_UV},
    pos2, vec2,
};

/// Labels supplied by the calling application's locale.
#[derive(Debug, Clone, Copy)]
pub struct Labels<'a> {
    pub channels: [&'a str; 4],
    pub hue: &'a str,
    pub saturation_value: &'a str,
    pub recent: &'a str,
    pub value: &'a str,
    pub invalid_value: &'a str,
    pub apply: &'a str,
    pub cancel: &'a str,
}

#[derive(Clone)]
struct Draft {
    color: [u8; 4],
    hsv: HsvaGamma,
    format: Format,
    text: String,
    invalid: bool,
}

impl Draft {
    fn new(color: [u8; 4], show_alpha: bool) -> Self {
        let mut draft = Self {
            color,
            hsv: opaque_hsv(color),
            format: Format::Hex,
            text: String::new(),
            invalid: false,
        };
        draft.refresh_text(show_alpha);
        draft
    }

    fn sample_rgb(&mut self) {
        let [r, g, b, _] = Hsva::from(self.hsv).to_srgba_unmultiplied();
        self.color[..3].copy_from_slice(&[r, g, b]);
    }

    fn set_color(&mut self, color: [u8; 4]) {
        self.color = color;
        self.hsv = opaque_hsv(color);
    }

    fn refresh_text(&mut self, show_alpha: bool) {
        self.text = self.format.encode(self.color, show_alpha);
        self.invalid = false;
    }
}

fn opaque_hsv([r, g, b, _]: [u8; 4]) -> HsvaGamma {
    Hsva::from_srgba_unmultiplied([r, g, b, 255]).into()
}

/// Draw the swatch using the same transparency pattern as the picker.
pub fn swatch(ui: &Ui, rect: Rect, [r, g, b, a]: [u8; 4]) {
    let theme = super::Theme::of(ui);
    if a != 255 {
        checkers(ui, rect, theme);
    }
    ui.painter()
        .rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(r, g, b, a));
    ui.painter().rect_stroke(
        rect,
        0.0,
        Stroke::new(1.0, theme.border),
        StrokeKind::Inside,
    );
}

fn checkers(ui: &Ui, rect: Rect, theme: super::Theme) {
    let size = 3.0;
    let dark = theme.inset.lerp_to_gamma(theme.text, 0.16);
    let light = theme.inset.lerp_to_gamma(theme.text, 0.38);
    let mut mesh = Mesh::default();
    mesh.add_colored_rect(rect, light);
    let mut y = rect.top();
    let mut offset = 0.0;
    while y < rect.bottom() {
        let mut x = rect.left() + offset;
        while x < rect.right() {
            let square = Rect::from_min_size(pos2(x, y), Vec2::splat(size)).intersect(rect);
            mesh.add_colored_rect(square, dark);
            x += size * 2.0;
        }
        y += size;
        offset = size - offset;
    }
    ui.painter().add(mesh);
}

/// Attach a picker to any clickable response. Returns a color when Apply is clicked.
/// Recent colors are shared by all pickers in this egui context for its lifetime.
pub fn popup(
    parent: &Ui,
    anchor: &Response,
    current: [u8; 4],
    show_alpha: bool,
    labels: Labels<'_>,
) -> Option<[u8; 4]> {
    let id = anchor.id.with("orange-color-picker");
    if !anchor.clicked() && !Popup::is_id_open(&anchor.ctx, id) {
        return None;
    }
    let current = if show_alpha {
        current
    } else {
        [current[0], current[1], current[2], 255]
    };
    if anchor.clicked() {
        anchor
            .ctx
            .data_mut(|data| data.insert_temp(id, Some(Draft::new(current, show_alpha))));
    }
    let mut applied = None;
    Popup::menu(anchor)
        .id(id)
        .frame(super::popup_frame(parent))
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            applied = inline(ui, id, current, show_alpha, labels);
        });
    applied
}

/// Discards uncommitted changes in an inline color picker.
pub fn reset(data: &mut egui::util::IdTypeMap, id: Id) {
    data.remove::<Option<Draft>>(id);
}

/// Draws a color picker in an existing menu. The menu closes on Apply or Cancel.
pub fn inline(
    ui: &mut Ui,
    id: Id,
    current: [u8; 4],
    show_alpha: bool,
    labels: Labels<'_>,
) -> Option<[u8; 4]> {
    let recent_id = Id::unique("orange-recent-colors");
    let theme = super::Theme::of(ui);
    let mut applied = None;
    let mut draft = ui
        .data_mut(|data| data.get_temp_mut_or_default::<Option<Draft>>(id).take())
        .unwrap_or_else(|| Draft::new(current, show_alpha));
    let width = (ui.ctx().content_rect().width() - 28.0).clamp(176.0, 280.0);
    let track = 18.0;
    let gap = super::SPACE_MEDIUM;
    let bars = if show_alpha { 2.0 } else { 1.0 };
    ui.set_width(width);
    ui.spacing_mut().item_spacing.x = gap;
    let old = draft.color;
    let sizes = vec2(width - bars * (track + gap), 150.0);
    ui.add_space(super::SPACE_SMALL);
    ui.horizontal(|ui| {
        let area = target(ui, sizes, labels.saturation_value);
        let hue = target(ui, vec2(track, sizes.y), labels.hue);
        let alpha = show_alpha.then(|| target(ui, vec2(track, sizes.y), labels.channels[3]));
        if let Some(at) = pointer(&area) {
            draft.hsv.s = at.x;
            draft.hsv.v = 1.0 - at.y;
            draft.sample_rgb();
        }
        if let Some(at) = pointer(&hue) {
            draft.hsv.h = at.y;
            draft.sample_rgb();
        }
        if let Some(at) = alpha.as_ref().and_then(pointer) {
            draft.color[3] = linear_u8_from_linear_f32(1.0 - at.y);
        }
        let h = draft.hsv.h;
        gradient(ui, area.rect, 8, |x, y| {
            HsvaGamma {
                h,
                s: x,
                v: 1.0 - y,
                a: 1.0,
            }
            .into()
        });
        gradient(ui, hue.rect, 24, |_, y| {
            HsvaGamma {
                h: y,
                s: 1.0,
                v: 1.0,
                a: 1.0,
            }
            .into()
        });
        let marker = area
            .rect
            .shrink(4.5)
            .clamp(area.rect.lerp_inside(vec2(draft.hsv.s, 1.0 - draft.hsv.v)));
        ui.painter()
            .circle_stroke(marker, 4.5, Stroke::new(2.5, Color32::BLACK));
        ui.painter()
            .circle_stroke(marker, 4.5, Stroke::new(1.0, Color32::WHITE));
        bar_mark(ui, hue.rect, draft.hsv.h);
        if let Some(alpha) = alpha {
            checkers(ui, alpha.rect, theme);
            let [r, g, b, _] = draft.color;
            gradient(ui, alpha.rect, 24, |_, y| {
                Color32::from_rgba_unmultiplied(r, g, b, linear_u8_from_linear_f32(1.0 - y))
            });
            bar_mark(ui, alpha.rect, 1.0 - f32::from(draft.color[3]) / 255.0);
        }
    });
    ui.horizontal_top(|ui| {
        let mut edited = false;
        let count = if show_alpha { 4 } else { 3 };
        let columns = if show_alpha { 4.0 } else { 3.0 };
        let cell_width = (width + gap) / columns - gap;
        for (&name, channel) in labels.channels.iter().zip(&mut draft.color).take(count) {
            edited |= ui
                .allocate_ui_with_layout(
                    vec2(
                        cell_width,
                        16.0 + ui.spacing().item_spacing.y + ui.spacing().interact_size.y,
                    ),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        let label = ui.add_sized(
                            [cell_width, 16.0],
                            egui::Label::new(egui::RichText::new(name).weak()).truncate(),
                        );
                        ui.add_sized(
                            [cell_width, ui.spacing().interact_size.y],
                            egui::DragValue::new(channel).range(0..=255),
                        )
                        .labelled_by(label.id)
                        .changed()
                    },
                )
                .inner;
        }
        if edited {
            draft.hsv = opaque_hsv(draft.color);
        }
    });
    if draft.color != old {
        draft.refresh_text(show_alpha);
    }
    ui.separator();
    let format = draft.format;
    let label = ui
        .horizontal_wrapped(|ui| {
            let label = ui.strong(labels.value);

            super::segmented(
                ui,
                super::Surface::Dialog,
                &label,
                &mut draft.format,
                &[
                    (Format::Hex, "Hex"),
                    (Format::Rgb, if show_alpha { "RGBA" } else { "RGB" }),
                ],
            );
            label
        })
        .inner;
    if draft.format != format {
        draft.refresh_text(show_alpha);
    }
    let field = super::input_frame(ui, super::Surface::Dialog, super::CONTROL_RADIUS);
    let response = ui
        .add(
            egui::TextEdit::singleline(&mut draft.text)
                .id(id.with("value"))
                .frame(field)
                .align(egui::Align2::LEFT_CENTER)
                .min_size(vec2(0.0, super::CONTROL_HEIGHT))
                .desired_width(f32::INFINITY)
                .font(egui::TextStyle::Monospace),
        )
        .labelled_by(label.id);
    super::paint_input_focus(ui, &response, super::CONTROL_RADIUS);
    if response.changed() {
        let color = parse(&draft.text);
        draft.invalid = color.is_none();
        if let Some(mut color) = color {
            if !show_alpha {
                color[3] = 255;
            }
            draft.set_color(color);
        }
    }
    if draft.invalid {
        ui.colored_label(theme.danger, labels.invalid_value);
    }
    let recent = ui
        .data_mut(|data| data.get_temp::<Vec<[u8; 4]>>(recent_id))
        .unwrap_or_default();
    if !recent.is_empty() {
        ui.label(labels.recent);
        ui.horizontal_wrapped(|ui| {
            for color in recent {
                let color = if show_alpha {
                    color
                } else {
                    [color[0], color[1], color[2], 255]
                };
                let response = ui
                    .add(egui::Button::new("").min_size(vec2(18.0, 18.0)))
                    .on_hover_text(hex(color, show_alpha));
                swatch(ui, response.rect.shrink(2.0), color);
                if response.clicked() {
                    draft.set_color(color);
                    draft.refresh_text(show_alpha);
                }
            }
        });
    }
    ui.separator();
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), ui.spacing().interact_size.y),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            if super::button(
                ui,
                super::Surface::Dialog,
                super::Action::Primary,
                !draft.invalid,
                egui::Button::new(labels.apply).min_size(vec2(76.0, 0.0)),
            )
            .clicked()
            {
                applied = Some(draft.color);
                ui.data_mut(|data| {
                    let mut colors = data.get_temp::<Vec<[u8; 4]>>(recent_id).unwrap_or_default();
                    colors.retain(|color| *color != draft.color);
                    colors.insert(0, draft.color);
                    colors.truncate(8);
                    data.insert_temp(recent_id, colors);
                });
                ui.close();
            }
            if super::button(
                ui,
                super::Surface::Dialog,
                super::Action::Outline,
                true,
                egui::Button::new(labels.cancel).min_size(vec2(76.0, 0.0)),
            )
            .clicked()
            {
                ui.close();
            }
        },
    );
    ui.data_mut(|data| *data.get_temp_mut_or_default::<Option<Draft>>(id) = Some(draft));
    applied
}

fn hex([r, g, b, a]: [u8; 4], alpha: bool) -> String {
    if alpha {
        format!("#{r:02X}{g:02X}{b:02X}{a:02X}")
    } else {
        format!("#{r:02X}{g:02X}{b:02X}")
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Format {
    Hex,
    Rgb,
}

impl Format {
    fn encode(self, color: [u8; 4], alpha: bool) -> String {
        let [r, g, b, a] = color;
        match (self, alpha) {
            (Self::Hex, _) => hex(color, alpha),
            (Self::Rgb, false) => format!("rgb({r}, {g}, {b})"),
            (Self::Rgb, true) => {
                let alpha = format!("{:.3}", f32::from(a) / 255.0);
                let alpha = alpha.trim_end_matches('0').trim_end_matches('.');
                format!("rgba({r}, {g}, {b}, {alpha})")
            }
        }
    }
}

fn parse(text: &str) -> Option<[u8; 4]> {
    let text = text.trim();
    if let Some((name, body)) = text.split_once('(') {
        if !name.eq_ignore_ascii_case("rgb") && !name.eq_ignore_ascii_case("rgba") {
            return None;
        }
        return parse_rgb(body.strip_suffix(')')?);
    }
    let text = text.strip_prefix('#').unwrap_or(text);
    if !matches!(text.len(), 3 | 4 | 6 | 8) || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let width = if text.len() < 6 { 1 } else { 2 };
    let mut color = [255; 4];
    for (index, channel) in color.iter_mut().take(text.len() / width).enumerate() {
        let at = index * width;
        let byte = u8::from_str_radix(&text[at..at + width], 16).ok()?;
        *channel = if width == 1 { byte * 17 } else { byte };
    }
    Some(color)
}

fn parse_rgb(body: &str) -> Option<[u8; 4]> {
    let (rgb, alpha) = if body.contains(',') {
        let mut parts = body.split(',').map(str::trim);
        let rgb = [parts.next()?, parts.next()?, parts.next()?];
        let alpha = parts.next();
        if parts.next().is_some() {
            return None;
        }
        (rgb, alpha)
    } else {
        let (rgb, alpha) = body
            .split_once('/')
            .map_or((body, None), |(rgb, alpha)| (rgb, Some(alpha.trim())));
        let mut parts = rgb.split_ascii_whitespace();
        let rgb = [parts.next()?, parts.next()?, parts.next()?];
        if parts.next().is_some() {
            return None;
        }
        (rgb, alpha)
    };
    let channel = |text: &str, scale| {
        let (number, scale) = text
            .strip_suffix('%')
            .map_or((text, scale), |number| (number, 100.0));
        let number = number.parse::<f32>().ok()?;
        (0.0..=scale)
            .contains(&number)
            .then(|| linear_u8_from_linear_f32(number / scale))
    };
    let [r, g, b] = rgb.map(|text| channel(text, 255.0));
    Some([
        r?,
        g?,
        b?,
        alpha.map_or(Some(255), |text| channel(text, 1.0))?,
    ])
}

fn target(ui: &mut Ui, size: Vec2, help: &str) -> Response {
    ui.allocate_exact_size(size, Sense::click_and_drag())
        .1
        .on_hover_text(help)
}

fn pointer(response: &Response) -> Option<Vec2> {
    let pos = response.interact_pointer_pos()?;
    let rect = response.rect;
    Some(((pos - rect.min) / rect.size()).clamp(Vec2::ZERO, Vec2::splat(1.0)))
}

fn bar_mark(ui: &Ui, rect: Rect, value: f32) {
    let y = (rect.top() + value * rect.height()).clamp(rect.top() + 1.5, rect.bottom() - 1.5);
    let line = [pos2(rect.left(), y), pos2(rect.right(), y)];
    ui.painter()
        .line_segment(line, Stroke::new(3.0, Color32::BLACK));
    ui.painter()
        .line_segment(line, Stroke::new(1.0, Color32::WHITE));
}

fn gradient(ui: &Ui, rect: Rect, rows: u16, color: impl Fn(f32, f32) -> Color32) {
    let mut mesh = Mesh::default();
    for i in 0..=rows {
        let y = f32::from(i) / f32::from(rows);
        for x in [0.0, 1.0] {
            let pos: Pos2 = rect.lerp_inside(vec2(x, y));
            mesh.vertices.push(Vertex {
                pos,
                uv: WHITE_UV,
                color: color(x, y),
            });
        }
        if i > 0 {
            let top = u32::from(i - 1) * 2;
            mesh.add_triangle(top, top + 1, top + 2);
            mesh.add_triangle(top + 1, top + 3, top + 2);
        }
    }
    ui.painter().add(mesh);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_and_css_text_preserve_exact_eight_bit_alpha() {
        for alpha in 0..=255 {
            let color = [17, 131, 247, alpha];
            for format in [Format::Hex, Format::Rgb] {
                assert_eq!(parse(&format.encode(color, true)), Some(color));
            }
        }
        assert_eq!(parse("#1230"), Some([17, 34, 51, 0]));
        assert_eq!(parse("rgb(100% 0% 0% / 50%)"), Some([255, 0, 0, 128]));
        assert_eq!(parse("rgba(42, 22, 11, 0)"), Some([42, 22, 11, 0]));
        for invalid in [
            "#12",
            "rgba(256, 22, 11, 0)",
            "rgba(42,22,11, NaN)",
            "rgb(1,,2,3)",
        ] {
            assert_eq!(parse(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn changing_alpha_does_not_quantize_rgb_and_transparent_colors_keep_their_hue() {
        let rgb = [24, 167, 233];
        for alpha in [0, 1, 32, 255] {
            let mut draft = Draft::new([rgb[0], rgb[1], rgb[2], alpha], true);
            draft.sample_rgb();
            assert_eq!(draft.color, [rgb[0], rgb[1], rgb[2], alpha]);
        }
    }

    #[test]
    fn the_picker_previews_and_round_trips_clipboard_text_before_apply() {
        let ctx = egui::Context::default();
        let current = [255, 0, 0, 255];
        let labels = Labels {
            channels: ["R", "G", "B", "A"],
            hue: "Hue",
            saturation_value: "Saturation and brightness",
            recent: "Recent",
            value: "Color value",
            invalid_value: "Invalid color",
            apply: "Use color",
            cancel: "Cancel",
        };
        let frame = |events| {
            let mut anchor = Pos2::ZERO;
            let mut picked = None;
            let output = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    let button = ui.button("Picker");
                    anchor = button.rect.center();
                    picked = popup(ui, &button, current, true, labels);
                },
            );
            (anchor, picked, output)
        };
        let click = |pos| {
            let (_, picked, output) = frame(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    pressed: true,
                    button: egui::PointerButton::Primary,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
            assert!(picked.is_none());
            output.drop_without_applying_deltas();
            frame(vec![egui::Event::PointerButton {
                pos,
                pressed: false,
                button: egui::PointerButton::Primary,
                modifiers: egui::Modifiers::NONE,
            }])
        };
        let (anchor, _, output) = frame(vec![]);
        output.drop_without_applying_deltas();
        let (_, _, output) = click(anchor);
        output.drop_without_applying_deltas();
        let (_, _, output) = frame(vec![]);
        let square = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Mesh(mesh) if mesh.vertices.len() == 18 => Some(Rect::from_min_max(
                    mesh.vertices[0].pos,
                    mesh.vertices.last()?.pos,
                )),
                _ => None,
            })
            .unwrap();
        assert!(square.width() > square.height());
        output.drop_without_applying_deltas();
        let (_, picked, output) = click(square.center());
        assert!(picked.is_none());
        let value = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text().starts_with('#') => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                _ => None,
            })
            .unwrap();
        output.drop_without_applying_deltas();
        click(value).2.drop_without_applying_deltas();
        let modifiers = egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        };
        let select_all = || egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        let pasted = "rgba(16, 155, 224, 0)";
        let (_, picked, output) = frame(vec![select_all(), egui::Event::Paste(pasted.to_owned())]);
        assert!(picked.is_none());
        output.drop_without_applying_deltas();
        let (_, picked, output) = frame(vec![select_all(), egui::Event::Copy]);
        assert!(picked.is_none());
        assert!(output.platform_output.commands.iter().any(|command| {
            matches!(command, egui::OutputCommand::CopyText(text) if text == pasted)
        }));
        let apply = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == labels.apply => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                _ => None,
            })
            .unwrap();
        output.drop_without_applying_deltas();
        let (_, picked, output) = click(apply);
        output.drop_without_applying_deltas();
        let picked = picked.unwrap();
        assert_eq!(picked, [16, 155, 224, 0]);
        assert_eq!(
            ctx.data_mut(|data| data.get_temp::<Vec<[u8; 4]>>(Id::unique("orange-recent-colors"))),
            Some(vec![picked])
        );
    }
}
