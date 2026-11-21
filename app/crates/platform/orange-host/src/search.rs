//! Finding node kinds by what a person types, like a shop search: by name, alias,
//! category, purpose ("concept") and description, in the active language and English,
//! forgiving one typo. The words are catalogue data ([`Catalogs`]); which nodes a
//! concept groups follows from their kind and ID, so new trigger kinds join "trigger"
//! without a list to keep up.
use std::borrow::Cow;
use std::sync::LazyLock;

use orange_model::document::NodeKind;

use crate::i18n::{CatalogError, Catalogs};
use crate::{Host, NodeInfo};
use orange_model::catalog::TextRef;

/// The concepts the search groups nodes by, as stable IDs. Their words are the
/// catalogue messages `concept-<id>`, their one-line purposes `concept-<id>-purpose`.
pub const CONCEPTS: &[&str] = &[
    "trigger", "math", "compare", "logic", "text", "time", "audio", "network", "file", "variable",
    "loop", "list", "data",
];

include!(concat!(env!("OUT_DIR"), "/locales.rs"));

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
    /// Its description or ID.
    Description,
    /// Its name, an alias or a concept, with one letter wrong, missing or swapped.
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
    /// The host's and the packages' catalogues: search words, node names and host
    /// messages in every language there is.
    pub fn catalogs(&self) -> &Catalogs {
        &self.catalogs
    }

    /// Adds catalogue text for `locale`, such as a new language's, without changing
    /// any ID.
    pub fn add_catalog(&mut self, locale: &str, text: &str) -> Result<(), CatalogError> {
        self.catalogs.add(locale, text)
    }

    /// Every node kind that matches `search` in language `locale` or English, best
    /// match first, then by name. Each word typed must match something of the node;
    /// a node ranks by its weakest word unless the whole search matches its name or an
    /// alias. With nothing typed, every kind by name.
    pub fn search_nodes(&self, search: &str, locale: &str) -> Vec<Found> {
        let search = fold(search);
        let words: Vec<&str> = search.split_whitespace().collect();
        let mut found: Vec<Found> = self
            .nodes()
            .filter(|info| info.definition.definition.id.is_core() || info.provider.is_some())
            .filter(|info| !info.presentation.is_some_and(|shown| shown.hidden))
            .filter_map(|info| {
                let matched = if words.is_empty() {
                    None
                } else {
                    Some(self.rank(&info, &search, &words, locale)?)
                };
                Some(Found {
                    definition: info.definition.definition.id.as_str().to_owned(),
                    label: self.label_in(&info, locale),
                    category: self.category_in(&info, locale),
                    purpose: self.purpose(&info, locale),
                    package: info
                        .provider
                        .map(|provider| provider.package.clone())
                        .unwrap_or_default(),
                    matched,
                })
            })
            .collect();
        found.sort_by(|a, b| {
            a.matched
                .cmp(&b.matched)
                .then_with(|| a.label.cmp(&b.label))
                .then_with(|| a.definition.cmp(&b.definition))
        });
        found.dedup_by(|a, b| a.definition == b.definition);
        found
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

    fn rank(
        &self,
        info: &NodeInfo<'_>,
        search: &str,
        words: &[&str],
        locale: &str,
    ) -> Option<Match> {
        let id = info.definition.definition.id.as_str();
        let key = node_key(id);
        // The name in the search's language, and in English too.
        let label = fold(&self.label_in(info, locale));
        let english = fold(&info.label());
        let mut aliases: Vec<String> = self
            .catalogs
            .words(locale, &format!("alias-{key}"))
            .map(|word| fold(&word))
            .collect();
        if english != label {
            aliases.push(english);
        }
        // The package's own keywords, translated by its catalogue and in English.
        if let Some(keywords) = info.presentation.and_then(|shown| shown.keywords.as_ref()) {
            aliases.extend(
                self.catalogs
                    .words(locale, &node_key(&keywords.key))
                    .map(|word| fold(&word)),
            );
            aliases.extend(
                keywords
                    .fallback
                    .split(',')
                    .map(str::trim)
                    .filter(|w| !w.is_empty())
                    .map(fold),
            );
        }
        let category = self.category_in(info, locale);
        let category_slug = category_key(info);
        let mut concept_words: Vec<String> = fold(&category)
            .split_whitespace()
            .map(str::to_owned)
            .collect();
        concept_words.extend(
            self.catalogs
                .words(locale, &format!("category-{category_slug}"))
                .map(|word| fold(&word)),
        );
        for concept in concepts(info) {
            concept_words.extend(
                self.catalogs
                    .words(locale, &format!("concept-{concept}"))
                    .map(|word| fold(&word)),
            );
        }
        let mut description = fold(id);
        if let Some(text) = info
            .presentation
            .and_then(|shown| shown.description.as_ref())
        {
            description.push(' ');
            description.push_str(&fold(&self.text_in(text, locale)));
            description.push(' ');
            description.push_str(&fold(&text.fallback));
        }
        if let Some(text) = self.catalogs.message(locale, &format!("purpose-{key}")) {
            description.push(' ');
            description.push_str(&fold(&text));
        }

        // The whole search against the name and aliases, as a phrase.
        let phrase = if label == search {
            Some(Match::Exact)
        } else if aliases.iter().any(|alias| alias == search) {
            Some(Match::AliasExact)
        } else if label.starts_with(search) {
            Some(Match::NameStart)
        } else if starts_a_word(&label, search) {
            Some(Match::WordStart)
        } else if aliases.iter().any(|alias| alias.starts_with(search)) {
            Some(Match::Alias)
        } else {
            None
        };
        // Otherwise every word must match something; the weakest decides.
        let each = words
            .iter()
            .map(|word| {
                if label == *word {
                    Some(Match::Exact)
                } else if label.starts_with(word) {
                    Some(Match::NameStart)
                } else if starts_a_word(&label, word) {
                    Some(Match::WordStart)
                } else if aliases.iter().any(|alias| starts_a_word(alias, word)) {
                    Some(Match::Alias)
                } else if concept_words
                    .iter()
                    .any(|concept| starts_a_word(concept, word))
                {
                    Some(Match::Concept)
                } else if description.contains(word) {
                    Some(Match::Description)
                } else if one_typo_from_any(word, &label, &aliases, &concept_words) {
                    Some(Match::Typo)
                } else {
                    None
                }
            })
            .try_fold(Match::Exact, |worst, matched| Some(worst.max(matched?)));
        match (phrase, each) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    fn purpose(&self, info: &NodeInfo<'_>, locale: &str) -> String {
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
            || ["core/whole-number", "core/number"].contains(&id),
    );
    add("text", namespace == "org.orange.text" || id == "core/text");
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

/// Lower case without accents, so "numero" finds "número".
fn fold(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ä' | 'ã' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            c => c,
        })
        .collect()
}

