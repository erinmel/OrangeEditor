//! Finding node kinds by what a person types, like a shop search: by name, alias,
//! category, purpose ("concept") and description, in the active language and English,
//! forgiving one typo. The words are catalogue data ([`SearchWords`]); which nodes a
//! concept groups follows from their kind and ID, so new trigger kinds join "trigger"
//! without a list to keep up.
use std::collections::BTreeMap;
use std::sync::LazyLock;

use orange_model::document::NodeKind;

use crate::{Host, NodeInfo};

/// The concepts the search groups nodes by, as stable IDs. Their words are the
/// catalogue messages `concept-<id>`, their one-line purposes `concept-<id>-purpose`.
pub const CONCEPTS: &[&str] = &[
    "trigger", "math", "compare", "logic", "text", "time", "audio", "network", "file", "variable",
    "loop", "list", "data",
];

/// The language every search also matches in, and whose words are always present.
pub const SOURCE_LANGUAGE: &str = "en";

static BUILT_IN: LazyLock<SearchWords> = LazyLock::new(|| {
    let mut words = SearchWords::default();
    for (locale, text) in [
        ("en", include_str!("../locales/search-en.ftl")),
        ("es", include_str!("../locales/search-es.ftl")),
    ] {
        words
            .add(locale, text)
            .unwrap_or_else(|error| panic!("bundled search words for {locale}: {error}"));
    }
    words
});

/// Translatable words the search finds nodes by, per language: a small subset of
/// Fluent (`id = text` lines and `#` comments). Message IDs are stable and
/// language-free, so a language only adds words. See `locales/search-en.ftl`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchWords {
    /// Lower-case language tag, then message ID, then its text.
    languages: BTreeMap<String, BTreeMap<String, String>>,
}

/// A catalogue that could not be read, with the line it stopped at.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("line {line}: {reason}")]
pub struct SearchWordsError {
    pub line: usize,
    pub reason: &'static str,
}

impl SearchWords {
    /// The words shipped with the host, in English and Spanish.
    pub fn built_in() -> &'static Self {
        &BUILT_IN
    }

    /// Adds the messages of `text` to language `locale`; a message already there is
    /// replaced. Nothing is added when a line is malformed.
    pub fn add(&mut self, locale: &str, text: &str) -> Result<(), SearchWordsError> {
        let locale = locale.trim().to_ascii_lowercase();
        let valid_tag = !locale.is_empty()
            && locale
                .split('-')
                .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric()));
        if !valid_tag {
            return Err(SearchWordsError {
                line: 0,
                reason: "the language tag is not BCP 47 shaped",
            });
        }
        let mut read = BTreeMap::new();
        for (index, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let error = |reason| SearchWordsError {
                line: index + 1,
                reason,
            };
            let (id, value) = line.split_once('=').ok_or(error("expected `id = text`"))?;
            let id = id.trim();
            let valid_id = id.starts_with(|c: char| c.is_ascii_lowercase())
                && id
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
            if !valid_id {
                return Err(error("message IDs are [a-z][a-z0-9-]*"));
            }
            if value.contains(['{', '}']) {
                return Err(error("search words take no arguments"));
            }
            read.insert(id.to_owned(), value.trim().to_owned());
        }
        self.languages.entry(locale).or_default().extend(read);
        Ok(())
    }

    /// The languages that have words, with their own names.
    pub fn languages(&self) -> Vec<(&str, &str)> {
        self.languages
            .iter()
            .map(|(tag, messages)| {
                let name = messages
                    .get("language-name")
                    .map_or(tag.as_str(), String::as_str);
                (tag.as_str(), name)
            })
            .collect()
    }

    /// The text of message `id` for `locale`: its exact tag, then each shorter one
    /// (`es-mx`, then `es`), then English.
    pub fn message(&self, locale: &str, id: &str) -> Option<&str> {
        self.chain(locale)
            .find_map(|messages| messages.get(id))
            .map(String::as_str)
    }

    /// The aliases of node `definition` in `locale` and in English.
    pub fn aliases(&self, locale: &str, definition: &str) -> Vec<&str> {
        let id = format!("alias-{}", node_key(definition));
        let mut found: Vec<&str> = Vec::new();
        for word in self.words(locale, &id) {
            if !found.contains(&word) {
                found.push(word);
            }
        }
        found
    }

    /// The comma-separated words of message `id` in `locale` and in English.
    fn words(&self, locale: &str, id: &str) -> impl Iterator<Item = &str> + use<'_> {
        let mut seen: Vec<&BTreeMap<String, String>> = Vec::new();
        for messages in self.chain(locale) {
            if !seen.iter().any(|known| std::ptr::eq(*known, messages)) {
                seen.push(messages);
            }
        }
        let texts: Vec<&String> = seen
            .into_iter()
            .filter_map(|messages| messages.get(id))
            .collect();
        texts
            .into_iter()
            .flat_map(|text| text.split(','))
            .map(str::trim)
            .filter(|word| !word.is_empty())
    }

    fn chain(&self, locale: &str) -> impl Iterator<Item = &BTreeMap<String, String>> {
        let locale = locale.trim().to_ascii_lowercase();
        let mut tags = Vec::new();
        let mut tag = locale.as_str();
        while !tag.is_empty() {
            tags.push(tag.to_owned());
            tag = tag.rsplit_once('-').map_or("", |(shorter, _)| shorter);
        }
        tags.push(SOURCE_LANGUAGE.to_owned());
        tags.into_iter().filter_map(|tag| self.languages.get(&tag))
    }
}

