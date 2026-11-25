//! Node catalogue projection and its presentation cache. The search itself is the
//! host's ([`Host::search_nodes`]), shared by this list and the quick-add box.
use crate::i18n::t;
use orange_host::{Draft, Host, NodeView};
use std::collections::HashMap;

#[derive(Clone)]
pub(crate) struct PaletteDrag {
    pub item: PaletteItem,
    pub card: Option<NodeView>,
}

/// A draggable name or an inline canvas card in a search result.
pub(crate) fn result_row(
    ui: &mut egui::Ui,
    name: &str,
    detail: &str,
    target: Option<&str>,
    highlighted: bool,
    card: Option<&NodeView>,
    definition: Option<&str>,
) -> egui::Response {
    let width = ui.available_width();
    let preview = card.map(|node| {
        let size = crate::canvas::node_size(node);
        let zoom = ((width - 12.0).max(1.0) / size.x).min(0.90);
        (node, size * zoom, zoom)
    });
    let height = preview.map_or(28.0, |(_, size, _)| size.y + 12.0);
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click_and_drag());
    let response = response.on_hover_ui(|ui| {
        ui.set_max_width(280.0);
        ui.label(if detail.is_empty() { name } else { detail });
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
    orange_ui_controls::list_row(ui, crate::controls::Surface::Panel, &response, highlighted);
    if let Some((node, size, zoom)) = preview {
        let preview_rect = egui::Rect::from_center_size(rect.center(), size);
        let mut preview_ui = ui.new_child(
            egui::UiBuilder::new()
                .id_salt(response.id)
                .max_rect(preview_rect),
        );
        crate::canvas::preview_card(&mut preview_ui, node, name, preview_rect, zoom);
        return response;
    }
    let font = egui::TextStyle::Body.resolve(ui.style());
    let color = visuals.text_color();
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_rows = 1;
    let left = if let Some(definition) = definition {
        let (glyph, accent, _) = badge(definition);
        crate::icons::paint_centered(
            ui.painter(),
            egui::pos2(rect.left() + 15.0, rect.center().y),
            glyph,
            font.size,
            color.lerp_to_gamma(accent, 0.57),
        );
        26.0
    } else {
        8.0
    };
    job.wrap.max_width = (rect.width() - left - 8.0).max(0.0);
    job.append(name, 0.0, egui::TextFormat::simple(font.clone(), color));
    if let Some(target) = target {
        let mut icon = [0; 4];
        job.append(
            crate::icons::FLOW.encode_utf8(&mut icon),
            5.0,
            egui::TextFormat::simple(egui::FontId::new(font.size, crate::icons::family()), color),
        );
        job.append(target, 5.0, egui::TextFormat::simple(font, color));
    }
    let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
    ui.painter().galley(
        egui::pos2(rect.min.x + left, rect.center().y - galley.size().y / 2.0),
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
        "core/set-variable" => &[
            ("in", true, t("port-enter"), ""),
            ("value", true, t("port-value"), "$T"),
            ("then", false, t("port-exit"), ""),
        ],
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
                exec: ty.is_empty(),
                ty: (!ty.is_empty()).then(|| ty.to_owned()),
                value: None,
                connected: false,
                default: None,
                label: Some(label.to_owned()),
            })
            .collect(),
        position: None,
        settings: serde_json::Value::Null,
        schema: None,
        references: if matches!(definition, "core/get-variable" | "core/set-variable") {
            vec![("/variable".to_owned(), orange_host::ReferenceKind::Variable)]
        } else {
            Vec::new()
        },
        types: Vec::new(),
        variadic: Vec::new(),
        paths: Vec::new(),
        controls: Default::default(),
        missing: None,
    })
}

/// Own only the currently displayed result, not old searches or provider snapshots.
#[derive(Debug, Default)]
pub(crate) struct PaletteCache {
    generation: Option<u64>,
    search: String,
    language: String,
    channels: Vec<(String, String)>,
    bridges: Vec<(String, String)>,
    groups: Vec<(String, Vec<PaletteItem>)>,
}

/// What a palette item places when it binds a Listen to an existing channel.
pub(crate) const BIND_LISTEN: &str = "bind:on-event:";

/// Quick-bind items for subscribers: each channel (event ID, name) that has a Dispatch
/// in the graph, offered under Listen when the search finds Listen or names the channel.
pub(crate) fn channel_items(
    channels: &[(String, String)],
    search: &str,
    listen_found: bool,
    listen: &str,
) -> Vec<PaletteItem> {
    subscriber_items(
        channels,
        search,
        listen_found,
        listen,
        BIND_LISTEN,
        "listen-bound-help",
    )
}

/// Quick-bind items for subscribers of either kind: Listen on a channel or Bridge In
/// on a Bridge Out, placed as `prefix` + the emitter's key.
fn subscriber_items(
    channels: &[(String, String)],
    search: &str,
    subscriber_found: bool,
    subscriber: &str,
    prefix: &str,
    help: &str,
) -> Vec<PaletteItem> {
    let (listen_found, listen) = (subscriber_found, subscriber);
    let query = orange_host::fold_search(search.trim());
    channels
        .iter()
        .filter(|(_, name)| {
            !query.is_empty()
                && (listen_found
                    || query
                        .split_whitespace()
                        .all(|word| orange_host::fold_search(name).contains(word)))
        })
        .map(|(id, name)| PaletteItem {
            id: format!("{prefix}{id}"),
            label: format!("{listen} · {name}"),
            package: String::new(),
            purpose: crate::i18n::tf(help, &[("channel", name.clone().into())]),
        })
        .collect()
}

pub(crate) struct PaletteResults<'a> {
    pub groups: &'a [(String, Vec<PaletteItem>)],
}

