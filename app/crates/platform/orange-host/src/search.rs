//! Finding node kinds by what a person types, like a shop search: by name, alias,
//! category, purpose ("concept") and description, in the active and bundled languages,
//! forgiving one typo. The words are catalogue data ([`Catalogs`]); which nodes a
//! concept groups follows from their kind and ID, so new trigger kinds join "trigger"
//! without a list to keep up.
use crate::intrinsic;
use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::{Arc, LazyLock};

use orange_model::document::NodeKind;

use crate::i18n::{CatalogError, Catalogs, primary_language};
use crate::{Host, NodeInfo};
use orange_model::catalog::TextRef;

mod matching;
pub use matching::{SearchQuery, fold_search, word_finds};
use matching::{cjk, starts_a_word};

/// The concepts the search groups nodes by, as stable IDs. Their words are the
/// catalogue messages `concept-<id>`, their one-line purposes `concept-<id>-purpose`.
pub const CONCEPTS: &[&str] = &[
    "trigger", "math", "compare", "logic", "text", "time", "audio", "network", "file", "variable",
    "loop", "list", "data", "color",
];

/// Bundled search languages available from every editor locale.
pub const SEARCH_LANGUAGES: [&str; 3] = ["en", "es", "ja"];

include!(concat!(env!("OUT_DIR"), "/locales.rs"));

/// Prepared search text for one language and registry generation.
#[derive(Debug, Default)]
pub(crate) struct SearchIndex {
    generation: Option<u64>,
    locale: String,
    nodes: Arc<[SearchNode]>,
}

#[derive(Debug)]
struct SearchNode {
    found: Found,
    active: SearchText,
    fallback: SearchText,
}

#[derive(Debug, Default)]
struct SearchText {
    names: Vec<String>,
    aliases: Vec<String>,
    concepts: Vec<String>,
    description: String,
    settings: Vec<String>,
}

/// Ordering of search results, including their language preference.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SearchRank {
    tier: u8,
    fallback: bool,
    kind: Match,
}

impl SearchRank {
    /// The rank of an exact match in the active language, and of every row when
    /// nothing is typed.
    pub const BEST: Self = Self::new(Match::Exact, false);

    /// A match of strength `kind`; `fallback` when it needed a caption outside the
    /// active language.
    pub const fn new(kind: Match, fallback: bool) -> Self {
        let tier = match kind {
            Match::Exact
            | Match::AliasExact
            | Match::NameStart
            | Match::WordStart
            | Match::Alias => 0,
            Match::Concept | Match::Description => 1,
            Match::Typo => 2,
        };
        Self {
            tier,
            fallback,
            kind,
        }
    }

    pub fn kind(self) -> Match {
        self.kind
    }

    /// This rank, but no stronger than `kind`, keeping its language preference.
    pub fn no_stronger_than(self, kind: Match) -> Self {
        Self::new(self.kind.max(kind), self.fallback)
    }
}

impl SearchNode {
    fn rank(&self, query: &SearchQuery) -> Option<SearchRank> {
        let phrase = [
            self.active.phrase(&query.phrase, false),
            self.fallback.phrase(&query.phrase, true),
        ]
        .into_iter()
        .flatten()
        .min();
        if phrase.is_some_and(|rank| !rank.fallback && rank.kind() <= Match::WordStart) {
            return phrase;
        }
        let each = query
            .terms
            .iter()
            .try_fold(SearchRank::BEST, |worst, term| {
                let best = [
                    self.active.term(term, false),
                    self.fallback.term(term, true),
                ]
                .into_iter()
                .flatten()
                .min()?;
                let kind = match (best.kind(), query.terms.len() > 1) {
                    (Match::Exact, true) => Match::WordStart,
                    (Match::AliasExact, true) => Match::Alias,
                    (kind, _) => kind,
                };
                Some(SearchRank::new(
                    worst.kind().max(kind),
                    worst.fallback || best.fallback,
                ))
            });
        phrase.into_iter().chain(each).min()
    }
}