/// Whether a word of `text` (split at spaces and dashes) starts with `search`.
fn starts_a_word(text: &str, search: &str) -> bool {
    text.split([' ', '-'])
        .any(|word| !word.is_empty() && word.starts_with(search))
        || text.starts_with(search)
}

/// Whether `word` is one letter off a word of the name, an alias or a concept, or off
/// the start of one (a typo while still typing). Short words are never fuzzy.
fn one_typo_from_any(word: &str, label: &str, aliases: &[String], concepts: &[String]) -> bool {
    if word.chars().count() < 4 {
        return false;
    }
    let mut candidates = label
        .split([' ', '-'])
        .chain(aliases.iter().flat_map(|alias| alias.split(' ')))
        .chain(concepts.iter().map(String::as_str));
    candidates.any(|candidate| {
        let candidate: Vec<char> = candidate.chars().collect();
        let word: Vec<char> = word.chars().collect();
        within_one(&word, &candidate)
            || (word.len()..=word.len() + 1)
                .filter(|&end| end < candidate.len())
                .any(|end| within_one(&word, &candidate[..end]))
    })
}

/// Edit distance at most one, counting a swap of two neighbours as one.
fn within_one(a: &[char], b: &[char]) -> bool {
    if a == b {
        return true;
    }
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    match long.len() - short.len() {
        0 => {
            let differ: Vec<usize> = (0..a.len()).filter(|&i| a[i] != b[i]).collect();
            differ.len() == 1
                || differ.len() == 2
                    && differ[1] == differ[0] + 1
                    && a[differ[0]] == b[differ[1]]
                    && a[differ[1]] == b[differ[0]]
        }
        1 => {
            let at = short
                .iter()
                .zip(long)
                .position(|(x, y)| x != y)
                .unwrap_or(short.len());
            short[at..] == long[at + 1..]
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typos_are_one_edit() {
        let chars = |text: &str| text.chars().collect::<Vec<_>>();
        assert!(within_one(&chars("timer"), &chars("tiemr")));
        assert!(within_one(&chars("timer"), &chars("timr")));
        assert!(within_one(&chars("timer"), &chars("timers")));
        assert!(within_one(&chars("timer"), &chars("tamer")));
        assert!(!within_one(&chars("timer"), &chars("tmire")));
        assert!(!within_one(&chars("timer"), &chars("time it")));
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
                let extra_words = ["alias-", "category-", "name-", "palette-", "port-"]
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
                .add_node(&mut draft, "core/number", [0.0, 0.0])
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