impl PaletteCache {
    pub fn get(
        &mut self,
        host: &Host,
        search: &str,
        language: &str,
        channels: &[(String, String)],
        bridges: &[(String, String)],
    ) -> PaletteResults<'_> {
        let generation = host.registry().generation();
        if self.generation != Some(generation)
            || self.search != search
            || self.language != language
            || self.channels != channels
            || self.bridges != bridges
        {
            self.groups = palette_in(host, search, language);
            // Existing channels follow Listen, so finding a subscriber also pairs it.
            let listen = self
                .groups
                .iter()
                .flat_map(|(_, items)| items)
                .find(|item| item.id == "core/on-event")
                .map(|item| item.label.clone());
            let bound = channel_items(
                channels,
                search,
                listen.is_some(),
                &listen.unwrap_or_else(|| text_in(language, "listen-node", "Listen")),
            );
            if !bound.is_empty() {
                let group = self
                    .groups
                    .iter()
                    .position(|(_, items)| items.iter().any(|item| item.id == "core/on-event"));
                match group {
                    Some(group) => {
                        let items = &mut self.groups[group].1;
                        let at = items
                            .iter()
                            .position(|item| item.id == "core/on-event")
                            .map_or(items.len(), |at| at + 1);
                        let _ = items.splice(at..at, bound);
                    }
                    None => self.groups.insert(
                        0,
                        (
                            text_in(language, "catalog-triggers", "Triggers & signals"),
                            bound,
                        ),
                    ),
                }
            }
            self.channels = channels.to_vec();
            // The same for Bridge In, under it, with each Bridge Out's channel.
            let bridge_in = self
                .groups
                .iter()
                .flat_map(|(_, items)| items)
                .find(|item| item.id == BRIDGE_IN)
                .map(|item| item.label.clone());
            let bound = subscriber_items(
                bridges,
                search,
                bridge_in.is_some(),
                &bridge_in.unwrap_or_else(|| text_in(language, "bridge-in-node", "Bridge In")),
                BIND_BRIDGE_IN,
                "bridge-bound-help",
            );
            if !bound.is_empty()
                && let Some(group) = self
                    .groups
                    .iter()
                    .position(|(_, items)| items.iter().any(|item| item.id == BRIDGE_IN))
            {
                let items = &mut self.groups[group].1;
                let at = items
                    .iter()
                    .position(|item| item.id == BRIDGE_IN)
                    .map_or(items.len(), |at| at + 1);
                let _ = items.splice(at..at, bound);
            }
            self.bridges = bridges.to_vec();
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
    templates: HashMap<String, Option<NodeView>>,
    cards: HashMap<String, Option<NodeView>>,
}

impl PreviewCache {
    pub fn invalidate(&mut self) {
        self.cards.clear();
        self.templates.clear();
        self.generation = None;
    }

    /// Forgets every card once the registry has changed.
    fn follow(&mut self, host: &Host) {
        if self.generation != Some(host.registry().generation()) {
            self.cards.clear();
            self.templates.clear();
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
        let cards = if draft.is_some() {
            &mut self.cards
        } else {
            &mut self.templates
        };
        if !cards.contains_key(definition) {
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
            cards.insert(definition.to_owned(), card);
        }
        cards.get(definition).and_then(Option::as_ref)
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
    for found in discover(host, search, language) {
        let category =
            catalog_group(&found.definition).map_or(found.category, |group| group.name(language));
        let purpose = if found.purpose.is_empty() {
            catalog_section(&found.definition)
                .map_or_else(String::new, |(_, help)| text_in(language, help, ""))
        } else {
            found.purpose
        };
        let item = PaletteItem {
            id: found.definition,
            label: found.label,
            package: found.package,
            purpose,
        };
        match groups.iter_mut().find(|(name, _)| *name == category) {
            Some((_, items)) => items.push(item),
            None => groups.push((category, vec![item])),
        }
    }
    let converts = conversion_items(host, search, language, None);
    if !converts.is_empty() {
        let category = DATA.name(language);
        if let Some((_, items)) = groups.iter_mut().find(|(name, _)| *name == category) {
            items.extend(converts);
        } else {
            groups.push((category, converts));
        }
    }
    if search.trim().is_empty() {
        groups.sort_by(|a, b| {
            catalog_group_rank(&a.0, language).cmp(&catalog_group_rank(&b.0, language))
        });
        for (_, items) in &mut groups {
            items.sort_by_key(|item| {
                let section = catalog_section(&item.id).map_or("", |(title, _)| title);
                let order = match section {
                    "audio-routing" => 0,
                    "audio-playback" => 1,
                    "audio-effects" => 2,
                    "audio-analyze" => 3,
                    _ => 0,
                };
                (order, section)
            });
        }
    }
    groups
}

pub(crate) fn conversion_items(
    host: &Host,
    search: &str,
    language: &str,
    from: Option<&str>,
) -> Vec<PaletteItem> {
    let conversions = host.conversions();
    matching_conversions(
        conversions
            .iter()
            .filter(|conversion| from.is_none_or(|ty| conversion.from == ty)),
        search,
        language,
    )
}

pub(crate) fn matching_conversions<'a>(
    conversions: impl Iterator<Item = &'a orange_host::Conversion>,
    search: &str,
    language: &str,
) -> Vec<PaletteItem> {
    let wanted = search.to_lowercase();
    conversions
        .map(|conversion| conversion_item(conversion, language))
        .filter(|item| {
            let terms = format!(
                "{} {} convert conversion convertir conversión {}",
                item.label,
                item.purpose,
                conversion_words(&item.id)
            )
            .to_lowercase();
            wanted.split_whitespace().all(|word| terms.contains(word))
        })
        .collect()
}

pub(crate) const BRIDGE_OUT: &str = "core/bridge-out";
pub(crate) const BRIDGE_IN: &str = "core/bridge-in";
/// What a palette item places when it binds a Bridge In to an existing Bridge Out.
pub(crate) const BIND_BRIDGE_IN: &str = "bind:bridge-in:";

/// The port a bridge's wireless link uses, which its card does not show: Bridge Out's
/// result and Bridge In's value.
pub(crate) fn bridge_link_port(node: &NodeView, port: &orange_host::PortView) -> bool {
    (node.definition == BRIDGE_OUT && !port.input && port.id == "result")
        || (node.definition == BRIDGE_IN && port.input && port.id == "value")
}

/// A node's bridge channel name, when it is a bridge.
pub(crate) fn bridge_channel(node: &NodeView) -> Option<&str> {
    matches!(node.definition.as_str(), BRIDGE_OUT | BRIDGE_IN)
        .then(|| node.settings["channel"].as_str().unwrap_or_default())
}

pub(crate) fn signal(node: &NodeView) -> Option<(bool, &str)> {
    let sender = match node.definition.as_str() {
        "core/emit" => true,
        "core/on-event" => false,
        _ => return None,
    };
    Some((sender, node.settings.get("event")?.as_str()?))
}

fn audio_section(definition: &str) -> Option<(&'static str, &'static str)> {
    let id = definition.strip_prefix("org.orange.audio/")?;
    Some(match id {
        "audio-source" | "audio-out" => ("audio-routing", "audio-routing-help"),
        "compressor" | "limiter" | "gain" | "filter" | "noise-gate" => {
            ("audio-effects", "audio-effects-help")
        }
        "audio-format" | "live-level" | "noise-level" | "multiband-spectrum" | "band-level"
        | "transient" | "source-finished" => ("audio-analyze", "audio-analyze-help"),
        _ => ("audio-playback", "audio-playback-help"),
    })
}

pub(crate) fn badge(definition: &str) -> (char, egui::Color32, &'static str) {
    use crate::icons;
    let (namespace, name) = definition.split_once('/').unwrap_or((definition, ""));
    let (glyph, rgb, description) = match (namespace, name) {
        ("core", "emit" | "on-event") | ("preset:signal" | "preset:watchdog", _) => {
            (icons::SIGNAL, [67, 152, 193], "node-family-events")
        }
        ("core", name) if name.starts_with("on-") => {
            (icons::FLOW, [205, 124, 58], "node-family-events")
        }
        ("core", "call" | "function-entry" | "function-outputs" | "return") => {
            (icons::FUNCTION, [137, 114, 207], "node-family-functions")
        }
        ("core", "and" | "or" | "all-conditions" | "any-conditions")
        | ("org.orange.math", "not" | "xor") => {
            (icons::BOOLEAN, [196, 160, 52], "node-family-logic")
        }
        ("core", name) if loop_name(name) => (icons::REFRESH, [51, 151, 163], "node-family-loops"),
        ("core", name) if name.contains("variable") || name.contains("shared") => {
            (icons::VARIABLE, [60, 156, 112], "node-family-state")
        }
        ("preset:increment" | "preset:decrement", _) => {
            (icons::VARIABLE, [60, 156, 112], "node-family-state")
        }
        ("core", "number" | "whole-number") | ("org.orange.math", _) => {
            (icons::MATH, [64, 145, 213], "node-family-math")
        }
        ("core", "text") | ("org.orange.text", _) => {
            (icons::TEXT, [183, 110, 174], "node-family-text")
        }
        ("core", "wait") | ("org.orange.time" | "org.orange.clock", _) => {
            (icons::CLOCK, [74, 152, 165], "node-family-time")
        }
        ("org.orange.audio", _) => (icons::AUDIO, [151, 119, 213], "node-family-audio"),
        ("org.orange.net", _) => (icons::NETWORK, [56, 150, 189], "node-family-network"),
        ("org.orange.files", _) => (icons::OPEN, [190, 144, 65], "node-family-files"),
        ("org.orange.logging", _) => (icons::LOG, [172, 129, 74], "node-family-log"),
        ("org.orange.json" | "org.orange.collections", _) => {
            (icons::VARIABLE, [62, 152, 130], "node-family-data")
        }
        (
            "core"
            | "preset:guard"
            | "preset:rising-edge"
            | "preset:falling-edge"
            | "preset:deadband",
            _,
        ) => (icons::LOGIC, [211, 131, 66], "node-family-flow"),
        _ => (icons::PACKAGES, [132, 132, 143], "node-family-package"),
    };
    (
        glyph,
        egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]),
        description,
    )
}