impl SearchText {
    fn phrase(&self, query: &str, fallback: bool) -> Option<SearchRank> {
        let kind = if self.names.iter().any(|name| name == query) {
            if fallback {
                Match::AliasExact
            } else {
                Match::Exact
            }
        } else if self.aliases.iter().any(|alias| alias == query) {
            Match::AliasExact
        } else if !fallback && self.names.iter().any(|name| name.starts_with(query)) {
            Match::NameStart
        } else if !fallback && self.names.iter().any(|name| starts_a_word(name, query)) {
            Match::WordStart
        } else if self
            .aliases
            .iter()
            .chain(self.names.iter().filter(|_| fallback))
            .any(|alias| alias.starts_with(query))
        {
            Match::Alias
        } else {
            return None;
        };
        Some(SearchRank::new(kind, fallback))
    }

    fn term(&self, term: &matching::Term, fallback: bool) -> Option<SearchRank> {
        let word = &term.text;
        let kind = if let Some(rank) = self.phrase(word, fallback) {
            rank.kind()
        } else if self
            .aliases
            .iter()
            .chain(self.names.iter().filter(|_| fallback))
            .any(|alias| starts_a_word(alias, word))
        {
            Match::Alias
        } else if self
            .concepts
            .iter()
            .any(|concept| starts_a_word(concept, word))
        {
            Match::Concept
        } else if self.description.contains(word)
            || (word.chars().count() >= 3 || word.contains(cjk))
                && self
                    .settings
                    .iter()
                    .any(|setting| starts_a_word(setting, word))
        {
            Match::Description
        } else if self
            .names
            .iter()
            .chain(&self.aliases)
            .chain(&self.concepts)
            .chain(&self.settings)
            .any(|candidate| term.typo_in(candidate))
        {
            Match::Typo
        } else {
            return None;
        };
        Some(SearchRank::new(kind, fallback))
    }
}

/// Kinds that stay loadable but are not offered: the two-input And and Or, which the
/// growable forms (`core/all-conditions`, `core/any-conditions`) replace as And and Or.
pub const UNLISTED: [&str; 2] = [intrinsic::AND, intrinsic::OR];

static BUILT_IN: LazyLock<Catalogs> = LazyLock::new(|| {
    let mut catalogs = Catalogs::default();
    for (locale, text) in CATALOGS {
        // The tests read every bundled catalogue; one that fails is left out.
        let added = catalogs.add(locale, text);
        debug_assert!(added.is_ok(), "bundled catalogue {locale}: {added:?}");
    }
    catalogs
});

impl Catalogs {
    /// The host's own catalogues: search words and host messages in every language
    /// it ships, from `locales/<tag>/*.ftl`.
    pub fn built_in() -> &'static Self {
        &BUILT_IN
    }

    /// The aliases of node `definition` in `locale` and in English.
    pub fn aliases(&self, locale: &str, definition: &str) -> Vec<String> {
        let id = format!("alias-{}", node_key(definition));
        let mut found: Vec<String> = Vec::new();
        for word in self.words(locale, &id) {
            if !found.contains(&word) {
                found.push(word);
            }
        }
        found
    }

    /// The comma-separated words of message `id` in `locale` and in English.
    fn words(&self, locale: &str, id: &str) -> impl Iterator<Item = String> + use<'_> {
        let texts: Vec<_> = self
            .chain(locale)
            .filter_map(|language| language.format(id, &[]))
            .collect();
        texts.into_iter().flat_map(|text| {
            text.split(',')
                .map(str::trim)
                .filter(|word| !word.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
    }
}

/// How a found node matched, best first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Match {
    /// Its name is the search.
    Exact,
    /// One of its aliases is the whole search.
    AliasExact,
    /// Its name starts with the search.
    NameStart,
    /// A word of its name starts with the search.
    WordStart,
    /// One of its aliases.
    Alias,
    /// A concept it belongs to, or its category.
    Concept,
    /// Its description, ID or configuration.
    Description,
    /// Its name, an alias, a concept or a setting with one spelling edit.
    Typo,
}

/// One node kind a search found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// The definition, such as `org.orange.math/add`.
    pub definition: String,
    pub label: String,
    pub category: String,
    /// One line on what it does, in the search's language, or empty.
    pub purpose: String,
    /// The package that provides it; empty for the language's own nodes.
    pub package: String,
    /// How it matched; `None` when nothing was typed.
    pub matched: Option<Match>,
}

