//! Node catalogue projection and its presentation cache. The search itself is the
//! host's ([`Host::search_nodes`]), shared by this list and the quick-add box.
use crate::i18n::t;
use orange_host::{Draft, Host, NodeView};
use std::collections::HashMap;

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
    preview_height: Option<f32>,
    highlighted: bool,
    paint_preview: impl FnOnce(&mut egui::Ui, egui::Rect, f32),
) -> egui::Response {
    let width = ui.available_width();
    let zoom = ((width - 32.0) / crate::canvas::NODE_WIDTH).clamp(0.1, 0.95);
    let card_size =
        preview_height.map(|height| egui::vec2(crate::canvas::NODE_WIDTH, height) * zoom);
    let height = card_size.map_or(28.0, |size| size.y + 16.0);
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click_and_drag());
    let response = if detail.is_empty() {
        response
    } else {
        response.on_hover_text(detail)
    };
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
    } else if card_size.is_none() {
        ui.painter()
            .rect_filled(rect.shrink(1.0), 5.0, ui.visuals().faint_bg_color);
        ui.painter().rect_stroke(
            rect.shrink(1.0),
            5.0,
            ui.visuals().widgets.noninteractive.bg_stroke,
            egui::StrokeKind::Inside,
        );
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
    if card_size.is_none() {
        draw(egui::RichText::new(name).strong().into(), rect.min.y + 5.0);
    }
    if let Some(size) = card_size {
        let card = egui::Rect::from_min_size(
            egui::pos2(rect.center().x - size.x / 2.0, rect.min.y + 8.0),
            size,
        );
        paint_preview(ui, card, zoom);
    }
    response
}

/// A kind can be searchable even when no declaration can configure it yet.
pub(crate) fn missing_preview(ui: &egui::Ui, rect: egui::Rect, name: &str) {
    let visuals = ui.visuals();
    ui.painter().rect_filled(rect, 6.0, visuals.window_fill);
    let label = egui::pos2(rect.left() + 10.0, rect.center().y - 9.0);
    let painter = ui.painter().with_clip_rect(rect.shrink(4.0));
    let _ = painter.text(
        label,
        egui::Align2::LEFT_CENTER,
        name,
        egui::FontId::proportional(14.0),
        visuals.text_color(),
    );
    let _ = painter.text(
        label + egui::vec2(0.0, 20.0),
        egui::Align2::LEFT_CENTER,
        t("configuration-required"),
        egui::FontId::proportional(12.0),
        visuals.weak_text_color(),
    );
}

/// Illustrative ports for definition kinds whose actual contracts need a declaration.
/// These cards are only used by search and drag previews.
fn reference_card(host: &Host, definition: &str) -> Option<NodeView> {
    let ports: &[(&str, bool, &str, &str)] = match definition {
        "core/get-variable" => &[("value", false, t("port-value"), "$T")],
        "org.orange.collections/make-enum" => &[
            ("payload", true, t("port-payload-optional"), "$T"),
            ("result", false, t("port-enum"), "$Enum"),
        ],
        "org.orange.collections/enum-payload" => &[
            ("value", true, t("port-enum"), "$Enum"),
            ("result", false, t("port-payload"), "$T"),
        ],
        _ => return None,
    };
    let label = host
        .nodes()
        .find(|node| node.definition.definition.id.as_str() == definition)?
        .label()
        .into_owned();
    Some(NodeView {
        id: "00000000-0000-4000-8000-000000000000".parse().ok()?,
        name: label.clone(),
        kind: label,
        definition: definition.to_owned(),
        detail: None,
        entry: false,
        ports: ports
            .iter()
            .map(|&(id, input, label, ty)| orange_host::PortView {
                id: id.to_owned(),
                input,
                exec: false,
                ty: Some(ty.to_owned()),
                value: None,
                connected: false,
                label: Some(label.to_owned()),
            })
            .collect(),
        position: None,
        settings: serde_json::Value::Null,
        schema: None,
        references: Vec::new(),
        types: Vec::new(),
        variadic: Vec::new(),
        paths: Vec::new(),
    })
}

/// Own only the currently displayed result, not old searches or provider snapshots.
#[derive(Debug, Default)]
pub(crate) struct PaletteCache {
    generation: Option<u64>,
    search: String,
    language: String,
    groups: Vec<(String, Vec<PaletteItem>)>,
}

pub(crate) struct PaletteResults<'a> {
    pub groups: &'a [(String, Vec<PaletteItem>)],
}

impl PaletteCache {
    pub fn get(&mut self, host: &Host, search: &str, language: &str) -> PaletteResults<'_> {
        let generation = host.registry().generation();
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
        }
    }
}

/// Builds only cards whose rows are visible. Registry changes and document edits
/// invalidate configured cards; search text and language do not retain old result lists.
#[derive(Debug, Default)]
pub(crate) struct PreviewCache {
    generation: Option<u64>,
    rows: HashMap<String, usize>,
    cards: HashMap<String, Option<NodeView>>,
}

