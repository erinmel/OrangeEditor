//! Node catalogue projection and its presentation cache. The search itself is the
//! host's ([`Host::search_nodes`]), shared by this list and the quick-add box.
use orange_host::Host;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum PaletteMode {
    Names,
    #[default]
    Previews,
}

#[derive(Clone)]
pub(crate) struct PaletteDrag {
    pub item: PaletteItem,
    pub card: Option<orange_host::NodeView>,
}

/// Full-width target shared by the docked palette and the canvas picker.
pub(crate) fn result_row(
    ui: &mut egui::Ui,
    name: &str,
    detail: &str,
    ports: &str,
    highlighted: bool,
    sense: egui::Sense,
) -> egui::Response {
    let height = 26.0
        + if detail.is_empty() { 0.0 } else { 16.0 }
        + if ports.is_empty() { 0.0 } else { 16.0 };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), sense);
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::accesskit::Role::Button,
            ui.is_enabled(),
            highlighted,
            name,
        )
    });
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let visuals = ui.style().interact_selectable(&response, highlighted);
    if highlighted || response.hovered() || response.has_focus() {
        ui.painter().rect_filled(rect, 5.0, visuals.weak_bg_fill);
    }
    if response.has_focus() || highlighted {
        ui.painter()
            .rect_stroke(rect, 5.0, visuals.fg_stroke, egui::StrokeKind::Inside);
    }
    let width = (rect.width() - 16.0).max(0.0);
    let draw = |text: egui::WidgetText, y| {
        let galley = text.into_galley(
            ui,
            Some(egui::TextWrapMode::Truncate),
            width,
            egui::TextStyle::Body,
        );
        ui.painter().galley(
            egui::pos2(rect.min.x + 8.0, y),
            galley,
            visuals.text_color(),
        );
    };
    draw(egui::RichText::new(name).strong().into(), rect.min.y + 5.0);
    if !detail.is_empty() {
        draw(
            egui::RichText::new(detail)
                .small()
                .color(ui.visuals().weak_text_color())
                .into(),
            rect.min.y + 23.0,
        );
    }
    if !ports.is_empty() {
        let y = rect.min.y + if detail.is_empty() { 23.0 } else { 39.0 };
        draw(egui::RichText::new(ports).small().monospace().into(), y);
    }
    response
}

/// Own only the currently displayed result, not old searches or provider snapshots.
#[derive(Debug, Default)]
pub(crate) struct PaletteCache {
    generation: Option<u64>,
    search: String,
    language: String,
    groups: Vec<(String, Vec<PaletteItem>)>,
    summaries: std::collections::HashMap<String, String>,
}

pub(crate) struct PaletteResults<'a> {
    pub groups: &'a [(String, Vec<PaletteItem>)],
    pub summaries: &'a std::collections::HashMap<String, String>,
}

impl PaletteCache {
    pub fn get(&mut self, host: &Host, search: &str, language: &str) -> PaletteResults<'_> {
        let generation = host.registry().generation();
        if self.generation != Some(generation) {
            self.summaries.clear();
            for item in host.nodes() {
                let mut inputs = Vec::new();
                let mut outputs = Vec::new();
                for (name, input) in item.declared_port_names() {
                    let side = if input { &mut inputs } else { &mut outputs };
                    side.push(crate::port_label(name));
                }
                if !inputs.is_empty() || !outputs.is_empty() {
                    self.summaries.insert(
                        item.definition.definition.id.to_string(),
                        format!("{} → {}", inputs.join(", "), outputs.join(", ")),
                    );
                }
            }
        }
        if self.generation != Some(generation) || self.search != search || self.language != language
        {
            self.groups = palette_in(host, search, language);
            self.search.clear();
            self.search.push_str(search);
            self.language.clear();
            self.language.push_str(language);
            self.generation = Some(generation);
        }
        PaletteResults {
            groups: &self.groups,
            summaries: &self.summaries,
        }
    }
}

/// One node kind a document can use, as the palette lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteItem {
    /// The definition, such as `org.orange.files/write-text`.
    pub id: String,
    pub label: String,
    /// The package that provides it; empty for the language's own nodes.
    pub package: String,
    /// One line on what it does, or empty.
    pub purpose: String,
}

/// [`palette_in`] in English.
pub fn palette(host: &Host, search: &str) -> Vec<(String, Vec<PaletteItem>)> {
    palette_in(host, search, orange_host::SOURCE_LANGUAGE)
}