fn loop_name(name: &str) -> bool {
    matches!(
        name,
        "count"
            | "while"
            | "for-each"
            | "for-range"
            | "break"
            | "continue"
            | "repeat-begin"
            | "repeat-end"
            | "map-begin"
            | "map-end"
            | "filter-begin"
            | "filter-end"
            | "fold-begin"
            | "fold-end"
    )
}

#[derive(Clone, Copy)]
struct CatalogGroup {
    key: &'static str,
    fallback: &'static str,
    representative: &'static str,
    help: &'static str,
}

impl CatalogGroup {
    fn name(self, language: &str) -> String {
        text_in(language, self.key, self.fallback)
    }
}

const FLOW: CatalogGroup = CatalogGroup {
    key: "catalog-flow",
    fallback: "Flow & triggers",
    representative: "core/branch",
    help: "catalog-flow-help",
};
const DATA: CatalogGroup = CatalogGroup {
    key: "catalog-data",
    fallback: "Data & text",
    representative: "org.orange.json/get",
    help: "catalog-data-help",
};
const CALCULATE: CatalogGroup = CatalogGroup {
    key: "catalog-math",
    fallback: "Math & logic",
    representative: "org.orange.math/add",
    help: "catalog-math-help",
};
const SERVICES: CatalogGroup = CatalogGroup {
    key: "catalog-services",
    fallback: "Files & network",
    representative: "org.orange.net/http-request",
    help: "catalog-services-help",
};
const AUDIO: CatalogGroup = CatalogGroup {
    key: "palette-audio",
    fallback: "Audio",
    representative: "org.orange.audio/audio-source",
    help: "node-family-audio",
};
const TOOLS: CatalogGroup = CatalogGroup {
    key: "catalog-tools",
    fallback: "Utilities",
    representative: "org.orange.logging/log-message",
    help: "catalog-tools-help",
};

fn catalog_group_rank(category: &str, language: &str) -> usize {
    for (rank, group) in [FLOW, DATA, CALCULATE, SERVICES, AUDIO, TOOLS]
        .iter()
        .enumerate()
    {
        if category == group.name(language) || category == group.fallback {
            return rank;
        }
    }
    6
}

fn catalog_group(definition: &str) -> Option<CatalogGroup> {
    if definition.starts_with(CONVERSION) {
        return Some(DATA);
    }
    let (namespace, name) = definition.split_once('/').unwrap_or((definition, ""));
    Some(match (namespace, name) {
        ("org.orange.audio", _) => AUDIO,
        ("org.orange.input", "on-key-pressed") => FLOW,
        ("org.orange.files" | "org.orange.net", _) => SERVICES,
        ("org.orange.math", _) => CALCULATE,
        (
            "org.orange.json" | "org.orange.collections" | "org.orange.text" | "org.orange.color",
            _,
        ) => DATA,
        ("org.orange.input" | "org.orange.time" | "org.orange.clock" | "org.orange.logging", _) => {
            TOOLS
        }
        ("core", "number" | "whole-number" | "bool" | "text" | "value" | "true-or-false") => DATA,
        ("core", name) if name.contains("variable") || name.contains("shared") => DATA,
        ("preset:increment" | "preset:decrement", _) => DATA,
        ("core", "and" | "or" | "all-conditions" | "any-conditions") => CALCULATE,
        ("core", "register" | "close-handle" | "close-stream" | "next-item") => TOOLS,
        ("core", _) => FLOW,
        (preset, _) if preset.starts_with("preset:") => FLOW,
        _ => return None,
    })
}

