//! The editor's own text in the person's language ([docs/13](../../../../../docs/13-internationalization.md)).
//!
//! Text lives in `locales/<tag>/*.ftl`, English first; code names a message by ID:
//! [`t`] for text without arguments, [`tf`] for a message with named arguments, so a
//! translation can reorder them or choose its plural forms. The editor sets the
//! language once per frame with [`set_language`]; until then, as in tests, text is
//! English.
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, OnceLock};

pub use orange_host::Arg;
use orange_host::{Catalogs, SOURCE_LANGUAGE};

include!(concat!(env!("OUT_DIR"), "/locales.rs"));

pub(crate) const DEFAULT_LOCALE: &str = "en-US";

// The built-in choices name the regional variety of each shared catalogue.
pub(crate) fn preference_tag(tag: &str) -> Cow<'_, str> {
    let tag = tag.trim();
    let comparable = tag.replace('_', "-");
    for (base, regional) in [("en", DEFAULT_LOCALE), ("es", "es-MX"), ("ja", "ja-JP")] {
        if comparable.eq_ignore_ascii_case(base) || comparable.eq_ignore_ascii_case(regional) {
            return Cow::Borrowed(regional);
        }
    }
    match orange_host::canonical_locale(&comparable) {
        Some(canonical) if canonical != tag => Cow::Owned(canonical),
        _ => Cow::Borrowed(tag),
    }
}

static APPLICATION: OnceLock<&'static [(&'static str, &'static str)]> = OnceLock::new();

static TEXT: LazyLock<Catalogs> = LazyLock::new(|| {
    let mut catalogs = Catalogs::default();
    let application = APPLICATION.get().copied().unwrap_or_default();
    for (tag, text) in CATALOGS.iter().chain(application) {
        // Tests read every bundled catalogue; one that fails is left out.
        let added = catalogs.add(tag, text);
        debug_assert!(added.is_ok(), "catalogue {tag}: {added:?}");
    }
    if cfg!(debug_assertions) {
        catalogs.add_pseudo();
    }
    catalogs
});

thread_local! {
    static LANGUAGE: RefCell<String> = RefCell::new(SOURCE_LANGUAGE.to_owned());
    /// Text formatted once for a language and kept for the life of the program:
    /// at most every message in every language.
    static KEPT: RefCell<HashSet<&'static str>> = RefCell::new(HashSet::new());
    /// The host's names of ports in the current language, by port ID with "_" as "-".
    static PORTS: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
    static OUTCOMES: RefCell<HashMap<String, (String, String)>> = RefCell::new(HashMap::new());
}

/// Registers an application's Fluent catalogues. Messages with the same ID and
/// language override editor messages. Call this once per process before any editor
/// text is read, including labels resolved before the first frame.
pub fn add_application_catalogs(catalogs: &'static [(&'static str, &'static str)]) -> bool {
    APPLICATION.set(catalogs).is_ok()
}

/// Shows text in language `tag` from now on, on this thread.
pub fn set_language(tag: &str) {
    LANGUAGE.with_borrow_mut(|language| {
        if language != tag {
            tag.clone_into(language);
        }
    });
}

/// Names ports with `names` from now on, on this thread: the host's port names in
/// the current language ([`orange_host::Host::port_names`]).
pub fn set_port_names(names: Vec<(String, String)>) {
    PORTS.set(names.into_iter().collect());
}

/// The name of port `id`, such as `max_items`, in the current language, if the
/// host gave one.
pub(crate) fn port_name(id: &str) -> Option<String> {
    PORTS.with_borrow(|names| names.get(&id.replace('_', "-")).cloned())
}

pub(crate) fn set_outcome_names(names: Vec<(String, (String, String))>) {
    OUTCOMES.set(names.into_iter().collect());
}

pub(crate) fn outcome_name(definition: &str, fallback: &str) -> Option<String> {
    OUTCOMES.with_borrow(|names| {
        let (source, translated) = names.get(definition)?;
        (source == fallback).then(|| translated.clone())
    })
}

/// The language text is shown in on this thread.
pub fn language() -> String {
    LANGUAGE.with_borrow(Clone::clone)
}

/// The closest available editor locale, such as `es-MX` for a Spanish system.
pub fn nearest_language(tag: &str) -> String {
    let tag = tag.trim().replace('_', "-").to_ascii_lowercase();
    let mut candidate = tag.as_str();
    while !candidate.is_empty() {
        if TEXT.has(candidate, "language-name") {
            return preference_tag(candidate).into_owned();
        }
        candidate = candidate
            .rsplit_once('-')
            .map_or("", |(shorter, _)| shorter);
    }
    DEFAULT_LOCALE.to_owned()
}