impl PreviewCache {
    pub fn height(&mut self, host: &Host, definition: &str) -> f32 {
        if let Some(Some(card)) = self.cards.get(definition) {
            let size = crate::canvas::node_size(card);
            return size.y * crate::canvas::NODE_WIDTH / size.x;
        }
        crate::canvas::card_height(definition, self.rows(host, definition))
    }
    pub fn invalidate(&mut self) {
        self.cards.clear();
        self.generation = None;
    }

    pub fn rows(&mut self, host: &Host, definition: &str) -> usize {
        if self.generation != Some(host.registry().generation()) {
            self.rows.clear();
            self.cards.clear();
            for item in host.nodes() {
                let (mut inputs, mut outputs) = (0, 0);
                for (_, input) in item.declared_port_names() {
                    if input {
                        inputs += 1;
                    } else {
                        outputs += 1;
                    }
                }
                self.rows.insert(
                    item.definition.definition.id.to_string(),
                    inputs.max(outputs).max(1)
                        + usize::from(
                            inputs >= outputs
                                && item.presentation.is_some_and(|presentation| {
                                    presentation.connection_adapter.is_some()
                                }),
                        ),
                );
            }
            self.generation = Some(host.registry().generation());
        }
        self.rows.get(definition).copied().unwrap_or(1)
    }

    pub fn card(
        &mut self,
        host: &Host,
        draft: Option<&Draft>,
        definition: &str,
    ) -> Option<&NodeView> {
        let _ = self.rows(host, definition);
        if !self.cards.contains_key(definition) {
            let configured = (|| {
                if let Some(draft) = draft {
                    host.placement_preview(draft, definition).ok()
                } else {
                    let text = orange_host::new_document().ok()?;
                    let draft = host.draft(&text).ok()?;
                    host.placement_preview(&draft, definition).ok()
                }
            })();
            let card = match configured {
                Some(card) if !card.ports.is_empty() => Some(card),
                other => reference_card(host, definition).or(other),
            };
            if let Some(card) = &card {
                let inputs = card.ports.iter().filter(|port| port.input).count();
                self.rows.insert(
                    definition.to_owned(),
                    inputs.max(card.ports.len() - inputs).max(1)
                        + usize::from(crate::canvas::connection_footer(card) > 0.0),
                );
            }
            self.cards.insert(definition.to_owned(), card);
        }
        self.cards.get(definition).and_then(Option::as_ref)
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
    fn declaration_dependent_kinds_have_illustrative_ports() {
        let (host, _events) = Host::new(HostOptions::default()).unwrap();
        let mut cache = PreviewCache::default();
        for definition in [
            "core/get-variable",
            "org.orange.collections/make-enum",
            "org.orange.collections/enum-payload",
        ] {
            let card = cache
                .card(&host, None, definition)
                .unwrap_or_else(|| panic!("missing card for {definition}"));
            assert!(card.ports.iter().any(|port| !port.input));
        }
    }

    #[test]
    fn previews_draw_the_canvas_card_and_skip_offscreen_and_names_rows() {
        let (host, _events) = Host::new(HostOptions::default()).unwrap();
        let mut cache = PreviewCache::default();
        let ctx = egui::Context::default();
        let mut preview_height = 0.0;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(280.0, 240.0),
                )),
                ..Default::default()
            },
            |ui| {
                let height = cache.height(&host, "core/and");
                preview_height = result_row(
                    ui,
                    "AND",
                    "Both inputs",
                    Some(height),
                    false,
                    |ui, rect, zoom| {
                        let card = cache.card(&host, None, "core/and").unwrap();
                        crate::canvas::draw_preview(ui, card, rect, zoom);
                    },
                )
                .rect
                .height();
                let compact = result_row(ui, "AND", "", None, false, |_, _, _| {
                    panic!("Names does not build a card")
                });
                assert!(compact.rect.height() < preview_height / 2.0);
                ui.add_space(500.0);
                let _ = result_row(ui, "Offscreen", "", Some(100.0), false, |_, _, _| {
                    panic!("Offscreen rows do not build cards")
                });
            },
        );
        output.textures_delta.clear();
        let labels: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                _ => None,
            })
            .collect();
        for label in ["And", "a", "b", "result"] {
            assert!(
                output.shapes.iter().any(|shape| matches!(
                    &shape.shape, egui::Shape::Text(text) if text.galley.text() == label
                )),
                "missing card label {label}: {labels:?}"
            );
        }
        assert!(
            output
                .shapes
                .iter()
                .filter(|shape| matches!(shape.shape, egui::Shape::Circle(_)))
                .count()
                >= 3,
            "both input ports and the output port are drawn"
        );
        assert_eq!(cache.cards.len(), 1);
        let id = cache.card(&host, None, "core/and").unwrap().id;
        assert_eq!(cache.card(&host, None, "core/and").unwrap().id, id);
        cache.invalidate();
        assert!(cache.cards.is_empty());
        assert_ne!(cache.card(&host, None, "core/and").unwrap().id, id);
        output.drop_without_applying_deltas();
    }

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
                    Some(112.0),
                    false,
                    |_, _, _| {},
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