impl Host {
    /// Changes when registry or catalogue updates affect search results.
    pub fn search_revision(&self) -> (u64, u64) {
        (self.registry().generation(), self.catalogs.revision())
    }
    /// The host's and the packages' catalogues: search words, node names and host
    /// messages in every language there is.
    pub fn catalogs(&self) -> &Catalogs {
        &self.catalogs
    }

    /// Adds catalogue text for `locale`, such as a new language's, without changing
    /// any ID.
    pub fn add_catalog(&mut self, locale: &str, text: &str) -> Result<(), CatalogError> {
        self.catalogs.add(locale, text)?;
        *self
            .search_index
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = SearchIndex::default();
        Ok(())
    }

    /// Every node kind that matches `search` in `locale` or a bundled search language.
    /// The best match comes first, with the active language breaking ties.
    /// Each typed word must match the node. With nothing typed, returns every offered
    /// kind ordered by display name.
    pub fn search_nodes(&self, search: &str, locale: &str) -> Vec<Found> {
        self.search_nodes_with(search, locale, |found, _| found)
    }

    /// Search results with ranks for clients that merge in their own results.
    pub fn ranked_search_nodes(
        &self,
        search: &str,
        locale: &str,
    ) -> Vec<(Found, Option<SearchRank>)> {
        self.search_nodes_with(search, locale, |found, rank| (found, rank))
    }

    fn search_nodes_with<T>(
        &self,
        search: &str,
        locale: &str,
        project: impl Fn(Found, Option<SearchRank>) -> T,
    ) -> Vec<T> {
        let query = SearchQuery::new(search);
        if !query.is_valid() {
            return Vec::new();
        }
        let index = self.node_search_index(locale);
        let mut ranked: Vec<_> = index
            .iter()
            .filter_map(|entry| {
                let matched = if query.is_empty() {
                    None
                } else {
                    Some(entry.rank(&query)?)
                };
                Some((matched, &entry.found))
            })
            .collect();
        ranked.sort_by(|(a_rank, a), (b_rank, b)| {
            a_rank
                .cmp(b_rank)
                .then_with(|| a.label.cmp(&b.label))
                .then_with(|| a.definition.cmp(&b.definition))
        });
        let mut seen = HashSet::with_capacity(ranked.len());
        // Preserve the first (highest-ranked) version of a definition.
        ranked
            .into_iter()
            .filter(|(_, entry)| seen.insert(entry.definition.as_str()))
            .map(|(rank, entry)| {
                let found = Found {
                    matched: rank.map(SearchRank::kind),
                    ..entry.clone()
                };
                project(found, rank)
            })
            .collect()
    }