pub(crate) fn catalog_section(definition: &str) -> Option<(&'static str, &'static str)> {
    if definition.starts_with(CONVERSION) {
        return Some(("conversions", "catalog-conversions-help"));
    }
    if let Some(section) = audio_section(definition) {
        return Some(section);
    }
    if let Some((key, _, _)) = family(definition) {
        match key {
            "palette-http" => return Some((key, "node-family-network")),
            "palette-compare" => return Some((key, "node-family-math")),
            "palette-loops" => return Some((key, "node-family-loops")),
            "palette-advanced" => return Some((key, "palette-advanced-help")),
            _ => {}
        }
    }
    let (namespace, name) = definition.split_once('/').unwrap_or((definition, ""));
    Some(match (namespace, name) {
        ("org.orange.files", _) => ("catalog-files", "node-family-files"),
        ("org.orange.net", _) => ("catalog-network", "node-family-network"),
        ("org.orange.math", _) => ("catalog-math-section", "node-family-math"),
        ("org.orange.json", _) => ("catalog-json", "node-family-data"),
        ("org.orange.collections", _) => ("catalog-collections", "node-family-data"),
        ("org.orange.text", _) => ("catalog-text", "node-family-text"),
        ("org.orange.color", _) => ("catalog-color", "catalog-data-help"),
        ("org.orange.time" | "org.orange.clock", _) => ("catalog-time", "node-family-time"),
        ("org.orange.logging", _) => ("catalog-logging", "node-family-log"),
        ("org.orange.input", "on-key-pressed") => ("catalog-triggers", "node-family-events"),
        ("org.orange.input", _) => ("catalog-input", "catalog-tools-help"),
        ("core", name) if name.starts_with("on-") || matches!(name, "emit" | "await-event") => {
            ("catalog-triggers", "node-family-events")
        }
        (
            "preset:signal"
            | "preset:watchdog"
            | "preset:wireless-value"
            | "preset:wireless-pulse"
            | "preset:rising-edge"
            | "preset:falling-edge"
            | "preset:deadband",
            _,
        ) => ("catalog-triggers", "node-family-events"),
        ("core", "call" | "return" | "function-entry" | "function-outputs") => {
            ("catalog-functions", "node-family-functions")
        }
        ("core", name) if name.contains("variable") || name.contains("shared") => {
            ("variables", "node-family-state")
        }
        ("preset:increment" | "preset:decrement", _) => ("variables", "node-family-state"),
        ("core", _) | ("preset:guard", _) => match catalog_group(definition)?.key {
            "catalog-data" => ("catalog-values", "node-family-data"),
            "catalog-math" => ("catalog-logic", "node-family-math"),
            _ => ("catalog-control", "node-family-flow"),
        },
        _ => return None,
    })
}

pub(crate) fn category_heading(
    category: &str,
    definition: &str,
    language: &str,
) -> (egui::text::LayoutJob, &'static str) {
    let group = catalog_group(definition).filter(|group| group.name(language) == category);
    let (glyph, color, help) = group.map_or_else(
        || badge(definition),
        |group| {
            let (glyph, color, _) = badge(group.representative);
            (glyph, color, group.help)
        },
    );
    let mut job = egui::text::LayoutJob::default();
    let mut content = [0; 4];
    job.append(
        glyph.encode_utf8(&mut content),
        0.0,
        egui::TextFormat::simple(egui::FontId::new(13.0, crate::icons::family()), color),
    );
    job.append(category, 6.0, egui::TextFormat::default());
    (job, help)
}

fn family(id: &str) -> Option<(&'static str, &'static str, &'static str)> {
    Some(match id {
        "org.orange.net/http-request"
        | "org.orange.net/http-request-bytes"
        | "org.orange.net/http-request-json"
        | "org.orange.net/http-stream"
        | "org.orange.net/http-stream-line" => (
            "palette-http",
            "HTTP Request",
            "http request peticion solicitud",
        ),
        "org.orange.audio/audio-format" | "org.orange.audio/live-level" => (
            "palette-audio-inspect",
            "Audio · Inspect",
            "audio inspect inspeccionar",
        ),
        "org.orange.audio/play" | "org.orange.audio/play-from" | "org.orange.audio/loop" => (
            "palette-playback",
            "Audio · Playback",
            "audio playback reproducir",
        ),
        "org.orange.files/read-text"
        | "org.orange.files/read-bytes"
        | "org.orange.files/read-range"
        | "org.orange.files/write-text"
        | "org.orange.files/write-bytes" => (
            "palette-file-content",
            "File Contents",
            "file contents archivo contenido",
        ),
        "org.orange.math/equal"
        | "org.orange.math/not-equal"
        | "org.orange.math/float-equal"
        | "org.orange.math/float-not-equal"
        | "org.orange.math/greater"
        | "org.orange.math/greater-or-equal"
        | "org.orange.math/less"
        | "org.orange.math/less-or-equal" => ("palette-compare", "Compare", "compare comparar"),
        "org.orange.math/is-finite"
        | "org.orange.math/is-nan"
        | "org.orange.math/is-infinite"
        | "org.orange.math/total-order-less"
        | "core/register"
        | "core/close-handle"
        | "core/close-stream"
        | "core/next-item"
        | "core/map-end"
        | "core/filter-end"
        | "core/fold-end"
        | "core/repeat-end" => ("palette-advanced", "Advanced", "advanced avanzado"),
        id if id.starts_with("org.orange.json/as-") || id.starts_with("org.orange.json/from-") => (
            "palette-json-convert",
            "Convert JSON Value",
            "convert json value convertir valor",
        ),
        id if id.strip_prefix("core/").is_some_and(loop_name) => (
            "palette-loops",
            "Loops",
            "loop repeat iterate bucle repetir iterar",
        ),
        _ => return None,
    })
}

