//! Node catalogue projection and its presentation cache. The search itself is the
//! host's ([`Host::search_nodes`]), shared by this list and the quick-add box.
use crate::i18n::t;
use orange_host::{Draft, Host, NodeView};
use std::collections::HashMap;

pub(crate) const BAND_TRIGGER: &str = "template/band-trigger";

#[derive(Clone)]
pub(crate) struct PaletteDrag {
    pub item: PaletteItem,
    pub card: Option<orange_host::NodeView>,
}

/// One result of a node search, shared by the docked palette and the canvas picker:
/// the node's name on one line. Hovering shows its name, purpose and ID together.
pub(crate) fn result_row(
    ui: &mut egui::Ui,
    name: &str,
    detail: &str,
    id: &str,
    highlighted: bool,
) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, 28.0), egui::Sense::click_and_drag());
    let response = response.on_hover_ui(|ui| {
        ui.set_max_width(280.0);
        ui.strong(name);
        if !detail.is_empty() {
            ui.label(detail);
        }
        ui.label(egui::RichText::new(id).small().weak());
    });
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
    } else {
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
    let galley = egui::WidgetText::from(egui::RichText::new(name).strong()).into_galley(
        ui,
        Some(egui::TextWrapMode::Truncate),
        (rect.width() - 16.0).max(0.0),
        egui::TextStyle::Body,
    );
    ui.painter().galley(
        egui::pos2(rect.min.x + 8.0, rect.center().y - galley.size().y / 2.0),
        galley,
        visuals.text_color(),
    );
    response
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
                default: None,
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

/// Builds only the cards of results being shown. Registry changes and document edits
/// invalidate configured cards; search text and language do not retain old result lists.
#[derive(Debug, Default)]
pub(crate) struct PreviewCache {
    generation: Option<u64>,
    cards: HashMap<String, Option<NodeView>>,
}

impl PreviewCache {
    pub fn invalidate(&mut self) {
        self.cards.clear();
        self.generation = None;
    }

    /// Forgets every card once the registry has changed.
    fn follow(&mut self, host: &Host) {
        if self.generation != Some(host.registry().generation()) {
            self.cards.clear();
            self.generation = Some(host.registry().generation());
        }
    }

    pub fn card(
        &mut self,
        host: &Host,
        draft: Option<&Draft>,
        definition: &str,
    ) -> Option<&NodeView> {
        self.follow(host);
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
    let band = host
        .search_nodes("org.orange.audio/live-band-level", language)
        .into_iter()
        .find(|item| item.definition == "org.orange.audio/live-band-level");
    if let Some(band) = band {
        let catalogs = crate::i18n::catalogs();
        let text = |key, fallback: &str| {
            catalogs
                .message(language, key)
                .map_or_else(|| fallback.to_owned(), std::borrow::Cow::into_owned)
        };
        let label = text("band-trigger", "Band trigger");
        let purpose = text(
            "band-trigger-purpose",
            "Wire an audio source into Band level to trigger at a threshold.",
        );
        let terms = format!("{label} {purpose} Band trigger").to_lowercase();
        if search
            .to_lowercase()
            .split_whitespace()
            .all(|word| terms.contains(word))
        {
            let template = PaletteItem {
                id: BAND_TRIGGER.to_owned(),
                label,
                package: band.package,
                purpose,
            };
            if let Some((_, items)) = groups
                .iter_mut()
                .find(|(category, _)| *category == band.category)
            {
                items.push(template);
            } else {
                groups.push((band.category, vec![template]));
            }
        }
    }
    // Conversions are search results too, in a category of their own.
    let wanted = search.to_lowercase();
    let converts: Vec<PaletteItem> = host
        .conversions()
        .iter()
        .map(|conversion| conversion_item(conversion, language))
        .filter(|item| {
            let terms = format!(
                "{} {} convert conversion{}",
                item.label,
                item.purpose,
                conversion_words(&item.id)
            )
            .to_lowercase();
            wanted.split_whitespace().all(|word| terms.contains(word))
        })
        .collect();
    if !converts.is_empty() {
        groups.push((text_in(language, "conversions", "Conversions"), converts));
    }
    if search.trim().is_empty() {
        groups.sort_by(|a, b| a.0.cmp(&b.0));
    }
    groups
}

/// Other words for the two types a conversion ID names, so "whole number text" finds
/// Int → Text.
fn conversion_words(id: &str) -> String {
    let converter = id
        .trim_start_matches(CONVERSION)
        .split('?')
        .next()
        .unwrap_or_default();
    let mut words = String::new();
    for (types, said) in [
        (&["i32", "i64", "u32", "u64"][..], "whole number integer"),
        (&["f32", "f64"][..], "decimal float number"),
        (&["string"][..], "text string"),
        (&["bool"][..], "true false boolean"),
        (&["bytes"][..], "bytes binary"),
    ] {
        if types.iter().any(|ty| converter.contains(ty)) {
            words.push(' ');
            words.push_str(said);
        }
    }
    words
}

/// The start of a palette item's ID that names a conversion rather than a node kind.
pub(crate) const CONVERSION: &str = "convert:";

/// The palette ID of a conversion: its converter and its policy choices.
pub(crate) fn conversion_id(conversion: &orange_host::Conversion) -> String {
    let options: Vec<String> = conversion
        .options
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect();
    format!("{CONVERSION}{}?{}", conversion.id(), options.join("&"))
}

/// The conversion a palette ID names, among the host's.
pub(crate) fn conversion_named(host: &Host, id: &str) -> Option<orange_host::Conversion> {
    host.conversions()
        .into_iter()
        .find(|conversion| conversion_id(conversion) == id)
}

/// A conversion as search shows it: "Decimal → Whole number (floor)", and whether it
/// can lose or fail.
pub(crate) fn conversion_label(conversion: &orange_host::Conversion) -> String {
    let mut label = format!(
        "{} → {}",
        crate::type_name(&conversion.from),
        crate::type_name(&conversion.to)
    );
    if !conversion.options.is_empty() {
        let choices: Vec<&str> = conversion.options.values().map(String::as_str).collect();
        label.push_str(&format!(" ({})", choices.join(", ")));
    }
    label
}

fn conversion_item(conversion: &orange_host::Conversion, language: &str) -> PaletteItem {
    let purpose = if conversion.lossy {
        text_in(
            language,
            "conversion-lossy",
            "May lose detail, and says what it lost.",
        )
    } else if conversion.checked {
        text_in(
            language,
            "conversion-checked",
            "Fails when the value does not fit.",
        )
    } else {
        text_in(language, "conversion-exact", "Exact: loses nothing.")
    };
    PaletteItem {
        id: conversion_id(conversion),
        label: conversion_label(conversion),
        package: String::new(),
        purpose,
    }
}

/// A message of this editor in `language`, or `fallback`.
fn text_in(language: &str, key: &str, fallback: &str) -> String {
    crate::i18n::catalogs()
        .message(language, key)
        .map_or_else(|| fallback.to_owned(), std::borrow::Cow::into_owned)
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
    fn a_result_is_one_compact_line_without_a_card() {
        let ctx = egui::Context::default();
        let mut height = 0.0;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(280.0, 240.0),
                )),
                ..Default::default()
            },
            |ui| {
                height = result_row(ui, "And", "Both inputs", "core/and", false)
                    .rect
                    .height();
            },
        );
        output.textures_delta.clear();
        let texts: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, ["And"]);
        assert!(height <= 28.0);
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
                let row = result_row(ui, "AND", "Both values must be true", "core/and", false);
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
        assert_eq!(first("temporizador", "es"), "Con temporizador");
        assert_eq!(first("entero", "es"), "Entero");
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
