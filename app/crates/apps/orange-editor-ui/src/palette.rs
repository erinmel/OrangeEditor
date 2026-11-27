//! Node catalogue projection and its presentation cache. The search itself is the
//! host's ([`Host::search_nodes`]), shared by this list and the quick-add box.
use crate::i18n::t;
use orange_host::intrinsic;
use orange_host::preset;
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
        ui.strong(name);
        if !detail.is_empty() {
            ui.label(detail);
        }
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
        intrinsic::GET_VARIABLE => &[("value", false, t("port-value"), "$T")],
        intrinsic::SET_VARIABLE => &[
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
        references: if matches!(
            definition,
            intrinsic::GET_VARIABLE | intrinsic::SET_VARIABLE
        ) {
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
    revision: Option<(u64, u64)>,
    search: String,
    language: String,
    channels: Vec<(String, String)>,
    groups: Vec<(String, Vec<PaletteItem>)>,
}

/// What a palette item places when it binds a Listen to an existing channel.
pub(crate) const BIND_LISTEN: &str = "bind:on-event:";

/// Quick-bind items for subscribers: each channel (event ID, name) that has a Dispatch
/// in the graph, offered under Listen when the search finds Listen or names the channel.
///
/// The search is tried as a channel name first. Failing that, words that name the
/// subscriber kind ("listen", in any search language and with a typo) are dropped,
/// first from the front and then anywhere, and the rest is tried. A search made only
/// of such words lists every channel when it names Listen itself.
pub(crate) fn channel_items(
    channels: &[(String, String)],
    search: &str,
    listen_found: bool,
    listen: &str,
    language: &str,
) -> Vec<PaletteItem> {
    let direct = orange_host::SearchQuery::new(search);
    if !direct.is_valid() || direct.is_empty() {
        return Vec::new();
    }
    let kind_words: Vec<_> = std::iter::once(language)
        .chain(orange_host::SEARCH_LANGUAGES)
        .map(|tag| text_in(tag, "listen-search-words", ""))
        .collect();
    let names_kind = |word: &str| names_listen(word, &kind_words);
    let folded = orange_host::fold_search(search);
    let words: Vec<&str> = folded.split_whitespace().collect();
    let leading = words.iter().take_while(|word| names_kind(word)).count();
    let without_leading = (1..=leading.min(words.len() - 1)).map(|skip| words[skip..].join(" "));
    let without_any = (leading < words.len())
        .then(|| {
            words
                .iter()
                .copied()
                .filter(|word| !names_kind(word))
                .collect::<Vec<_>>()
        })
        .filter(|rest| rest.len() < words.len())
        .map(|rest| rest.join(" "));
    let matching_channels = |text: &str| {
        let query = orange_host::SearchQuery::new(text);
        channels
            .iter()
            .filter_map(|channel| {
                query
                    .rank([channel.1.as_str()], [])
                    .map(|rank| (rank, channel))
            })
            .collect::<Vec<_>>()
    };
    let found = std::iter::once(search.to_owned())
        .chain(without_leading)
        .chain(without_any)
        .map(|text| matching_channels(&text))
        .find(|found| !found.is_empty());
    let explicit_kind = listen_found || orange_host::fold_search(listen) == folded;
    let mut named = match found {
        Some(found) => found,
        None if leading == words.len() && explicit_kind => channels
            .iter()
            .map(|channel| (orange_host::SearchRank::BEST, channel))
            .collect(),
        None => Vec::new(),
    };
    named.sort_by_key(|(rank, _)| *rank);
    named
        .into_iter()
        .map(|(_, (id, name))| PaletteItem {
            id: format!("{BIND_LISTEN}{id}"),
            label: format!("{listen} · {name}"),
            package: String::new(),
            purpose: crate::i18n::catalogs()
                .format(
                    language,
                    "listen-bound-help",
                    &[("channel", name.as_str().into())],
                )
                .map_or_else(String::new, std::borrow::Cow::into_owned),
        })
        .collect()
}

/// Whether a folded search word names the subscriber kind: one of the `captions`'
/// words exactly or, from four letters on, with a typo or as its start.
fn names_listen(word: &str, captions: &[String]) -> bool {
    let typed = orange_host::SearchQuery::new(word);
    captions
        .iter()
        .flat_map(|caption| caption.split_whitespace())
        .any(|term| {
            orange_host::fold_search(term) == word
                || word.chars().count() >= 4 && typed.matches([term])
        })
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
    ) -> PaletteResults<'_> {
        let revision = host.search_revision();
        if self.revision != Some(revision)
            || self.search != search
            || self.language != language
            || self.channels != channels
        {
            self.groups = palette_in(host, search, language);
            // Existing channels follow Listen, so finding a subscriber also pairs it.
            let listen = self
                .groups
                .iter()
                .flat_map(|(_, items)| items)
                .find(|item| item.id == intrinsic::ON_EVENT)
                .map(|item| item.label.clone());
            let bound = channel_items(
                channels,
                search,
                listen.is_some(),
                &listen.unwrap_or_else(|| text_in(language, "listen-node", "Listen")),
                language,
            );
            if !bound.is_empty() {
                let group = self
                    .groups
                    .iter()
                    .position(|(_, items)| items.iter().any(|item| item.id == intrinsic::ON_EVENT));
                match group {
                    Some(group) => {
                        let items = &mut self.groups[group].1;
                        let at = items
                            .iter()
                            .position(|item| item.id == intrinsic::ON_EVENT)
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
            self.search.clear();
            self.search.push_str(search);
            self.language.clear();
            self.language.push_str(language);
            self.revision = Some(revision);
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
    conversions: Option<HashMap<String, orange_host::Conversion>>,
    templates: HashMap<String, Option<NodeView>>,
    cards: HashMap<String, Option<NodeView>>,
}

impl PreviewCache {
    pub fn invalidate(&mut self) {
        self.cards.clear();
        self.templates.clear();
        self.conversions = None;
        self.generation = None;
    }

    /// Forgets every card once the registry has changed.
    fn follow(&mut self, host: &Host) {
        if self.generation != Some(host.registry().generation()) {
            self.invalidate();
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
                let empty;
                let draft = if let Some(draft) = draft {
                    draft
                } else {
                    let text = orange_host::new_document().ok()?;
                    empty = host.draft(&text).ok()?;
                    &empty
                };
                if definition.starts_with(CONVERSION) {
                    let conversions = self.conversions.get_or_insert_with(|| {
                        host.conversions()
                            .into_iter()
                            .map(|conversion| (conversion_id(&conversion), conversion))
                            .collect()
                    });
                    host.conversion_preview(draft, conversions.get(definition)?)
                        .ok()
                } else {
                    host.placement_preview(draft, definition).ok()
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

/// The host's nodes that match `search` in `language` or a bundled search language.
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
            label: found.label,
            id: found.definition,
            package: found.package,
            purpose,
        };
        match groups.iter_mut().find(|(name, _)| *name == category) {
            Some((_, items)) => items.push(item),
            None => groups.push((category, vec![item])),
        }
    }
    if search.trim().is_empty() {
        groups.sort_by(|a, b| {
            catalog_group_rank(&a.1[0].id)
                .cmp(&catalog_group_rank(&b.1[0].id))
                .then_with(|| a.0.cmp(&b.0))
        });
        for (_, items) in &mut groups {
            items.sort_by_key(|item| {
                let section = catalog_section(&item.id).map_or("", |(title, _)| title);
                let order = match section {
                    "audio-routing" => 0,
                    "audio-playback" => 1,
                    "audio-effects" => 2,
                    "audio-analyze" => 3,
                    "catalog-loop-ends" => 4,
                    _ => 0,
                };
                (order, section, browse_order(&item.id))
            });
        }
    }
    groups.extend(conversion_groups(host, search, language));
    groups
}

fn browse_order(id: &str) -> u8 {
    match id {
        intrinsic::ON_EVENT => 1,
        intrinsic::BRIDGE_OUT => 2,
        intrinsic::BRIDGE_IN => 3,
        _ => 0,
    }
}

fn conversion_groups(host: &Host, search: &str, language: &str) -> Vec<(String, Vec<PaletteItem>)> {
    let conversions = host.conversions();
    let mut groups: Vec<(&str, Vec<PaletteItem>)> = Vec::new();
    for (conversion, item) in ranked_conversions(conversions.iter(), search, language) {
        let source = conversion.from.as_str();
        match groups.iter_mut().find(|(ty, _)| *ty == source) {
            Some((_, items)) => items.push(item),
            None => groups.push((source, vec![item])),
        }
    }
    let mut groups: Vec<_> = groups
        .into_iter()
        .map(|(source, items)| {
            let name = conversion_type_name(source, language);
            let title = crate::i18n::catalogs()
                .format(
                    language,
                    "conversion-from-type",
                    &[("type", name.as_str().into())],
                )
                .map_or(name, std::borrow::Cow::into_owned);
            (title, items)
        })
        .collect();
    if search.trim().is_empty() {
        groups.sort_by(|a, b| a.0.cmp(&b.0));
    }
    groups
}

pub(crate) fn conversion_items(
    host: &Host,
    search: &str,
    language: &str,
    from: Option<&str>,
    to: Option<&str>,
) -> Vec<PaletteItem> {
    let conversions = host.conversions();
    matching_conversions(
        conversions.iter().filter(|conversion| {
            from.is_none_or(|ty| conversion.from == ty) && to.is_none_or(|ty| conversion.to == ty)
        }),
        search,
        language,
    )
}

pub(crate) fn matching_conversions<'a>(
    conversions: impl Iterator<Item = &'a orange_host::Conversion>,
    search: &str,
    language: &str,
) -> Vec<PaletteItem> {
    ranked_conversions(conversions, search, language)
        .into_iter()
        .map(|(_, item)| item)
        .collect()
}

fn ranked_conversions<'a>(
    conversions: impl Iterator<Item = &'a orange_host::Conversion>,
    search: &str,
    language: &str,
) -> Vec<(&'a orange_host::Conversion, PaletteItem)> {
    let wanted = orange_host::SearchQuery::new(search);
    if !wanted.is_valid() {
        return Vec::new();
    }
    if wanted.is_empty() {
        return conversions
            .map(|conversion| (conversion, conversion_item(conversion, language)))
            .collect();
    }
    let keywords = text_in(language, "conversion-search-words", "");
    let other_keywords =
        orange_host::SEARCH_LANGUAGES.map(|tag| text_in(tag, "conversion-search-words", ""));
    let legacy_utf8 = [
        "conversion-utf8-decode-search",
        "conversion-utf8-replace-search",
    ]
    .map(|key| {
        (
            text_in(language, key, ""),
            orange_host::SEARCH_LANGUAGES.map(|tag| text_in(tag, key, "")),
        )
    });
    let mut found: Vec<_> = conversions
        .filter_map(|conversion| {
            let item = conversion_item(conversion, language);
            let aliases = conversion_words(&item.id);
            let legacy = match conversion.id() {
                "core/bytes-to-string" => Some(&legacy_utf8[0]),
                "core/bytes-to-string-lossy" => Some(&legacy_utf8[1]),
                _ => None,
            };
            let active = [
                item.label.as_str(),
                item.purpose.as_str(),
                keywords.as_str(),
                legacy.map_or("", |(active, _)| active.as_str()),
            ];
            let other_labels =
                orange_host::SEARCH_LANGUAGES.map(|tag| conversion_label_in(conversion, tag));
            let legacy_fallback = legacy.map(|(_, fallback)| fallback).into_iter().flatten();
            let fallback = other_labels
                .iter()
                .chain(&other_keywords)
                .chain(legacy_fallback)
                .map(String::as_str)
                .chain([aliases.as_str()]);
            let rank = wanted.rank(active, fallback)?;
            Some((rank, conversion, item))
        })
        .collect();
    found.sort_by_key(|(rank, _, _)| *rank);
    found
        .into_iter()
        .map(|(_, conversion, item)| (conversion, item))
        .collect()
}

pub(crate) const BRIDGE_OUT: &str = intrinsic::BRIDGE_OUT;
pub(crate) const BRIDGE_IN: &str = intrinsic::BRIDGE_IN;

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
        intrinsic::EMIT => true,
        intrinsic::ON_EVENT => false,
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

/// The family a node kind is drawn in: its badge glyph, its color and the message that
/// explains the family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NodeFamily {
    Conversions,
    Signals,
    Triggers,
    Functions,
    Logic,
    Loops,
    State,
    Math,
    Text,
    Time,
    Audio,
    Network,
    Files,
    Log,
    Data,
    Flow,
    /// A kind from a package without a family of its own.
    Package,
}

impl NodeFamily {
    pub(crate) fn of(definition: &str) -> Self {
        if definition == intrinsic::CONVERT || definition.starts_with(CONVERSION) {
            return Self::Conversions;
        }
        let (namespace, name) = definition.split_once('/').unwrap_or((definition, ""));
        match (namespace, name) {
            ("core", "emit" | "on-event") | (preset::SIGNAL | preset::WATCHDOG, _) => Self::Signals,
            ("core", name) if name.starts_with("on-") => Self::Triggers,
            ("core", "call" | "function-entry" | "function-outputs" | "return") => Self::Functions,
            ("core", "and" | "or" | "all-conditions" | "any-conditions")
            | ("org.orange.math", "not" | "xor") => Self::Logic,
            ("core", name) if loop_name(name) => Self::Loops,
            ("core", name) if name.contains("variable") || name.contains("shared") => Self::State,
            ("core", "increment-counter" | "decrement-counter") => Self::State,
            ("core", "number" | "whole-number") | ("org.orange.math", _) => Self::Math,
            ("core", "text") | ("org.orange.text", _) => Self::Text,
            ("core", "wait" | "now") | ("org.orange.time" | "org.orange.clock", _) => Self::Time,
            ("org.orange.audio", _) => Self::Audio,
            ("org.orange.net", _) => Self::Network,
            ("org.orange.files", _) => Self::Files,
            ("org.orange.logging", _) => Self::Log,
            ("org.orange.json" | "org.orange.collections", _) => Self::Data,
            (
                "core"
                | preset::GUARD
                | preset::RISING_EDGE
                | preset::FALLING_EDGE
                | preset::DEADBAND,
                _,
            ) => Self::Flow,
            _ => Self::Package,
        }
    }

    pub(crate) fn glyph(self) -> char {
        use crate::icons;
        match self {
            Self::Conversions | Self::Loops => icons::REFRESH,
            Self::Signals => icons::SIGNAL,
            Self::Triggers => icons::FLOW,
            Self::Functions => icons::FUNCTION,
            Self::Logic => icons::BOOLEAN,
            Self::State | Self::Data => icons::VARIABLE,
            Self::Math => icons::MATH,
            Self::Text => icons::TEXT,
            Self::Time => icons::CLOCK,
            Self::Audio => icons::AUDIO,
            Self::Network => icons::NETWORK,
            Self::Files => icons::OPEN,
            Self::Log => icons::LOG,
            Self::Flow => icons::LOGIC,
            Self::Package => icons::PACKAGES,
        }
    }

    pub(crate) fn color(self) -> egui::Color32 {
        let [r, g, b] = match self {
            Self::Conversions | Self::Time => [74, 152, 165],
            Self::Signals => [67, 152, 193],
            Self::Triggers => [205, 124, 58],
            Self::Functions => [137, 114, 207],
            Self::Logic => [196, 160, 52],
            Self::Loops => [51, 151, 163],
            Self::State => [60, 156, 112],
            Self::Math => [64, 145, 213],
            Self::Text => [183, 110, 174],
            Self::Audio => [151, 119, 213],
            Self::Network => [56, 150, 189],
            Self::Files => [190, 144, 65],
            Self::Log => [172, 129, 74],
            Self::Data => [62, 152, 130],
            Self::Flow => [211, 131, 66],
            Self::Package => [132, 132, 143],
        };
        egui::Color32::from_rgb(r, g, b)
    }

    /// The catalogue message that explains this family.
    pub(crate) fn help_key(self) -> &'static str {
        match self {
            Self::Conversions => "catalog-conversions-help",
            Self::Signals | Self::Triggers => "node-family-events",
            Self::Functions => "node-family-functions",
            Self::Logic => "node-family-logic",
            Self::Loops => "node-family-loops",
            Self::State => "node-family-state",
            Self::Math => "node-family-math",
            Self::Text => "node-family-text",
            Self::Time => "node-family-time",
            Self::Audio => "node-family-audio",
            Self::Network => "node-family-network",
            Self::Files => "node-family-files",
            Self::Log => "node-family-log",
            Self::Data => "node-family-data",
            Self::Flow => "node-family-flow",
            Self::Package => "node-family-package",
        }
    }
}

/// A node kind's badge: its family's glyph, color and explaining message.
pub(crate) fn badge(definition: &str) -> (char, egui::Color32, &'static str) {
    let family = NodeFamily::of(definition);
    (family.glyph(), family.color(), family.help_key())
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

const LOOPS: CatalogGroup = CatalogGroup {
    key: "palette-loops",
    fallback: "Loops",
    representative: intrinsic::REPEAT_END,
    help: "node-family-loops",
};
const VARIABLES: CatalogGroup = CatalogGroup {
    key: "variables",
    fallback: "Variables",
    representative: intrinsic::GET_VARIABLE,
    help: "node-family-state",
};

const FLOW: CatalogGroup = CatalogGroup {
    key: "catalog-control",
    fallback: "Control flow",
    representative: intrinsic::BRANCH,
    help: "catalog-flow-help",
};
const CALCULATE: CatalogGroup = CatalogGroup {
    key: "catalog-math",
    fallback: "Math & logic",
    representative: "org.orange.math/add",
    help: "catalog-math-help",
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

fn catalog_group_rank(definition: &str) -> usize {
    match catalog_group(definition).map(|group| group.key) {
        Some("catalog-triggers") => 0,
        Some("catalog-control") => 1,
        Some("palette-loops") => 2,
        _ => 3,
    }
}

const TRIGGERS: CatalogGroup = CatalogGroup {
    key: "catalog-triggers",
    fallback: "Triggers & signals",
    representative: intrinsic::ON_EVENT,
    help: "node-family-events",
};

fn domain_group(
    key: &'static str,
    fallback: &'static str,
    representative: &'static str,
    help: &'static str,
) -> CatalogGroup {
    CatalogGroup {
        key,
        fallback,
        representative,
        help,
    }
}

fn catalog_group(definition: &str) -> Option<CatalogGroup> {
    let (namespace, name) = definition.split_once('/').unwrap_or((definition, ""));
    Some(match (namespace, name) {
        ("org.orange.audio", _) => AUDIO,
        ("org.orange.input", "on-key-pressed") => TRIGGERS,
        ("org.orange.input", _) => domain_group(
            "catalog-input",
            "Input",
            "org.orange.input/on-key-pressed",
            "catalog-tools-help",
        ),
        ("org.orange.files", _) => domain_group(
            "catalog-files",
            "Files",
            "org.orange.files/read-text",
            "node-family-files",
        ),
        ("org.orange.net", _) => domain_group(
            "catalog-network",
            "Network",
            "org.orange.net/http-request",
            "node-family-network",
        ),
        ("org.orange.math", _) => CALCULATE,
        ("org.orange.json", _) => domain_group(
            "catalog-json",
            "JSON",
            "org.orange.json/get",
            "node-family-data",
        ),
        ("org.orange.collections", _) => domain_group(
            "catalog-collections",
            "Collections",
            "org.orange.collections/make-enum",
            "node-family-data",
        ),
        ("org.orange.text", _) => domain_group(
            "catalog-text",
            "Text",
            "org.orange.text/concat",
            "node-family-text",
        ),
        ("org.orange.color", _) => domain_group(
            "catalog-color",
            "Color",
            "org.orange.color/rgba",
            "catalog-data-help",
        ),
        ("org.orange.time" | "org.orange.clock", _) | ("core", "wait" | "now") => {
            domain_group("catalog-time", "Time", intrinsic::WAIT, "node-family-time")
        }
        ("org.orange.logging", _) => domain_group(
            "catalog-logging",
            "Logging",
            "org.orange.logging/log-message",
            "node-family-log",
        ),
        ("core", "number" | "whole-number" | "bool" | "text" | "value" | "true-or-false") => {
            domain_group(
                "catalog-values",
                "Values",
                intrinsic::WHOLE_NUMBER,
                "node-family-data",
            )
        }
        ("core", name) if name.contains("variable") || name.contains("shared") => VARIABLES,
        ("core", "increment-counter" | "decrement-counter") => VARIABLES,
        ("core", "call" | "return" | "function-entry" | "function-outputs") => domain_group(
            "catalog-functions",
            "Functions",
            intrinsic::CALL,
            "node-family-functions",
        ),
        ("core", name) if name.starts_with("on-") => TRIGGERS,
        ("core", "emit" | "await-event" | "bridge-in" | "bridge-out") | (preset::SIGNAL, _) => {
            TRIGGERS
        }
        ("core", name) if loop_name(name) => LOOPS,
        ("core", "rising-edge" | "falling-edge" | "change-beyond-tolerance") => TRIGGERS,
        ("core", "and" | "or" | "all-conditions" | "any-conditions") => CALCULATE,
        ("core", "register" | "close-handle" | "close-stream" | "next-item") => TOOLS,
        ("core", _) => FLOW,
        (id, _) if preset::is_preset(id) => FLOW,
        _ => return None,
    })
}

pub(crate) fn catalog_section(definition: &str) -> Option<(&'static str, &'static str)> {
    if definition.starts_with(CONVERSION) {
        return None;
    }
    if let Some(section) = audio_section(definition) {
        return Some(section);
    }
    match definition {
        intrinsic::EMIT | intrinsic::ON_EVENT | intrinsic::BRIDGE_OUT | intrinsic::BRIDGE_IN => {
            return Some(("catalog-wireless", "catalog-wireless-help"));
        }
        intrinsic::REPEAT_END
        | intrinsic::MAP_END
        | intrinsic::FILTER_END
        | intrinsic::FOLD_END => {
            return Some(("catalog-loop-ends", "catalog-loop-ends-help"));
        }
        _ => {}
    }
    if let Some((key, _, _)) = family(definition) {
        match key {
            "palette-http" => return Some((key, "node-family-network")),
            "palette-json-convert" => return Some((key, "node-family-data")),
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
            preset::SIGNAL
            | preset::WIRELESS_VALUE
            | preset::WIRELESS_PULSE
            | preset::RISING_EDGE
            | preset::FALLING_EDGE
            | preset::DEADBAND,
            _,
        ) => ("catalog-triggers", "node-family-events"),
        ("core", "call" | "return" | "function-entry" | "function-outputs") => {
            ("catalog-functions", "node-family-functions")
        }
        ("core", name) if name.contains("variable") || name.contains("shared") => {
            ("variables", "node-family-state")
        }
        ("core", _) | (preset::GUARD, _) => match catalog_group(definition)?.key {
            "catalog-math" => ("catalog-logic", "node-family-math"),
            _ => {
                let group = catalog_group(definition)?;
                (group.key, group.help)
            }
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
        | intrinsic::REGISTER
        | intrinsic::CLOSE_HANDLE
        | intrinsic::CLOSE_STREAM
        | intrinsic::NEXT_ITEM
        | intrinsic::MAP_END
        | intrinsic::FILTER_END
        | intrinsic::FOLD_END
        | intrinsic::REPEAT_END => ("palette-advanced", "Advanced", "advanced avanzado"),
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
    let query = orange_host::SearchQuery::new(search);
    if !query.is_valid() {
        return Vec::new();
    }
    let mut nodes = host.ranked_search_nodes(search, language);
    if query.is_empty() {
        return nodes.into_iter().map(|(node, _)| node).collect();
    }
    for mut found in host.search_nodes("", language) {
        if nodes
            .iter()
            .any(|(node, _)| node.definition == found.definition)
        {
            continue;
        }
        let alias = family(&found.definition).and_then(|(key, label, words)| {
            let caption = text_in(language, key, label);
            query.rank([words, caption.as_str()], [])
        });
        let catalog = catalog_group(&found.definition).and_then(|group| {
            let section_key = catalog_section(&found.definition).map(|(key, _)| key);
            let section = section_key.map_or_else(String::new, |key| text_in(language, key, ""));
            let other_sections = section_key.map_or_else(Vec::new, |key| {
                orange_host::SEARCH_LANGUAGES
                    .map(|tag| text_in(tag, key, ""))
                    .to_vec()
            });
            let localized = group.name(language);
            query.rank(
                [localized.as_str(), found.label.as_str(), section.as_str()],
                [group.fallback]
                    .into_iter()
                    .chain(other_sections.iter().map(String::as_str)),
            )
        });
        if let Some(rank) = [alias, catalog].into_iter().flatten().min() {
            // Found through its family or catalogue group, not its own words.
            let rank = rank.no_stronger_than(orange_host::Match::Concept);
            found.matched = Some(rank.kind());
            nodes.push((found, Some(rank)));
        }
    }
    nodes.sort_by_key(|(_, rank)| *rank);
    nodes.into_iter().map(|(found, _)| found).collect()
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
    if matches!(
        converter,
        "core/bytes-to-string" | "core/bytes-to-string-lossy"
    ) {
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
    conversion_label_in(conversion, &crate::i18n::language())
}

fn conversion_type_name(ty: &str, language: &str) -> String {
    match ty {
        "core/string@1" => text_in(language, "type-text", "Text"),
        "core/bytes@1" => text_in(language, "type-bytes", "Bytes"),
        "core/bool@1" => text_in(language, "type-bool", "Bool"),
        "core/duration@1" => text_in(language, "type-duration", "Duration"),
        "core/timestamp@1" => text_in(language, "type-timestamp", "Timestamp"),
        "core/json@1" => "JSON".to_owned(),
        orange_host::RGBA_TYPE => text_in(language, "type-rgba", "Color"),
        _ => ty
            .trim_start_matches("core/")
            .strip_suffix("@1")
            .unwrap_or(ty)
            .trim_start_matches("core/")
            .to_owned(),
    }
}

fn conversion_label_in(conversion: &orange_host::Conversion, language: &str) -> String {
    converter_title(
        conversion.id(),
        &conversion.from,
        &conversion.to,
        conversion.options.values().map(String::as_str),
        language,
    )
}

fn converter_title<'a>(
    id: &str,
    from: &str,
    to: &str,
    options: impl Iterator<Item = &'a str>,
    language: &str,
) -> String {
    let from = conversion_type_name(from, language);
    let to = conversion_type_name(to, language);
    let mut label = crate::i18n::catalogs()
        .format(
            language,
            "conversion-title",
            &[("from", from.as_str().into()), ("to", to.as_str().into())],
        )
        .map_or_else(|| format!("{from} → {to}"), std::borrow::Cow::into_owned);
    let encoding = match id {
        "core/string-to-bytes" | "core/bytes-to-string" => Some("conversion-utf8"),
        "core/bytes-to-string-lossy" => Some("conversion-utf8-replace"),
        _ => None,
    };
    let choices: Vec<_> = encoding
        .into_iter()
        .map(|key| text_in(language, key, "UTF-8"))
        .chain(
            options
                .filter(|value| id != "core/bytes-to-string-lossy" || *value != "replace")
                .map(|value| {
                    let key = match value {
                        "seconds" => "conversion-unit-seconds",
                        "milliseconds" => "conversion-unit-milliseconds",
                        "microseconds" => "conversion-unit-microseconds",
                        "nanoseconds" => "conversion-unit-nanoseconds",
                        "minutes" => "conversion-unit-minutes",
                        "hours" => "conversion-unit-hours",
                        "truncate" => "conversion-round-truncate",
                        "floor" => "conversion-round-floor",
                        "ceiling" => "conversion-round-ceiling",
                        "nearest-even" => "conversion-round-nearest",
                        "reject" => "conversion-overflow-reject",
                        other => other,
                    };
                    text_in(language, key, value)
                }),
        )
        .collect();
    if !choices.is_empty() {
        label.push_str(&format!(" ({})", choices.join(", ")));
    }
    label
}

/// The placed card uses the same title and saved policy labels as the picker.
pub(crate) fn converter_card_title(node: &NodeView, language: &str) -> Option<String> {
    let ty = |input, id| {
        node.ports
            .iter()
            .find(|port| port.input == input && !port.exec && port.id == id)?
            .ty
            .as_deref()
    };
    let options = node.settings["options"]
        .as_object()
        .into_iter()
        .flat_map(|options| options.values().filter_map(serde_json::Value::as_str));
    Some(converter_title(
        node.settings["converter"].as_str().unwrap_or_default(),
        ty(true, "value")?,
        ty(false, "result")?,
        options,
        language,
    ))
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
        label: conversion_label_in(conversion, language),
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
    fn core_kinds_and_presets_always_have_a_family_and_packages_share_one() {
        for definition in [
            intrinsic::EMIT,
            intrinsic::WAIT,
            intrinsic::BRANCH,
            "core/some-future-kind",
            preset::GUARD,
            preset::SIGNAL,
        ] {
            assert_ne!(
                NodeFamily::of(definition),
                NodeFamily::Package,
                "{definition}"
            );
        }
        assert_eq!(NodeFamily::of("com.example.tools/x"), NodeFamily::Package);
        assert_eq!(NodeFamily::of(intrinsic::ON_START), NodeFamily::Triggers);
        assert_eq!(
            badge(intrinsic::CONVERT),
            (
                crate::icons::REFRESH,
                NodeFamily::Time.color(),
                "catalog-conversions-help"
            )
        );
    }

    #[test]
    fn specialized_nodes_are_listed_once_in_their_node_families() {
        let (host, _) = Host::new(HostOptions::default()).unwrap();
        for (query, id, group) in [
            ("rising edge", intrinsic::RISING_EDGE, TRIGGERS),
            ("falling edge", intrinsic::FALLING_EDGE, TRIGGERS),
            ("deadband", intrinsic::CHANGE_BEYOND_TOLERANCE, TRIGGERS),
            ("bump+", intrinsic::INCREMENT_COUNTER, VARIABLES),
            ("bump -", intrinsic::DECREMENT_COUNTER, VARIABLES),
        ] {
            for search in ["", query] {
                let groups = palette_in(&host, search, "en");
                let entries: Vec<_> = groups
                    .iter()
                    .flat_map(|(name, items)| items.iter().map(move |item| (name, item)))
                    .filter(|(_, item)| item.id == id)
                    .collect();
                assert_eq!(entries.len(), 1, "{search:?}: {id}");
                assert_eq!(*entries[0].0, group.name("en"));
                assert!(
                    groups
                        .iter()
                        .flat_map(|(_, items)| items)
                        .all(|item| !preset::is_preset(&item.id))
                );
            }
        }
        for (language, old_name) in [("es-MX", "Banda muerta"), ("ja-JP", "不感帯")] {
            assert!(
                discover(&host, old_name, language)
                    .iter()
                    .any(|node| node.definition == intrinsic::CHANGE_BEYOND_TOLERANCE)
            );
        }
    }

    #[test]
    fn every_converter_preview_has_the_ports_and_policy_used_for_placement() {
        let (host, _) = Host::new(HostOptions::default()).unwrap();
        let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
        let before = draft.text().unwrap();
        let mut previews = PreviewCache::default();
        let conversions = host.conversions();
        for conversion in &conversions {
            let id = conversion_id(conversion);
            let card = previews
                .card(&host, Some(&draft), &id)
                .unwrap_or_else(|| panic!("{id}"));
            for (id, input, ty) in [
                ("value", true, conversion.from.as_str()),
                ("result", false, conversion.to.as_str()),
            ] {
                assert!(card.ports.iter().any(|port| port.id == id
                    && port.input == input
                    && port.ty.as_deref() == Some(ty)));
            }
            assert_eq!(
                card.ports.iter().any(|port| port.id == "loss"),
                conversion.lossy
            );
            assert_eq!(
                card.settings["options"],
                serde_json::to_value(&conversion.options).unwrap()
            );
            for language in ["en-US", "es-MX", "ja-JP"] {
                assert_eq!(
                    converter_card_title(card, language).unwrap(),
                    conversion_label_in(conversion, language)
                );
            }
        }
        assert_eq!(draft.text().unwrap(), before);
        assert_eq!(draft.can_undo(), (false, false));

        let conversion = conversions
            .iter()
            .find(|item| item.lossy && !item.options.is_empty())
            .unwrap();
        let id = conversion_id(conversion);
        let card = previews.card(&host, None, &id).unwrap();
        let node = host
            .add_conversion(&mut draft, conversion, [0.0, 0.0])
            .unwrap();
        let view = host.draft_view(&draft).unwrap();
        let placed = view.nodes.iter().find(|item| item.id == node).unwrap();
        assert_eq!(card.settings, placed.settings);
        let ctx = egui::Context::default();
        crate::icons::install(&ctx);
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(300.0);
            result_row(
                ui,
                &conversion_label_in(conversion, "en"),
                "",
                None,
                false,
                Some(card),
                Some(&id),
            );
        });
        for port in &card.ports {
            let label = port
                .label
                .clone()
                .unwrap_or_else(|| crate::port_label(&port.id));
            assert!(
                output.shapes.iter().any(|shape| {
                    matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == label)
                }),
                "missing converter port {label}"
            );
        }
        output.drop_without_applying_deltas();
    }

    #[test]
    fn conversion_groups_have_a_single_source_type() {
        let (host, _) = Host::new(HostOptions::default()).unwrap();
        let conversions: HashMap<_, _> = host
            .conversions()
            .into_iter()
            .map(|conversion| (conversion_id(&conversion), conversion.from))
            .collect();
        let groups = conversion_groups(&host, "", "es-MX");
        assert!(groups.len() > 1);
        assert_eq!(
            groups.iter().map(|(_, items)| items.len()).sum::<usize>(),
            conversions.len()
        );
        for (title, items) in groups {
            assert!(title.starts_with("Desde "), "{title}");
            let from = &conversions[&items[0].id];
            assert!(items.iter().all(|item| &conversions[&item.id] == from));
        }
    }

    #[test]
    fn converter_names_identify_both_types_and_legacy_names_still_find_them() {
        let (host, _) = Host::new(HostOptions::default()).unwrap();
        for (language, old, name, id) in [
            (
                "en",
                "JSON as text",
                "JSON to Optional Text",
                "org.orange.json/as-string",
            ),
            (
                "es",
                "Como entero",
                "JSON a i64 opcional",
                "org.orange.json/as-i64",
            ),
            (
                "ja",
                "JSON を小数に",
                "JSON からオプションの f64 へ",
                "org.orange.json/as-f64",
            ),
            (
                "en",
                "Decode Base64",
                "Base64 Text to Bytes",
                "org.orange.text/decode-base64",
            ),
        ] {
            for query in [old, name] {
                assert!(
                    discover(&host, query, language)
                        .iter()
                        .any(|node| node.definition == id && node.label == name),
                    "{language}: {query}"
                );
            }
        }
        let choices = conversion_items(&host, "decode UTF-8", "en", Some("core/bytes@1"), None);
        let names: Vec<_> = choices
            .iter()
            .map(|item| orange_host::fold_search(&item.label))
            .collect();
        assert!(names.contains(&"bytes to text (utf 8)".to_owned()));
        assert!(names.contains(&"bytes to text (utf 8 with replacement)".to_owned()));
        let legacy = conversion_items(
            &host,
            "replace invalid bytes",
            "en",
            Some("core/bytes@1"),
            None,
        );
        assert_eq!(legacy.len(), 1);
        assert!(
            legacy[0]
                .id
                .starts_with("convert:core/bytes-to-string-lossy")
        );
    }

    #[test]
    fn loops_use_their_node_names_and_wireless_senders_precede_readers() {
        let (host, _) = Host::new(HostOptions::default()).unwrap();
        for locale in ["en-US", "es-MX", "ja-JP"] {
            let shown = palette_in(&host, "", locale);
            let loops = &shown
                .iter()
                .find(|(name, _)| *name == LOOPS.name(locale))
                .unwrap()
                .1;
            for id in [
                intrinsic::REPEAT_BEGIN,
                intrinsic::MAP_BEGIN,
                intrinsic::FILTER_BEGIN,
                intrinsic::FOLD_BEGIN,
            ] {
                let item = loops.iter().find(|item| item.id == id).unwrap();
                assert!(
                    host.search_nodes(&item.label, locale)
                        .iter()
                        .any(|found| found.definition == id && found.label == item.label),
                    "{locale}: {}",
                    item.label
                );
            }
            let signals = &shown
                .iter()
                .find(|(name, _)| *name == TRIGGERS.name(locale))
                .unwrap()
                .1;
            let wireless: Vec<_> = signals
                .iter()
                .map(|item| item.id.as_str())
                .filter(|id| {
                    [intrinsic::EMIT, intrinsic::ON_EVENT, BRIDGE_OUT, BRIDGE_IN].contains(id)
                })
                .collect();
            assert_eq!(
                wireless,
                [intrinsic::EMIT, intrinsic::ON_EVENT, BRIDGE_OUT, BRIDGE_IN]
            );
        }
    }

    #[test]
    fn palette_cache_refreshes_after_a_catalog_edit_with_the_same_query() {
        let (mut host, _) = Host::new(HostOptions::default()).unwrap();
        let mut cache = PaletteCache::default();
        let query = "zqxword";
        let generation = host.registry().generation();
        assert!(cache.get(&host, query, "en-US", &[]).groups.is_empty());
        host.add_catalog("en", "alias-core-wait = zqxword").unwrap();
        assert_eq!(host.registry().generation(), generation);
        assert!(
            cache
                .get(&host, query, "en-US", &[])
                .groups
                .iter()
                .flat_map(|(_, items)| items)
                .any(|item| item.id == "core/wait")
        );
    }

    #[test]
    fn palette_keeps_the_preferred_name_prefix_on_top() {
        let (host, _) = Host::new(HostOptions::default()).unwrap();
        for (query, expected) in [
            ("int", "core/try"),
            ("obtener campo", "org.orange.json/get"),
        ] {
            let groups = palette_in(&host, query, "es-MX");
            assert_eq!(groups[0].1[0].id, expected, "{query}");
        }
    }

    #[test]
    fn the_catalog_has_no_templates_and_names_conversions_unambiguously() {
        let (host, _) = Host::new(HostOptions::default()).unwrap();
        for (language, templates) in [("en", "Templates"), ("es", "Plantillas")] {
            let groups = palette_in(&host, "", language);
            assert!(groups.iter().all(|(name, _)| name != templates));
            let mut labels = std::collections::HashSet::new();
            for item in groups
                .iter()
                .flat_map(|(_, items)| items)
                .filter(|item| item.id.starts_with(CONVERSION))
            {
                assert!(
                    labels.insert(item.label.as_str()),
                    "{language}: {}",
                    item.label
                );
            }
        }
        let result = palette_in(&host, "coalesce", "en");
        assert!(
            result
                .iter()
                .flat_map(|(_, items)| items)
                .any(|item| item.id == intrinsic::COALESCE && item.label == "Value or Fallback")
        );
    }

    #[test]
    fn counter_cards_require_explicit_binding_and_reopen_with_only_useful_ports() {
        let (mut host, _) = Host::new(HostOptions::default()).unwrap();
        let mut editor = crate::Editor::new();
        editor.open_untitled(&mut host, orange_host::new_document().unwrap());
        for (id, legacy) in [
            (intrinsic::INCREMENT_COUNTER, preset::INCREMENT),
            (intrinsic::DECREMENT_COUNTER, preset::DECREMENT),
        ] {
            let node = editor.add_node_at(&mut host, legacy, [0.0, 0.0]).unwrap();
            assert!(editor.declarations().unwrap().variables.is_empty());
            let variable = |ty: &str, initializer: &str| orange_host::Variable {
                id: None,
                name: "Points".into(),
                ty: ty.into(),
                scope: orange_host::VarScope::Run,
                persistent: false,
                initial: Some(initializer.into()),
            };
            assert!(
                editor
                    .declare_variable_for(
                        &mut host,
                        node,
                        "/variable",
                        &variable("core/string@1", "text")
                    )
                    .is_none()
            );
            let counter = editor
                .declare_variable_for(&mut host, node, "/variable", &variable("core/i64@1", "0"))
                .unwrap();
            let saved = editor.text().unwrap();
            editor.open_untitled(&mut host, saved);
            let graph = editor.graph().unwrap();
            let card = graph.nodes.iter().find(|card| card.id == node).unwrap();
            assert_eq!(card.definition, id);
            assert_eq!(card.settings, serde_json::json!({"variable":counter}));
            let data: Vec<_> = card
                .ports
                .iter()
                .filter(|port| !port.exec && port.id != "error")
                .collect();
            assert_eq!(
                data.iter().map(|port| port.id.as_str()).collect::<Vec<_>>(),
                ["value", "new"]
            );
            assert_eq!(data[0].default.as_deref(), Some("1"));
            assert_eq!(crate::properties::input_caption(card, data[0]), "Step");
            editor.open_untitled(&mut host, orange_host::new_document().unwrap());
        }
    }

    /// Searching for a subscriber offers each channel a Dispatch sends on, bound in
    /// one step; naming the channel finds it too, and an empty search lists nothing.
    #[test]
    fn event_palette_offers_roles_without_duplicate_pair_presets() {
        let (host, _) = Host::new(HostOptions::default()).unwrap();
        for (language, query) in [("en", ""), ("en", "dispatch"), ("es", "escuchar")] {
            let found = discover(&host, query, language);
            assert!(!found.iter().any(|item| matches!(
                item.definition.as_str(),
                preset::SIGNAL | preset::WIRELESS_PULSE
            )));
            if query.is_empty() {
                for id in [intrinsic::EMIT, intrinsic::ON_EVENT] {
                    assert_eq!(found.iter().filter(|item| item.definition == id).count(), 1);
                }
            }
        }
    }

    #[test]
    fn listen_searches_offer_existing_channels_to_bind() {
        let channels = [
            ("e1".to_owned(), "Kick".to_owned()),
            ("e2".to_owned(), "Door bell".to_owned()),
        ];
        let found = channel_items(&channels, "listen", true, "Listen", "en-US");
        assert_eq!(
            found
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            [format!("{BIND_LISTEN}e1"), format!("{BIND_LISTEN}e2")]
        );
        assert_eq!(found[0].label, "Listen · Kick");
        let named = channel_items(&channels, "door", false, "Listen", "en-US");
        assert_eq!(named.len(), 1);
        assert_eq!(named[0].id, format!("{BIND_LISTEN}e2"));
        assert_eq!(
            channel_items(&channels, "door listen", true, "Listen", "en-US"),
            named
        );
        assert!(channel_items(&channels, "", true, "Listen", "en-US").is_empty());
        let localized = channel_items(&channels, "受信 door", false, "イベントを受信", "ja-JP");
        assert_eq!(localized.len(), 1);
        assert!(localized[0].purpose.contains("チャンネル"));
        let channels = [
            ("e1".to_owned(), "Canal".to_owned()),
            ("e2".to_owned(), "Other".to_owned()),
        ];
        for query in ["canal", "listen canal"] {
            let found = channel_items(&channels, query, true, "Listen", "en-US");
            assert_eq!(found.len(), 1, "{query}");
            assert_eq!(found[0].id, format!("{BIND_LISTEN}e1"));
        }
    }

    #[test]
    fn a_fresh_dispatch_can_add_a_bound_subscriber_and_rename_its_channel() {
        let (mut host, _) = Host::new(HostOptions::default()).unwrap();
        let mut editor = crate::Editor::new();
        editor.open_untitled(&mut host, orange_host::new_document().unwrap());
        let dispatch = editor
            .add_node_at(&mut host, intrinsic::EMIT, [0.0, 0.0])
            .unwrap();
        editor.open_menu(
            crate::MenuTarget::Node(dispatch),
            [0.0, 0.0],
            egui::Pos2::ZERO,
        );
        assert!(editor.run_command(&mut host, crate::Command::AddSignalReceiver));
        let graph = editor.graph().unwrap();
        assert_eq!(graph.nodes.len(), 2);
        let emitter = graph.nodes.iter().find(|node| node.id == dispatch).unwrap();
        let (_, id) = signal(emitter).unwrap();
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| signal(node) == Some((false, id)))
        );
        editor.open_menu(
            crate::MenuTarget::Node(dispatch),
            [0.0, 0.0],
            egui::Pos2::ZERO,
        );
        assert!(editor.run_command(&mut host, crate::Command::RenameEvent));
    }

    #[test]
    fn wireless_signal_shares_one_event_with_multiple_receivers() {
        let (mut host, _) = Host::new(HostOptions::default()).unwrap();
        let mut editor = crate::Editor::new();
        editor.open_untitled(&mut host, orange_host::new_document().unwrap());
        assert!(
            discover(&host, "senal inalambrica", "es")
                .iter()
                .any(|r| r.definition == intrinsic::EMIT)
        );
        let sender = editor
            .add_node_at(&mut host, preset::SIGNAL, [0.0, 0.0])
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
                [intrinsic::ON_MANUAL, intrinsic::ON_EVENT, preset::SIGNAL],
                "Triggers & signals",
            ),
            (
                [intrinsic::BRANCH, intrinsic::SEQUENCE, preset::GUARD],
                "Control flow",
            ),
            (
                [
                    intrinsic::GET_VARIABLE,
                    intrinsic::SET_VARIABLE,
                    intrinsic::ON_VARIABLE_CHANGED,
                ],
                "Variables",
            ),
            (
                [
                    "org.orange.net/http-request-json",
                    "org.orange.net/http-request",
                    "org.orange.net/websocket",
                ],
                "Network",
            ),
        ] {
            for node in nodes {
                assert_eq!(catalog_group(node).unwrap().name("en"), expected);
            }
        }
        assert_eq!(
            catalog_section(intrinsic::FOR_EACH).unwrap().0,
            "palette-loops"
        );
        assert_eq!(
            catalog_section("org.orange.net/http-request").unwrap().0,
            "palette-http"
        );
        let (host, _) = Host::new(HostOptions::default()).unwrap();
        assert!(
            palette_in(&host, "entero", "es")
                .iter()
                .any(|(group, items)| {
                    group == "Valores"
                        && items.iter().any(|item| item.id == intrinsic::WHOLE_NUMBER)
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
        let (mut host, _) = Host::new(HostOptions::default()).unwrap();
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
        let (host, _) = Host::new(HostOptions::default()).unwrap();
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
                    .any(|node| node.definition == preset::WIRELESS_VALUE)
            );
        }
    }

    #[test]
    fn watchdog_is_absent_from_pickers_but_can_still_be_placed_by_its_id() {
        let (mut host, _) = Host::new(HostOptions::default()).unwrap();
        assert!(
            !discover(&host, "detector de inactividad", "es")
                .iter()
                .any(|node| node.definition == preset::WATCHDOG)
        );
        let mut editor = crate::Editor::new();
        editor.open_untitled(&mut host, orange_host::new_document().unwrap());
        editor.open_quick_add(None, [0.0, 0.0], egui::Pos2::ZERO);
        assert!(
            !editor
                .quick_choices(&host, "heartbeat timeout")
                .into_iter()
                .any(|choice| choice.definition == preset::WATCHDOG)
        );
        let emitter = editor
            .add_node_at(&mut host, preset::WATCHDOG, [0.0, 0.0])
            .unwrap();
        let draft = &editor.document.as_ref().unwrap().draft;
        let doc = draft.document();
        let body = doc.root();
        assert_eq!(doc.events().len(), 1);
        assert_eq!(body.nodes.len(), 3);
        assert_eq!(
            doc.events().values().next().unwrap().payload,
            None,
            "a heartbeat is an event-only channel"
        );
        let edge = body.edges.values().next().unwrap();
        let entry = &body.nodes[&edge.from.node];
        let wait = &body.nodes[&edge.to.node];
        assert_eq!(body.nodes[&emitter].definition.id.as_str(), intrinsic::EMIT);
        assert_eq!(entry.definition.id.as_str(), intrinsic::ON_EVENT);
        assert_eq!(
            serde_json::to_value(entry.activation.as_ref().unwrap()).unwrap()["mode"],
            "replace"
        );
        assert_eq!(wait.definition.id.as_str(), intrinsic::WAIT);
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
    fn guard_search_finds_the_branch_with_a_boolean_condition() {
        let (mut host, _) = Host::new(HostOptions::default()).unwrap();
        assert!(
            palette_in(&host, "guarda", "es")
                .iter()
                .flat_map(|(_, items)| items)
                .any(|item| item.id == intrinsic::BRANCH)
        );
        let mut editor = crate::Editor::new();
        editor.open_untitled(&mut host, orange_host::new_document().unwrap());
        editor.open_quick_add(None, [0.0, 0.0], egui::Pos2::ZERO);
        let choice = editor.quick_choices(&host, "guard").remove(0);
        assert_eq!(choice.definition, intrinsic::BRANCH);
        let node = editor.pick_quick(&mut host, &choice).unwrap();
        let graph = editor.graph().unwrap();
        let card = graph.nodes.iter().find(|card| card.id == node).unwrap();
        assert_eq!(card.definition, intrinsic::BRANCH);
        assert!(card.ports.iter().any(|port| port.input
            && port.id == "condition"
            && port.ty.as_deref() == Some("core/bool@1")));
    }

    #[test]
    fn specialized_triggers_place_with_minimal_ports_and_no_mode_setting() {
        let (mut host, _) = Host::new(HostOptions::default()).unwrap();
        let mut editor = crate::Editor::new();
        editor.open_untitled(&mut host, orange_host::new_document().unwrap());
        for (id, query) in [
            (intrinsic::RISING_EDGE, "rising edge"),
            (intrinsic::FALLING_EDGE, "falling edge"),
            (intrinsic::CHANGE_BEYOND_TOLERANCE, "deadband"),
        ] {
            editor.open_quick_add(None, [0.0, 0.0], egui::Pos2::ZERO);
            let choice = editor
                .quick_choices(&host, query)
                .into_iter()
                .find(|choice| choice.definition == id)
                .unwrap();
            let node = editor.pick_quick(&mut host, &choice).unwrap();
            let graph = editor.graph().unwrap();
            let card = graph.nodes.iter().find(|card| card.id == node).unwrap();
            assert_eq!(card.definition, id);
            if id == intrinsic::CHANGE_BEYOND_TOLERANCE {
                assert_eq!(card.settings, serde_json::json!({"threshold":1}));
                assert_eq!(
                    crate::properties::watch_field_label(card, "threshold"),
                    "Tolerance"
                );
                assert_eq!(crate::properties::watch_field_entries(card).count(), 1);
                assert!(
                    card.ports
                        .iter()
                        .any(|port| !port.input && port.id == "value")
                );
            } else {
                assert_eq!(card.settings, serde_json::json!({}));
                assert_eq!(card.ports.len(), 2);
                assert!(card.ports.iter().any(|port| port.input
                    && port.id == "watch"
                    && port.ty.as_deref() == Some("core/bool@1")));
            }
            editor.undo(&mut host);
            assert!(editor.graph().unwrap().nodes.is_empty());
        }
    }

    #[test]
    fn declaration_dependent_kinds_have_illustrative_ports() {
        let (host, _events) = Host::new(HostOptions::default()).unwrap();
        let mut cache = PreviewCache::default();
        for definition in [
            intrinsic::GET_VARIABLE,
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
        assert_eq!(cache.get(&host, "", "en", &[]).groups, palette(&host, ""));
        let allocation = cache.get(&host, "", "en", &[]).groups.as_ptr();
        assert_eq!(cache.get(&host, "", "en", &[]).groups.as_ptr(), allocation);
        for search in ["MATH add", "unmatched-xyz", ""] {
            assert_eq!(
                cache.get(&host, search, "en", &[]).groups,
                palette(&host, search)
            );
        }
        assert_eq!(
            cache.get(&host, "disparador", "es", &[]).groups,
            palette_in(&host, "disparador", "es")
        );
        assert!(!cache.get(&host, "disparador", "en", &[]).groups.is_empty());
        // Registry generations are process-unique, including when one editor is
        // reused with another host. Old provider labels must not survive that switch.
        let (core, _events) = Host::new(HostOptions {
            base_nodes: false,
            ..Default::default()
        })
        .unwrap();
        assert_ne!(core.registry().generation(), host.registry().generation());
        assert_ne!(palette(&core, ""), palette(&host, ""));
        assert_eq!(cache.get(&core, "", "en", &[]).groups, palette(&core, ""));
        assert_eq!(cache.get(&host, "", "en", &[]).groups, palette(&host, ""));
    }

    #[test]
    fn palette_preserves_locale_order_and_search_cannot_edit_the_document() {
        let (mut host, _) = Host::new(HostOptions::default()).unwrap();
        host.add_catalog("es", "alias-core-wait = capturamx")
            .unwrap();
        host.add_catalog("en", "alias-core-branch = capturamx")
            .unwrap();
        let mut editor = crate::Editor::new();
        editor.open_untitled(&mut host, orange_host::new_document().unwrap());
        let before = editor.text().unwrap();
        let found = palette_in(&host, "capturmax", "es-MX");
        let ids: Vec<_> = found
            .iter()
            .flat_map(|(_, items)| items.iter())
            .map(|item| item.id.as_str())
            .collect();
        assert_eq!(ids.first(), Some(&"core/wait"));
        assert!(ids.contains(&"core/branch"));
        for input in ["[{<>}]", "capturamx\0", &"x".repeat(4097)] {
            assert!(palette_in(&host, input, "es-MX").is_empty());
        }
        assert_eq!(editor.text().unwrap(), before);
    }
}