/// The host's nodes that match `search` in `language` or English, grouped by category.
/// With a search, the best match comes first in its category and the category of the
/// best match first; without one both are in alphabetical order.
pub fn palette_in(host: &Host, search: &str, language: &str) -> Vec<(String, Vec<PaletteItem>)> {
    let mut groups: Vec<(String, Vec<PaletteItem>)> = Vec::new();
    // Found nodes come best first, so a category's place is that of its best node.
    for found in host.search_nodes(search, language) {
        let item = PaletteItem {
            id: found.definition,
            label: found.label,
            package: found.package,
            purpose: found.purpose,
        };
        match groups
            .iter_mut()
            .find(|(category, _)| *category == found.category)
        {
            Some((_, items)) => items.push(item),
            None => groups.push((found.category, vec![item])),
        }
    }
    if search.trim().is_empty() {
        groups.sort_by(|a, b| a.0.cmp(&b.0));
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use orange_host::HostOptions;

    #[test]
    fn the_result_can_be_dragged_from_the_preview_line() {
        let ctx = egui::Context::default();
        let mut rect = egui::Rect::NOTHING;
        let mut dragged = false;
        let mut frame = |events| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(280.0, 220.0),
                )),
                events,
                ..Default::default()
            };
            ctx.run_ui(input, |ui| {
                let row = result_row(
                    ui,
                    "AND",
                    "Both values must be true",
                    "A, B → Result",
                    false,
                    egui::Sense::click_and_drag(),
                );
                rect = row.rect;
                dragged |= row.drag_started();
            })
            .drop_without_applying_deltas();
            rect
        };
        let initial = frame(Vec::new());
        let at = egui::pos2(initial.right() - 14.0, initial.bottom() - 9.0);
        frame(vec![
            egui::Event::PointerMoved(at),
            egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        frame(vec![egui::Event::PointerMoved(at + egui::vec2(0.0, 65.0))]);
        assert!(dragged);
    }

    #[test]
    fn a_search_finds_aliases_concepts_and_spanish_and_puts_the_best_match_first() {
        let (host, _events) = Host::new(HostOptions::default()).unwrap();
        let first = |search: &str, language: &str| {
            palette_in(&host, search, language)[0].1[0].label.clone()
        };
        assert_eq!(first("integer", "en"), "Int");
        assert_eq!(first("boolean", "en"), "Bool");
        assert_eq!(first("wait", "en"), "Wait");
        assert_eq!(first("temporizador", "es"), "On timer");
        assert_eq!(first("entero", "es"), "Int");
        // "trigger" lists the triggers, each with what it is for.
        let triggers = palette_in(&host, "trigger", "en");
        let core = &triggers[0].1;
        assert!(core.iter().any(|item| item.label == "On manual"));
        assert!(core.iter().all(|item| !item.purpose.is_empty()));
        assert!(palette(&host, "unmatched-xyz").is_empty());
    }

    #[test]
    fn cached_palette_tracks_search_language_and_registry_and_keeps_the_current_allocation() {
        let (host, _events) = Host::new(HostOptions::default()).unwrap();
        let mut cache = PaletteCache::default();
        assert_eq!(cache.get(&host, "", "en").groups, palette(&host, ""));
        let allocation = cache.get(&host, "", "en").groups.as_ptr();
        assert_eq!(cache.get(&host, "", "en").groups.as_ptr(), allocation);
        for search in ["MATH add", "unmatched-xyz", ""] {
            assert_eq!(
                cache.get(&host, search, "en").groups,
                palette(&host, search)
            );
        }
        assert_eq!(
            cache.get(&host, "disparador", "es").groups,
            palette_in(&host, "disparador", "es")
        );
        assert!(cache.get(&host, "disparador", "en").groups.is_empty());
        // Registry generations are process-unique, including when one editor is
        // reused with another host. Old provider labels must not survive that switch.
        let (core, _events) = Host::new(HostOptions {
            base_nodes: false,
            ..Default::default()
        })
        .unwrap();
        assert_ne!(core.registry().generation(), host.registry().generation());
        assert_ne!(palette(&core, ""), palette(&host, ""));
        assert_eq!(cache.get(&core, "", "en").groups, palette(&core, ""));
        assert_eq!(cache.get(&host, "", "en").groups, palette(&host, ""));
    }
}