    /// The node kinds the palette and search offer: core and provided kinds that are
    /// neither hidden nor [`UNLISTED`]. Other kinds stay loadable.
    pub fn offered_nodes(&self) -> impl Iterator<Item = NodeInfo<'_>> {
        self.nodes()
            .filter(|info| info.definition.definition.id.is_core() || info.provider.is_some())
            .filter(|info| !info.presentation.is_some_and(|shown| shown.hidden))
            .filter(|info| !UNLISTED.contains(&info.definition.definition.id.as_str()))
    }

    fn node_search_index(&self, locale: &str) -> Arc<[SearchNode]> {
        let generation = self.registry().generation();
        let mut cache = self
            .search_index
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if cache.generation != Some(generation) || cache.locale != locale {
            let nodes = self
                .offered_nodes()
                .map(|info| self.prepare_search_node(&info, locale))
                .collect::<Vec<_>>();
            *cache = SearchIndex {
                generation: Some(generation),
                locale: locale.to_owned(),
                nodes: nodes.into(),
            };
        }
        Arc::clone(&cache.nodes)
    }

    /// The concepts node kind `info` belongs to.
    pub fn concepts_of(&self, info: &NodeInfo<'_>) -> Vec<&'static str> {
        concepts(info)
    }

    /// `text` in `locale` from the host's and the packages' catalogues, or its English
    /// text when no catalogue has it.
    pub fn text_in<'a>(&'a self, text: &'a TextRef, locale: &str) -> Cow<'a, str> {
        self.catalogs
            .message(locale, &node_key(&text.key))
            .unwrap_or(Cow::Borrowed(&text.fallback))
    }

    /// A node kind's name in `locale`: its definition file's label in that language, or
    /// for a node without one the catalogue message `name-<node key>`, or the English
    /// name made from its ID.
    pub fn label_in(&self, info: &NodeInfo<'_>, locale: &str) -> String {
        match info.presentation.and_then(|shown| shown.label.as_ref()) {
            Some(label) => self.text_in(label, locale).into_owned(),
            None => self
                .catalogs
                .message(
                    locale,
                    &format!("name-{}", node_key(info.definition.definition.id.as_str())),
                )
                .map_or_else(|| info.label().into_owned(), Cow::into_owned),
        }
    }

    /// A node kind's palette category in `locale`: its definition file's category in
    /// that language, or for a node without one the catalogue message
    /// `palette-<category>`, such as `palette-math`, or the English category.
    pub fn category_in(&self, info: &NodeInfo<'_>, locale: &str) -> String {
        match info.presentation.and_then(|shown| shown.category.as_ref()) {
            Some(category) => self.text_in(category, locale).into_owned(),
            None => self
                .catalogs
                .message(locale, &format!("palette-{}", category_key(info)))
                .map_or_else(|| info.category().into_owned(), Cow::into_owned),
        }
    }

    /// The names of ports in `locale` where a catalogue gives one, by port ID with
    /// "_" written as "-": the messages `port-<id>` of the host's and the packages'
    /// catalogues, such as `port-result`. A port without one is named from its ID.
    pub fn port_names(&self, locale: &str) -> Vec<(String, String)> {
        let mut names: Vec<(String, String)> = Vec::new();
        for language in self.catalogs.chain(locale) {
            for id in language.ids() {
                let Some(port) = id.strip_prefix("port-") else {
                    continue;
                };
                if names.iter().any(|(known, _)| known == port) {
                    continue;
                }
                if let Some(text) = language.format(id, &[]) {
                    names.push((port.to_owned(), text.into_owned()));
                }
            }
        }
        names
    }

    /// Translated success captions, keyed by definition ID and source caption.
    pub fn outcome_names(&self, locale: &str) -> Vec<(String, (String, String))> {
        self.nodes()
            .filter_map(|info| {
                let label = info.presentation?.then_label.as_ref()?;
                let translated = self.catalogs.message(locale, &node_key(&label.key))?;
                Some((
                    info.definition.definition.id.as_str().to_owned(),
                    (label.fallback.clone(), translated.into_owned()),
                ))
            })
            .collect()
    }

    fn prepare_search_node(&self, info: &NodeInfo<'_>, locale: &str) -> SearchNode {
        let id = info.definition.definition.id.as_str();
        let key = node_key(id);
        let mut active = SearchText::default();
        let mut fallback = SearchText {
            description: fold_search(id),
            ..SearchText::default()
        };
        let primary = primary_language(locale);
        self.add_search_text(
            &mut active,
            info,
            &key,
            locale,
            primary == crate::SOURCE_LANGUAGE,
        );
        for tag in SEARCH_LANGUAGES {
            if primary != tag {
                self.add_search_text(
                    &mut fallback,
                    info,
                    &key,
                    tag,
                    tag == crate::SOURCE_LANGUAGE,
                );
            }
        }
        SearchNode {
            found: Found {
                definition: id.to_owned(),
                label: self.label_in(info, locale),
                category: self.category_in(info, locale),
                purpose: self.purpose_in(info, locale),
                package: info
                    .provider
                    .map(|provider| provider.package.clone())
                    .unwrap_or_default(),
                matched: None,
            },
            active,
            fallback,
        }
    }

    fn add_search_text(
        &self,
        text: &mut SearchText,
        info: &NodeInfo<'_>,
        key: &str,
        locale: &str,
        source: bool,
    ) {
        let languages: Vec<_> = self
            .catalogs
            .chain(locale)
            .filter(|language| source || !language.is_source())
            .collect();
        let message = |id: &str| {
            languages
                .iter()
                .find_map(|language| language.format(id, &[]))
        };
        let words = |id: &str, terms: &mut Vec<String>| {
            for caption in languages
                .iter()
                .filter_map(|language| language.format(id, &[]))
            {
                terms.extend(
                    caption
                        .split(',')
                        .map(str::trim)
                        .filter(|word| !word.is_empty())
                        .map(fold_search),
                );
            }
        };
        let shown = info.presentation;
        let label = shown.and_then(|shown| shown.label.as_ref());
        let name = label.map_or_else(|| format!("name-{key}"), |label| node_key(&label.key));
        if let Some(label) = message(&name) {
            text.names.push(fold_search(&label));
        } else if source {
            text.names.push(fold_search(&info.label()));
        }
        words(&format!("alias-{key}"), &mut text.aliases);
        if let Some(keywords) = shown.and_then(|shown| shown.keywords.as_ref()) {
            words(&node_key(&keywords.key), &mut text.aliases);
            if source {
                text.aliases.extend(
                    keywords
                        .fallback
                        .split(',')
                        .map(str::trim)
                        .filter(|word| !word.is_empty())
                        .map(fold_search),
                );
            }
        }

        let category = shown.and_then(|shown| shown.category.as_ref());
        let slug = category_key(info);
        let category =
            category.map_or_else(|| format!("palette-{slug}"), |label| node_key(&label.key));
        if let Some(caption) = message(&category) {
            text.concepts.push(fold_search(&caption));
        } else if source {
            text.concepts.push(fold_search(&info.category()));
        }
        words(&format!("category-{slug}"), &mut text.concepts);
        for concept in concepts(info) {
            words(&format!("concept-{concept}"), &mut text.concepts);
        }

        if let Some(description) = shown.and_then(|shown| shown.description.as_ref()) {
            if let Some(caption) = message(&node_key(&description.key)) {
                text.description.push(' ');
                text.description.push_str(&fold_search(&caption));
            } else if source {
                text.description.push(' ');
                text.description
                    .push_str(&fold_search(&description.fallback));
            }
        }
        if let Some(caption) = message(&format!("purpose-{key}")) {
            text.description.push(' ');
            text.description.push_str(&fold_search(&caption));
        }

        let Some(properties) = info
            .definition
            .config_schema
            .as_ref()
            .and_then(|schema| schema["properties"].as_object())
        else {
            return;
        };
        for (name, schema) in properties
            .iter()
            .filter(|(_, schema)| schema["readOnly"] != true)
        {
            if source {
                text.settings.push(fold_search(name));
            }
            if let Some(caption) = message(&format!("setting-{}", node_key(name))) {
                text.settings.push(fold_search(&caption));
            }
            for value in schema["enum"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(serde_json::Value::as_str)
            {
                if source {
                    text.settings.push(fold_search(value));
                }
                if let Some(caption) = message(&format!("setting-choice-{}", node_key(value))) {
                    text.settings.push(fold_search(&caption));
                }
            }
        }
    }

    /// A short, localized description of what this node does.
    pub fn purpose_in(&self, info: &NodeInfo<'_>, locale: &str) -> String {
        let key = node_key(info.definition.definition.id.as_str());
        if let Some(text) = self.catalogs.message(locale, &format!("purpose-{key}")) {
            return text.into_owned();
        }
        if let Some(text) = info
            .presentation
            .and_then(|shown| shown.description.as_ref())
        {
            let text = self.text_in(text, locale);
            let text = text.trim();
            let end = text.find(". ").map_or(text.len(), |at| at + 1);
            return text[..end].to_owned();
        }
        concepts(info)
            .first()
            .and_then(|concept| {
                self.catalogs
                    .message(locale, &format!("concept-{concept}-purpose"))
            })
            .map(Cow::into_owned)
            .unwrap_or_default()
    }
}

/// The stable key of a node kind's palette category, whatever the language: its
/// definition file's category ID, or its English category in lower case, such as `math`.
fn category_key(info: &NodeInfo<'_>) -> String {
    match info
        .presentation
        .and_then(|shown| shown.category_id.as_deref())
    {
        Some(id) => id.to_owned(),
        None => info.category().to_lowercase().replace(' ', "-"),
    }
}

/// The catalogue key of a node ID: `org.orange.audio/play` is `org-orange-audio-play`.
fn node_key(id: &str) -> String {
    id.chars()
        .map(|c| match c {
            '/' | '.' | '_' => '-',
            c => c.to_ascii_lowercase(),
        })
        .collect()
}

/// The concepts a node belongs to, from its kind, ID and declared category.
fn concepts(info: &NodeInfo<'_>) -> Vec<&'static str> {
    let id = info.definition.definition.id.as_str();
    let (namespace, slug) = id.rsplit_once('/').unwrap_or(("", id));
    let has = |part: &str| slug.split('-').any(|word| word == part);
    let mut found = Vec::new();
    let mut add = |concept: &'static str, yes: bool| {
        if yes && !found.contains(&concept) {
            found.push(concept);
        }
    };
    let kind = info.definition.kind;
    add("trigger", kind == NodeKind::Entry);
    add(
        "loop",
        kind == NodeKind::Iterator
            || namespace == "core"
                && ["break", "continue", "repeat", "map", "fold", "filter"]
                    .iter()
                    .any(|part| has(part)),
    );
    let compares = ["equal", "greater", "less", "order"]
        .iter()
        .any(|part| has(part));
    add("compare", compares);
    add(
        "logic",
        ["and", "or", "not", "xor"].contains(&slug) && namespace != "org.orange.text"
            || namespace == "core"
                && [
                    "branch",
                    "select",
                    "switch",
                    "gate",
                    "flip-flop",
                    "coalesce",
                    "all-conditions",
                    "any-conditions",
                    "true-or-false",
                ]
                .contains(&slug),
    );
    add(
        "math",
        namespace == "org.orange.math" && !compares
            || [intrinsic::WHOLE_NUMBER, intrinsic::NUMBER].contains(&id),
    );
    add(
        "text",
        namespace == "org.orange.text" || id == intrinsic::TEXT,
    );
    add(
        "time",
        namespace == "org.orange.time"
            || ["timer", "timeout", "wait", "now", "time", "delay"]
                .iter()
                .any(|part| has(part)),
    );
    add("color", namespace == "org.orange.color");
    add(
        "network",
        ["http", "mqtt", "websocket"]
            .iter()
            .any(|part| slug.contains(part)),
    );
    add(
        "variable",
        ["variable", "shared", "register"]
            .iter()
            .any(|part| has(part)),
    );
    add("list", namespace == "org.orange.collections");
    add("data", namespace == "org.orange.json");
    // A package joins a concept by naming it as its category, such as
    // `org.orange.audio/category-audio`; a plural (`files`) names the same concept.
    if let Some(category) = info
        .presentation
        .and_then(|shown| shown.category_id.as_deref())
    {
        let last = category.rsplit(['/', '.']).next().unwrap_or(category);
        let last = last.strip_prefix("category-").unwrap_or(last);
        let singular = last.strip_suffix('s').unwrap_or(last);
        if let Some(concept) = CONCEPTS
            .iter()
            .find(|concept| **concept == last || **concept == singular)
        {
            add(concept, true);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_channel_name_is_found_with_one_typo_and_without_accents() {
        assert!(word_finds("door", "Door bell"));
        assert!(word_finds("bel", "Door bell"));
        assert!(word_finds("doro", "Door bell"));
        assert!(word_finds("puerta", "Timbre de la puérta"));
        assert!(word_finds("puetra", "Timbre de la puerta"));
        assert!(!word_finds("window", "Door bell"));
        assert!(!word_finds("", "Door bell"));
    }

    #[test]
    fn every_bundled_catalogue_reads_and_translates_english_messages() {
        for (locale, text) in CATALOGS {
            assert!(Catalogs::default().add(locale, text).is_ok(), "{locale}");
        }
        let words = Catalogs::built_in();
        let english = words.ids(crate::i18n::SOURCE_LANGUAGE);
        for (tag, _) in words.languages() {
            // Extra search words are a language's own, and so are the names of nodes,
            // categories and ports that English makes from their IDs; every other
            // message translates one.
            for id in words.ids(tag) {
                let extra_words = [
                    "alias-",
                    "category-",
                    "name-",
                    "palette-",
                    "port-",
                    "setting-",
                ]
                .iter()
                .any(|prefix| id.starts_with(prefix));
                assert!(
                    extra_words || english.contains(&id),
                    "{tag} has {id}, which English lacks"
                );
            }
        }
        assert_eq!(
            words.message("es-MX", "concept-text"),
            words.message("es", "concept-text")
        );
    }

    /// One Greater than for every kind of number: search offers it and not the old
    /// decimal-only one, and two decimals wired into it make a valid graph.
    #[test]
    fn one_greater_than_compares_decimals_too() {
        let (host, _) = Host::new(crate::HostOptions::default()).unwrap();
        let found: Vec<_> = host
            .search_nodes("greater", "en")
            .into_iter()
            .map(|found| found.definition)
            .collect();
        assert!(found.iter().any(|id| id == "org.orange.math/greater"));
        assert!(
            !found
                .iter()
                .any(|id| id.starts_with("org.orange.math/float-greater")),
            "{found:?}"
        );
        let mut draft = host.draft(&crate::new_document().unwrap()).unwrap();
        let greater = host
            .add_node(&mut draft, "org.orange.math/greater", [300.0, 0.0])
            .unwrap();
        for (port, value) in [("a", "1.5"), ("b", "0.5")] {
            let number = host
                .add_node(&mut draft, intrinsic::NUMBER, [0.0, 0.0])
                .unwrap();
            host.set_input(&mut draft, (number, "value"), Some(value))
                .unwrap();
            host.connect(&mut draft, (number, "result"), (greater, port))
                .unwrap();
        }
        assert_eq!(host.problems(&mut draft).unwrap(), []);
    }

    /// Every collection and JSON node says what it does in its own words, in both
    /// languages, instead of sharing one sentence with its whole category.
    #[test]
    fn collection_and_json_nodes_each_have_their_own_purpose() {
        let (host, _) = Host::new(crate::HostOptions::default()).unwrap();
        for locale in ["en", "es"] {
            let found: Vec<_> = host
                .search_nodes("", locale)
                .into_iter()
                .filter(|found| {
                    found.definition.starts_with("org.orange.collections/")
                        || found.definition.starts_with("org.orange.json/")
                })
                .collect();
            assert!(found.len() >= 30, "{locale}: {}", found.len());
            let mut purposes: Vec<_> = found.iter().map(|found| found.purpose.clone()).collect();
            purposes.sort();
            purposes.dedup();
            assert_eq!(purposes.len(), found.len(), "{locale}: {purposes:?}");
        }
    }

    /// One Absolute serves whole numbers and decimals, each by its own rules, and
    /// search offers no decimal-only twin.
    #[test]
    fn one_absolute_for_every_number_kind() {
        let (mut host, _) = Host::new(crate::HostOptions::default()).unwrap();
        let found: Vec<_> = host
            .search_nodes("absolute", "en")
            .into_iter()
            .map(|found| found.definition)
            .collect();
        assert!(found.iter().any(|id| id == "org.orange.math/absolute"));
        assert!(!found.iter().any(|id| id.contains("float-")), "{found:?}");
        for (ty, typed, expected) in [
            ("core/i64@1", "-7", crate::Value::I64(7)),
            ("core/f64@1", "-2.5", crate::Value::F64(2.5)),
        ] {
            let mut draft = host.draft(&crate::new_document().unwrap()).unwrap();
            let node = host
                .add_node(&mut draft, "org.orange.math/absolute", [0.0, 0.0])
                .unwrap();
            host.choose_type(&mut draft, node, "T", Some(ty)).unwrap();
            host.set_input(&mut draft, (node, "value"), Some(typed))
                .unwrap();
            let wanted = [(node, "result".parse().unwrap())];
            let values = host.preview(&draft.text().unwrap(), &wanted).unwrap();
            assert_eq!(values[0].value, Ok(expected), "{ty}");
        }
    }
}