pub(crate) fn discover(host: &Host, search: &str, language: &str) -> Vec<orange_host::Found> {
    let mut nodes = host.search_nodes(search, language);
    let query = orange_host::fold_search(search.trim());
    if !query.is_empty() {
        let counter = preset_match(
            &query,
            language,
            "counter-increment",
            "Increment",
            "bump+ bump + count up",
        ) || preset_match(
            &query,
            language,
            "counter-decrement",
            "Decrement",
            "bump- bump - count down",
        );
        for found in host.search_nodes("", language) {
            let alias = family(&found.definition).is_some_and(|(_, _, words)| {
                query.split_whitespace().all(|word| words.contains(word))
            });
            let catalog = catalog_group(&found.definition).is_some_and(|group| {
                let section = catalog_section(&found.definition)
                    .map_or_else(String::new, |(key, _)| text_in(language, key, key));
                let terms = orange_host::fold_search(&format!(
                    "{} {} {} {section}",
                    group.name(language),
                    group.fallback,
                    found.label
                ));
                query.split_whitespace().all(|word| terms.contains(word))
            });
            let deadband = found.definition == "core/on-watch"
                && preset_match(
                    &query,
                    language,
                    "when-deadband",
                    "Deadband",
                    "change tolerance",
                );
            let watchdog = found.definition == "core/on-event"
                && preset_match(
                    &query,
                    language,
                    "watchdog-preset",
                    "Idle Watchdog",
                    "heartbeat timeout",
                );
            let signal = found.definition == "core/on-event"
                && preset_match(
                    &query,
                    language,
                    "signal-preset",
                    "Dispatch and Listen",
                    "link relay trigger enlace señal senal evento wireless inalambrica inalámbrica dispatch listen despachar escuchar fan out",
                );
            let wireless_pulse = found.definition == "core/on-event"
                && preset_match(
                    &query,
                    language,
                    "wireless-pulse-preset",
                    "Dispatch without payload",
                    "event no payload pulse wireless dispatch pulso inalambrico",
                );
            if (alias
                || wireless_pulse
                || catalog
                || signal
                || deadband
                || watchdog
                || counter && found.definition == "core/update-variable")
                && !nodes.iter().any(|node| node.definition == found.definition)
            {
                nodes.push(found);
            }
        }
    }
    search_with_presets(nodes, search, language)
}

fn preset_match(query: &str, language: &str, key: &str, label: &str, aliases: &str) -> bool {
    let terms = orange_host::fold_search(&format!(
        "{label} {} {aliases}",
        text_in(language, key, label)
    ));
    query.split_whitespace().all(|word| terms.contains(word))
}

pub(crate) fn search_with_presets(
    results: Vec<orange_host::Found>,
    search: &str,
    language: &str,
) -> Vec<orange_host::Found> {
    let query = orange_host::fold_search(search.trim());
    let mut choices = Vec::with_capacity(results.len());
    for base in results {
        choices.extend(
            [
                (
                    "core/on-watch",
                    "preset:rising-edge",
                    "rising-edge",
                    "Rising Edge",
                    "Fires when a boolean becomes true.",
                ),
                (
                    "core/on-watch",
                    "preset:falling-edge",
                    "falling-edge",
                    "Falling Edge",
                    "Fires when a boolean becomes false.",
                ),
                (
                    "core/on-watch",
                    "preset:deadband",
                    "when-deadband",
                    "Deadband",
                    "Fires when a numeric value moves beyond a tolerance.",
                ),
                (
                    "core/update-variable",
                    "preset:increment",
                    "counter-increment",
                    "Increment",
                    "Add a step to a counter.",
                ),
                (
                    "core/update-variable",
                    "preset:decrement",
                    "counter-decrement",
                    "Decrement",
                    "Subtract a step from a counter.",
                ),
                (
                    "core/on-event",
                    "preset:watchdog",
                    "watchdog-preset",
                    "Idle Watchdog",
                    "Wire heartbeats into Emit. Wait finishes after the last heartbeat.",
                ),
                (
                    "core/on-event",
                    "preset:signal",
                    "signal-preset",
                    "Dispatch and Listen",
                    "Dispatch an event on a named channel to any number of Listen nodes.",
                ),
                (
                    "core/on-event",
                    "preset:wireless-pulse",
                    "wireless-pulse-preset",
                    "Dispatch without payload",
                    "Start receivers with no payload value.",
                ),
                (
                    "core/branch",
                    "preset:guard",
                    "guard-preset",
                    "Guard",
                    "Wire True to continue only when the condition passes.",
                ),
            ]
            .into_iter()
            .filter(|(parent, ..)| *parent == base.definition)
            .filter_map(|(_, id, key, fallback, purpose)| {
                let label = text_in(language, key, fallback);
                let purpose = text_in(language, &format!("{key}-help"), purpose);
                let aliases = match id {
                    "preset:deadband" => "change tolerance",
                    "preset:watchdog" => "heartbeat timeout",
                    "preset:wireless-value" => "store read shared value",
                    "preset:wireless-pulse" => "event no payload pulse wireless dispatch pulso inalambrico",
                    "preset:signal" => "link relay trigger enlace señal senal evento wireless inalambrica inalámbrica dispatch listen despachar escuchar fan out",
                    "preset:increment" => "bump+ bump + count up",
                    "preset:decrement" => "bump- bump - count down",
                    _ => "",
                };
                preset_match(&query, language, key, fallback, aliases).then(|| orange_host::Found {
                    definition: id.to_owned(),
                    label,
                    package: base.package.clone(),
                    category: base.category.clone(),
                    purpose,
                    matched: base.matched,
                })
            }),
        );
        choices.push(base);
    }
    choices
}

