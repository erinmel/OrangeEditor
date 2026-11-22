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
        let category = match found.definition.as_str() {
            id if id.starts_with("org.orange.audio/") => {
                text_in(language, "palette-audio", "Audio")
            }
            "core/get-variable"
            | "core/set-variable"
            | "core/update-variable"
            | "preset:increment"
            | "preset:decrement" => text_in(language, "variables", "Variables"),
            id => family(id).map_or(found.category, |(key, caption, _)| {
                text_in(language, key, caption)
            }),
        };
        let item = PaletteItem {
            id: found.definition,
            label: found.label,
            package: found.package,
            purpose: found.purpose,
        };
        match groups.iter_mut().find(|(name, _)| *name == category) {
            Some((_, items)) => items.push(item),
            None => groups.push((category, vec![item])),
        }
    }
    // Conversions are search results too, in a category of their own.
    let converts = conversion_items(host, search, language, None);
    if !converts.is_empty() {
        groups.push((text_in(language, "conversions", "Conversions"), converts));
    }
    if search.trim().is_empty() {
        groups.sort_by(|a, b| a.0.cmp(&b.0));
    }
    if search.trim().is_empty() {
        for (_, items) in &mut groups {
            items.sort_by_key(|item| {
                let section = audio_section(&item.id).map(|(name, _)| name);
                match section {
                    Some("audio-routing") => 0,
                    Some("audio-playback") => 1,
                    Some("audio-effects") => 2,
                    Some("audio-analyze") => 3,
                    _ => 0,
                }
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
    let wanted = search.to_lowercase();
    host.conversions()
        .iter()
        .filter(|conversion| from.is_none_or(|ty| conversion.from == ty))
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

pub(crate) fn signal(node: &NodeView) -> Option<(bool, &str)> {
    let sender = match node.definition.as_str() {
        "core/emit" => true,
        "core/on-event" => false,
        _ => return None,
    };
    Some((sender, node.settings.get("event")?.as_str()?))
}

pub(crate) fn audio_section(definition: &str) -> Option<(&'static str, &'static str)> {
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

pub(crate) fn category_heading(
    category: &str,
    definition: &str,
    language: &str,
) -> (egui::text::LayoutJob, &'static str) {
    let (mut glyph, mut color, mut help) = badge(definition);
    if category == text_in(language, "palette-advanced", "Advanced") {
        glyph = crate::icons::SETTINGS;
        color = egui::Color32::from_rgb(149, 132, 178);
        help = "palette-advanced-help";
    }
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
            let audio = audio_section(&found.definition).is_some_and(|(section, _)| {
                let terms = orange_host::fold_search(&format!(
                    "audio sound sonido {} {}",
                    text_in(language, section, section),
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
                    "Wireless Signal",
                    "link relay trigger enlace señal senal evento",
                );
            if (alias
                || audio
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
                    "Wireless Signal",
                    "Send to named receivers without a long wire.",
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
                    "preset:signal" => "link relay trigger enlace señal senal evento",
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
        let (host, _) = Host::new(orange_host::HostOptions::default()).unwrap();
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