/// Match every query term across the translated and source captions of a setting.
pub(crate) fn matches_messages(query: &str, ids: &[&str]) -> bool {
    if query.is_empty() {
        return true;
    }
    LANGUAGE.with_borrow(|locale| {
        let terms: Vec<_> = ids
            .iter()
            .flat_map(|id| [locale.as_str(), SOURCE_LANGUAGE].map(|tag| TEXT.message(tag, id)))
            .flatten()
            .map(|text| orange_host::fold_search(&text))
            .collect();
        query
            .split_whitespace()
            .all(|word| terms.iter().any(|text| text.contains(word)))
    })
}

/// Message `id` in the current language, the nearest one that has it, or the ID.
pub fn t(id: &'static str) -> &'static str {
    LANGUAGE.with_borrow(|language| match TEXT.message(language, id) {
        Some(Cow::Borrowed(text)) => text,
        Some(Cow::Owned(text)) => keep(text),
        None => id,
    })
}

/// Message `id` with named `args` in the current language.
pub fn tf(id: &str, args: &[(&str, Arg<'_>)]) -> String {
    LANGUAGE.with_borrow(|language| {
        TEXT.format(language, id, args)
            .map_or_else(|| id.to_owned(), Cow::into_owned)
    })
}

/// The editor's catalogues, for tests and the language list.
pub(crate) fn catalogs() -> &'static Catalogs {
    &TEXT
}

fn keep(text: String) -> &'static str {
    KEPT.with_borrow_mut(|kept| {
        if let Some(known) = kept.get(text.as_str()) {
            return *known;
        }
        let leaked: &'static str = Box::leak(text.into_boxed_str());
        kept.insert(leaked);
        leaked
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use orange_host::PSEUDO_LANGUAGE;

    /// Every `t("…")` and `tf("…"` in the source names an English message, and every
    /// other language uses English message IDs. Spanish ships with full coverage.
    #[test]
    fn every_message_the_code_names_is_in_every_language() {
        let english = catalogs().ids(SOURCE_LANGUAGE);
        let mut named = Vec::new();
        let source = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
        let mut folders = vec![std::path::PathBuf::from(source)];
        while let Some(folder) = folders.pop() {
            for entry in std::fs::read_dir(folder).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    folders.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let text = std::fs::read_to_string(&path).unwrap();
                    for call in ["t(\"", "tf(\""] {
                        for (at, _) in text.match_indices(call) {
                            let rest = &text[at + call.len()..];
                            let id = &rest[..rest.find('"').unwrap()];
                            let starts_word = at == 0
                                || !text[..at].ends_with(|c: char| c.is_alphanumeric() || c == '_');
                            if starts_word && !id.contains('…') {
                                named.push((path.display().to_string(), id.to_owned()));
                            }
                        }
                    }
                }
            }
        }
        let missing: Vec<_> = named
            .iter()
            .filter(|(_, id)| !english.contains(&id.as_str()))
            .collect();
        assert!(missing.is_empty(), "not in English: {missing:?}");
        for (tag, _) in catalogs().languages() {
            if tag == SOURCE_LANGUAGE || tag == PSEUDO_LANGUAGE {
                continue;
            }
            let ids = catalogs().ids(tag);
            let extra: Vec<_> = ids.iter().filter(|id| !english.contains(id)).collect();
            let untranslated: Vec<_> = english.iter().filter(|id| !ids.contains(id)).collect();
            assert!(
                extra.is_empty(),
                "{tag} has messages English lacks: {extra:?}"
            );
            if tag != "ja" {
                assert!(untranslated.is_empty(), "{tag} lacks {untranslated:?}");
            }
        }
    }

    #[test]
    fn text_follows_the_language_and_falls_back_to_english() {
        assert_eq!(t("cancel"), "Cancel");
        set_language("es-MX");
        assert_eq!(t("cancel"), "Cancelar");
        assert_eq!(
            tf("nodes-selected", &[("count", 3.into())]),
            "3 nodos seleccionados"
        );
        set_language(PSEUDO_LANGUAGE);
        assert!(t("cancel").starts_with("Cáñçél"));
        set_language(SOURCE_LANGUAGE);
        assert_eq!(
            tf("nodes-selected", &[("count", 1.into())]),
            "One node selected"
        );
        assert_eq!(nearest_language("es_MX"), "es-MX");
        assert_eq!(nearest_language("es-ES"), "es-MX");
        assert_eq!(nearest_language("EN-us"), "en-US");
        assert_eq!(preference_tag("ja_JP"), "ja-JP");
        assert_eq!(nearest_language("fr-FR"), "en-US");
        assert_eq!(preference_tag("zh_hant_tw"), "zh-Hant-TW");
    }

    #[test]
    fn japanese_preview_covers_the_editor_and_search_entry_points() {
        for id in [
            "language-name",
            "nodes",
            "search-hint",
            "quick-add-hint",
            "quick-add-wire-hint",
            "settings-title",
            "settings-search-hint",
            "shortcut-search",
            "properties",
            "configuration-required",
            "settings-node-search",
            "listen-search-words",
            "conversion-search-words",
            "catalog-flow",
            "catalog-services",
        ] {
            assert!(catalogs().has("ja", id), "Japanese preview lacks {id}");
        }
    }
}