/// Other words for the two types a conversion ID names, so "whole number text" finds
/// Int to Text.
fn conversion_words(id: &str) -> String {
    let converter = id
        .trim_start_matches(CONVERSION)
        .split('?')
        .next()
        .unwrap_or_default();
    let mut words = String::new();
    if utf8_label(converter).is_some() {
        words.push_str(" utf8 utf-8 decode decodificar bytes to text ");
    }
    for (types, said) in [
        (
            &["i32", "i64", "u32", "u64"][..],
            "whole number integer int entero número",
        ),
        (&["f32", "f64"][..], "decimal float number número"),
        (&["string"][..], "text string texto cadena"),
        (&["bool"][..], "true false boolean verdadero falso booleano"),
        (&["bytes"][..], "bytes binary binario"),
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

/// A conversion as search shows it: "Float to Int (floor)".
pub(crate) fn conversion_label(conversion: &orange_host::Conversion) -> String {
    if let Some(label) = utf8_label(conversion.id()) {
        return label.to_owned();
    }
    let mut label = crate::conversion_name(&conversion.from, &conversion.to);
    if !conversion.options.is_empty() {
        let choices: Vec<&str> = conversion
            .options
            .values()
            .map(|value| match value.as_str() {
                "seconds" => t("conversion-unit-seconds"),
                "milliseconds" => t("conversion-unit-milliseconds"),
                "microseconds" => t("conversion-unit-microseconds"),
                "nanoseconds" => t("conversion-unit-nanoseconds"),
                "minutes" => t("conversion-unit-minutes"),
                "hours" => t("conversion-unit-hours"),
                "truncate" => t("conversion-round-truncate"),
                "floor" => t("conversion-round-floor"),
                "ceiling" => t("conversion-round-ceiling"),
                "nearest-even" => t("conversion-round-nearest"),
                "reject" => t("conversion-overflow-reject"),
                other => other,
            })
            .collect();
        label.push_str(&format!(" ({})", choices.join(", ")));
    }
    label
}

pub(crate) fn conversion_detail(conversion: &orange_host::Conversion) -> &'static str {
    match conversion.id() {
        "core/f64-to-duration" => t("conversion-duration-round"),
        "core/timestamp-to-i64" => t("conversion-timestamp-truncate"),
        _ if conversion.lossy => t("conversion-lossy"),
        _ if conversion.checked => t("conversion-checked"),
        _ => t("conversion-exact"),
    }
}

pub(crate) fn utf8_label(converter: &str) -> Option<&'static str> {
    match converter {
        "core/bytes-to-string" => Some(t("decode-utf8")),
        "core/bytes-to-string-lossy" => Some(t("decode-utf8-lossy")),
        _ => None,
    }
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
    fn counter_presets_share_an_initialized_signed_counter_and_offer_a_step() {
        let (host, _) = Host::new(HostOptions::default()).unwrap();
        let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
        for (query, preset, operation) in [
            ("bump+", "preset:increment", "add"),
            ("bump -", "preset:decrement", "sub"),
        ] {
            assert!(
                discover(&host, query, "en")
                    .iter()
                    .any(|node| node.definition == preset)
            );
            let id = host.add_node(&mut draft, preset, [0.0, 0.0]).unwrap();
            let graph = host.draft_view(&draft).unwrap();
            let card = graph.nodes.iter().find(|node| node.id == id).unwrap();
            assert_eq!(card.settings["operation"], operation);
            assert!(
                card.ports
                    .iter()
                    .any(|port| port.id == "value" && port.value.as_deref() == Some("1"))
            );
        }
        assert_eq!(draft.document().variables().len(), 1);
    }

    #[test]
    fn counter_presets_reuse_the_numbered_counter_after_a_name_collision() {
        let (host, _) = Host::new(HostOptions::default()).unwrap();
        let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
        host.declare_variable(
            &mut draft,
            &orange_host::Variable {
                id: None,
                name: "Counter".to_owned(),
                ty: "core/string@1".to_owned(),
                scope: orange_host::VarScope::Run,
                persistent: false,
                initial: None,
            },
        )
        .unwrap();

        let increment = host
            .add_node(&mut draft, "preset:increment", [0.0, 0.0])
            .unwrap();
        let decrement = host
            .add_node(&mut draft, "preset:decrement", [0.0, 0.0])
            .unwrap();
        let doc = draft.document();
        let counter = doc
            .variables()
            .values()
            .find(|var| var.name == "Counter 2")
            .unwrap();
        assert_eq!(doc.variables().len(), 2);
        for node in [increment, decrement] {
            assert_eq!(
                doc.root().nodes[&node].config["variable"],
                serde_json::json!(counter.id)
            );
        }
    }

    /// Searching for a subscriber offers each channel a Dispatch sends on, bound in
    /// one step; naming the channel finds it too, and an empty search lists nothing.
    #[test]
    fn listen_searches_offer_existing_channels_to_bind() {
        let channels = [
            ("e1".to_owned(), "Kick".to_owned()),
            ("e2".to_owned(), "Door bell".to_owned()),
        ];
        let found = channel_items(&channels, "listen", true, "Listen");
        assert_eq!(
            found
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            [format!("{BIND_LISTEN}e1"), format!("{BIND_LISTEN}e2")]
        );
        assert_eq!(found[0].label, "Listen · Kick");
        let named = channel_items(&channels, "door", false, "Listen");
        assert_eq!(named.len(), 1);
        assert_eq!(named[0].id, format!("{BIND_LISTEN}e2"));
        assert!(channel_items(&channels, "", true, "Listen").is_empty());
    }

    #[test]
    fn wireless_signal_shares_one_event_with_multiple_receivers() {
        let (mut host, _) = Host::new(HostOptions::default()).unwrap();
        let mut editor = crate::Editor::new();
        editor.open_untitled(&mut host, orange_host::new_document().unwrap());
        assert!(
            discover(&host, "senal inalambrica", "es")
                .iter()
                .any(|r| r.definition == "preset:signal")
        );
        let sender = editor
            .add_node_at(&mut host, "preset:signal", [0.0, 0.0])
            .unwrap();
        editor.open_menu(
            crate::MenuTarget::Node(sender),
            [64.0, 0.0],
            egui::Pos2::ZERO,
        );
        assert!(
            editor
                .menu_entries()
                .iter()
                .any(|entry| entry.command == crate::Command::AddSignalReceiver)
        );
        assert!(editor.run_command(&mut host, crate::Command::AddSignalReceiver));
        let graph = editor.graph().unwrap();
        let sender = graph.nodes.iter().find(|node| node.id == sender).unwrap();
        let (_, event) = signal(sender).unwrap();
        assert_eq!(
            graph
                .nodes
                .iter()
                .filter(|node| signal(node) == Some((false, event)))
                .count(),
            2
        );
        assert!(graph.wires.is_empty());
        assert_eq!(
            editor
                .document
                .as_ref()
                .unwrap()
                .draft
                .document()
                .events()
                .len(),
            1
        );
    }

    #[test]
    fn catalog_groups_related_operations_under_common_headings() {
        for (nodes, expected) in [
            (
                ["core/on-manual", "core/for-each", "preset:signal"],
                "Flow & triggers",
            ),
            (
                [
                    "core/get-variable",
                    "org.orange.json/get",
                    "convert:example",
                ],
                "Data & text",
            ),
            (
                [
                    "org.orange.files/read-text",
                    "org.orange.net/http-request",
                    "org.orange.net/websocket",
                ],
                "Files & network",
            ),
        ] {
            for node in nodes {
                assert_eq!(catalog_group(node).unwrap().name("en"), expected);
            }
        }
        assert_eq!(catalog_section("core/for-each").unwrap().0, "palette-loops");
        assert_eq!(
            catalog_section("org.orange.net/http-request").unwrap().0,
            "palette-http"
        );
        let (host, _) = Host::new(HostOptions::default()).unwrap();
        assert!(
            palette_in(&host, "datos texto entero", "es")
                .iter()
                .any(|(group, items)| {
                    group == "Datos y texto"
                        && items.iter().any(|item| item.id == "core/whole-number")
                })
        );
    }

    #[test]
    fn audio_is_one_category_and_its_sections_are_searchable() {
        let (host, _) =
            Host::with_services(HostOptions::default(), vec![orange_audio::package()]).unwrap();
        let audio: Vec<_> = palette_in(&host, "", "es")
            .into_iter()
            .filter(|(_, items)| {
                items
                    .iter()
                    .any(|item| item.id.starts_with("org.orange.audio/"))
            })
            .collect();
        assert_eq!(audio.len(), 1);
        assert_eq!(audio[0].0, "Audio");
        assert!(
            discover(&host, "procesar audio", "es")
                .iter()
                .any(|item| item.definition == "org.orange.audio/compressor")
        );
    }

    /// A Bridge Out gets a channel of its own; + Add Subscriber binds a Bridge In to it;
    /// deleting the Bridge Out keeps the Bridge In Unbound with the channel it had, and
    /// Create Emitter binds a new Bridge Out on that channel.
    #[test]
    fn bridges_spawn_bound_survive_their_emitter_and_rebind() {
        let (mut host, _) = Host::new(orange_host::HostOptions::default()).unwrap();
        let mut editor = crate::Editor::new();
        editor.open_untitled(&mut host, orange_host::new_document().unwrap());
        let out = editor
            .add_node_at(&mut host, BRIDGE_OUT, [0.0, 0.0])
            .unwrap();
        let second = editor
            .add_node_at(&mut host, BRIDGE_OUT, [0.0, 300.0])
            .unwrap();
        let channel = |editor: &crate::Editor, id| {
            let graph = editor.graph().unwrap();
            let node = graph.nodes.iter().find(|node| node.id == id).unwrap();
            bridge_channel(node).unwrap().to_owned()
        };
        assert_eq!(channel(&editor, out), "Bridge 1");
        assert_eq!(channel(&editor, second), "Bridge 2");
        assert!(editor.add_signal_endpoint(&mut host, out, false));
        let reading = {
            let graph = editor.graph().unwrap();
            let node = graph
                .nodes
                .iter()
                .find(|node| node.definition == BRIDGE_IN)
                .unwrap();
            assert!(
                graph
                    .wires
                    .iter()
                    .any(|wire| wire.from.0 == out && wire.to.0 == node.id)
            );
            node.id
        };
        assert_eq!(channel(&editor, reading), "Bridge 1");
        assert!(editor.remove_nodes(&mut host, &[out]));
        assert_eq!(
            channel(&editor, reading),
            "Bridge 1",
            "the channel name is kept"
        );
        assert!(
            !editor
                .graph()
                .unwrap()
                .wires
                .iter()
                .any(|wire| wire.to.0 == reading)
        );
        // Create Emitter, from the Unbound tag.
        assert!(editor.add_signal_endpoint(&mut host, reading, true));
        let graph = editor.graph().unwrap();
        let created = graph
            .nodes
            .iter()
            .find(|node| node.definition == BRIDGE_OUT && node.id != second)
            .unwrap();
        assert_eq!(bridge_channel(created), Some("Bridge 1"));
        assert!(
            graph
                .wires
                .iter()
                .any(|wire| wire.from.0 == created.id && wire.to.0 == reading)
        );
    }

    /// A value shared without a wire is a bridge, not a variable: searching for one finds
    /// Bridge Out and Bridge In, and the retired Wireless Value preset is not offered.
    #[test]
    fn a_wireless_value_is_a_bridge_not_a_variable() {
        let (host, _) = Host::new(orange_host::HostOptions::default()).unwrap();
        for (search, language) in [
            ("valor inalámbrico", "es"),
            ("wireless value", "en"),
            ("pull value", "en"),
        ] {
            let found = discover(&host, search, language);
            assert!(
                found.iter().any(|node| node.definition == BRIDGE_OUT),
                "{search}"
            );
            assert!(
                found.iter().any(|node| node.definition == BRIDGE_IN),
                "{search}"
            );
            assert!(
                !found
                    .iter()
                    .any(|node| node.definition == "preset:wireless-value")
            );
        }
    }

    #[test]
    fn watchdog_search_places_an_editable_timer_as_one_undo() {
        let (mut host, _) = Host::new(HostOptions::default()).unwrap();
        assert!(
            discover(&host, "detector de inactividad", "es")
                .iter()
                .any(|node| node.definition == "preset:watchdog")
        );
        let mut editor = crate::Editor::new();
        editor.open_untitled(&mut host, orange_host::new_document().unwrap());
        editor.open_quick_add(None, [0.0, 0.0], egui::Pos2::ZERO);
        let choice = editor
            .quick_choices(&host, "heartbeat timeout")
            .into_iter()
            .find(|choice| choice.definition == "preset:watchdog")
            .unwrap();
        let emitter = editor.pick_quick(&mut host, &choice).unwrap();
        let draft = &editor.document.as_ref().unwrap().draft;
        let doc = draft.document();
        let body = doc.root();
        assert_eq!(doc.events().len(), 1);
        assert_eq!(body.nodes.len(), 3);
        assert_eq!(
            doc.events().values().next().unwrap().payload.as_str(),
            "core/option@1<core/bool@1>"
        );
        let edge = body.edges.values().next().unwrap();
        let entry = &body.nodes[&edge.from.node];
        let wait = &body.nodes[&edge.to.node];
        assert_eq!(body.nodes[&emitter].definition.id.as_str(), "core/emit");
        assert_eq!(entry.definition.id.as_str(), "core/on-event");
        assert_eq!(
            serde_json::to_value(entry.activation.as_ref().unwrap()).unwrap()["mode"],
            "replace"
        );
        assert_eq!(wait.definition.id.as_str(), "core/wait");
        assert_eq!(body.edges.len(), 1);
        editor.undo(&mut host);
        assert!(editor.graph().unwrap().nodes.is_empty());
        assert!(
            editor
                .document
                .as_ref()
                .unwrap()
                .draft
                .document()
                .events()
                .is_empty()
        );
    }

    #[test]
    fn guard_search_places_a_branch_with_a_boolean_condition() {
        let (mut host, _) = Host::new(HostOptions::default()).unwrap();
        assert!(
            palette_in(&host, "guarda", "es")
                .iter()
                .flat_map(|(_, items)| items)
                .any(|item| item.id == "preset:guard")
        );
        let mut editor = crate::Editor::new();
        editor.open_untitled(&mut host, orange_host::new_document().unwrap());
        editor.open_quick_add(None, [0.0, 0.0], egui::Pos2::ZERO);
        let choice = editor.quick_choices(&host, "guard").remove(0);
        assert_eq!(choice.definition, "preset:guard");
        let node = editor.pick_quick(&mut host, &choice).unwrap();
        let graph = editor.graph().unwrap();
        let card = graph.nodes.iter().find(|card| card.id == node).unwrap();
        assert_eq!(card.definition, "core/branch");
        assert!(card.ports.iter().any(|port| port.input
            && port.id == "condition"
            && port.ty.as_deref() == Some("core/bool@1")));
    }

    #[test]
    fn edge_presets_are_searchable_and_place_a_typed_watch() {
        let (mut host, _) = Host::new(HostOptions::default()).unwrap();
        let mut editor = crate::Editor::new();
        editor.open_untitled(&mut host, orange_host::new_document().unwrap());
        for (language, query, id, mode) in [
            ("en", "rising edge", "preset:rising-edge", "rising"),
            ("es", "flanco descendente", "preset:falling-edge", "falling"),
        ] {
            assert!(
                palette_in(&host, query, language)
                    .iter()
                    .flat_map(|(_, items)| items)
                    .any(|item| item.id == id)
            );
            let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
            let node = host.add_node(&mut draft, id, [0.0, 0.0]).unwrap();
            let inserted = host.draft_view(&draft).unwrap();
            let card = inserted.nodes.iter().find(|card| card.id == node).unwrap();
            assert_eq!(card.definition, "core/on-watch");
            assert_eq!(card.settings["mode"], mode);
            assert!(card.ports.iter().any(|port| port.input
                && port.id == "watch"
                && port.ty.as_deref() == Some("core/bool@1")));
            assert_eq!(
                host.placement_preview(&draft, id).unwrap().settings["mode"],
                mode
            );
        }
        editor.open_quick_add(None, [0.0, 0.0], egui::Pos2::ZERO);
        let choice = editor
            .quick_choices(&host, "falling edge")
            .into_iter()
            .find(|choice| choice.definition == "preset:falling-edge")
            .unwrap();
        let node = editor.pick_quick(&mut host, &choice).unwrap();
        let graph = editor.graph().unwrap();
        assert_eq!(
            graph
                .nodes
                .iter()
                .find(|card| card.id == node)
                .unwrap()
                .settings["mode"],
            "falling"
        );
        editor.undo(&mut host);
        assert!(editor.graph().unwrap().nodes.is_empty());
    }

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
        let mut row_layer = None;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(280.0, 240.0),
                )),
                ..Default::default()
            },
            |ui| {
                let row = result_row(ui, "And", "Both inputs", None, false, None, None);
                height = row.rect.height();
                row_layer = Some((row.rect, orange_ui_controls::Theme::of(ui).secondary));
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
        let (rect, fill) = row_layer.unwrap();
        assert!(output.shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Rect(layer) if layer.rect == rect && layer.fill == fill)
        }));
        output.drop_without_applying_deltas();
    }

    #[test]
    fn a_wire_result_uses_the_bundled_arrow_font() {
        let ctx = egui::Context::default();
        crate::icons::install(&ctx);
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            result_row(ui, "Absolute", "", Some("value"), true, None, None);
        });
        assert!(output.shapes.iter().any(|shape| match &shape.shape {
            egui::Shape::Text(text) => {
                text.galley.text().contains(crate::icons::FLOW)
                    && text
                        .galley
                        .job
                        .sections
                        .iter()
                        .any(|section| section.format.font_id.family == crate::icons::family())
            }
            _ => false,
        }));
        output.drop_without_applying_deltas();
    }

    #[test]
    fn the_splitter_search_card_paints_its_chart_without_hover() {
        let (host, _) =
            Host::with_services(HostOptions::default(), vec![orange_audio::package()]).unwrap();
        let mut previews = PreviewCache::default();
        let card = previews
            .card(&host, None, orange_audio::MULTIBAND_SPECTRUM)
            .unwrap();
        let ctx = egui::Context::default();
        crate::icons::install(&ctx);
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(300.0);
            result_row(ui, "Frequency Splitter", "", None, false, Some(card), None);
        });
        for expected in [t("spectrum-live-trace"), t("spectrum-connect"), "Hz"] {
            assert!(
                output.shapes.iter().any(|shape| matches!(&shape.shape,
                    egui::Shape::Text(text) if text.galley.text() == expected
                )),
                "missing chart text {expected}",
            );
        }
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
                    None,
                    false,
                    None,
                    None,
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
        assert_eq!(
            cache.get(&host, "", "en", &[], &[]).groups,
            palette(&host, "")
        );
        let allocation = cache.get(&host, "", "en", &[], &[]).groups.as_ptr();
        assert_eq!(
            cache.get(&host, "", "en", &[], &[]).groups.as_ptr(),
            allocation
        );
        for search in ["MATH add", "unmatched-xyz", ""] {
            assert_eq!(
                cache.get(&host, search, "en", &[], &[]).groups,
                palette(&host, search)
            );
        }
        assert_eq!(
            cache.get(&host, "disparador", "es", &[], &[]).groups,
            palette_in(&host, "disparador", "es")
        );
        assert!(
            cache
                .get(&host, "disparador", "en", &[], &[])
                .groups
                .is_empty()
        );
        // Registry generations are process-unique, including when one editor is
        // reused with another host. Old provider labels must not survive that switch.
        let (core, _events) = Host::new(HostOptions {
            base_nodes: false,
            ..Default::default()
        })
        .unwrap();
        assert_ne!(core.registry().generation(), host.registry().generation());
        assert_ne!(palette(&core, ""), palette(&host, ""));
        assert_eq!(
            cache.get(&core, "", "en", &[], &[]).groups,
            palette(&core, "")
        );
        assert_eq!(
            cache.get(&host, "", "en", &[], &[]).groups,
            palette(&host, "")
        );
    }
}