/// How a found node matched, best first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Match {
    /// Its name is the search.
    Exact,
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
    /// The words the search uses: the built-in ones and any added since.
    pub fn search_words(&self) -> &SearchWords {
        &self.search_words
    }

    /// Adds search words for `locale` from catalogue text, such as a package's or a
    /// new language's, without changing any ID.
    pub fn add_search_words(&mut self, locale: &str, text: &str) -> Result<(), SearchWordsError> {
        self.search_words.add(locale, text)
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
            .filter_map(|info| {
                let matched = if words.is_empty() {
                    None
                } else {
                    Some(self.rank(&info, &search, &words, locale)?)
                };
                Some(Found {
                    definition: info.definition.definition.id.as_str().to_owned(),
                    label: info.label().into_owned(),
                    category: info.category().into_owned(),
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

    fn rank(
        &self,
        info: &NodeInfo<'_>,
        search: &str,
        words: &[&str],
        locale: &str,
    ) -> Option<Match> {
        let id = info.definition.definition.id.as_str();
        let key = node_key(id);
        let label = fold(&info.label());
        let aliases: Vec<String> = self
            .search_words
            .words(locale, &format!("alias-{key}"))
            .map(fold)
            .collect();
        let category = info.category();
        let category_slug = category.to_lowercase().replace(' ', "-");
        let mut concept_words: Vec<String> = fold(&category)
            .split_whitespace()
            .map(str::to_owned)
            .collect();
        concept_words.extend(
            self.search_words
                .words(locale, &format!("category-{category_slug}"))
                .map(fold),
        );
        for concept in concepts(info) {
            concept_words.extend(
                self.search_words
                    .words(locale, &format!("concept-{concept}"))
                    .map(fold),
            );
        }
        let mut description = fold(id);
        if let Some(text) = info
            .presentation
            .and_then(|shown| shown.description.as_ref())
        {
            description.push(' ');
            description.push_str(&fold(&text.fallback));
        }
        if let Some(text) = self.search_words.message(locale, &format!("purpose-{key}")) {
            description.push(' ');
            description.push_str(&fold(text));
        }

        // The whole search against the name and aliases, as a phrase.
        let phrase = if label == search {
            Some(Match::Exact)
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
        if let Some(text) = self.search_words.message(locale, &format!("purpose-{key}")) {
            return text.to_owned();
        }
        if let Some(text) = info
            .presentation
            .and_then(|shown| shown.description.as_ref())
        {
            let text = text.fallback.trim();
            let end = text.find(". ").map_or(text.len(), |at| at + 1);
            return text[..end].to_owned();
        }
        concepts(info)
            .first()
            .and_then(|concept| {
                self.search_words
                    .message(locale, &format!("concept-{concept}-purpose"))
            })
            .unwrap_or_default()
            .to_owned()
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
        ["timer", "timeout", "wait", "now", "time", "delay"]
            .iter()
            .any(|part| has(part)),
    );
    add("audio", namespace == "org.orange.audio");
    add(
        "network",
        namespace == "org.orange.net"
            || ["http", "mqtt", "websocket"]
                .iter()
                .any(|part| slug.contains(part)),
    );
    add("file", namespace == "org.orange.files");
    add(
        "variable",
        ["variable", "shared", "register"]
            .iter()
            .any(|part| has(part)),
    );
    add("list", namespace == "org.orange.collections");
    add("data", namespace == "org.orange.json");
    // A package joins a concept by naming it as its category.
    if let Some(category) = info
        .presentation
        .and_then(|shown| shown.category_id.as_deref())
    {
        let last = category.rsplit(['/', '.']).next().unwrap_or(category);
        if let Some(concept) = CONCEPTS.iter().find(|concept| **concept == last) {
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
    fn catalogues_fall_back_to_shorter_tags_then_english_and_refuse_bad_lines() {
        let words = SearchWords::built_in();
        assert_eq!(
            words.message("es-MX", "concept-text"),
            words.message("es", "concept-text")
        );
        assert_eq!(words.message("fr", "language-name"), Some("English"));
        assert!(words.languages().contains(&("es", "Español")));
        let mut words = SearchWords::default();
        assert_eq!(words.add("en", "Bad-Id = x").unwrap_err().line, 1);
        assert!(words.add("en", "\n# note\nok = { $arg }").is_err());
        assert!(words.add("not a tag", "ok = x").is_err());
        assert_eq!(words, SearchWords::default());
    }
}
