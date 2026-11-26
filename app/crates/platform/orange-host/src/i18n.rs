//! Translated text ([docs/13](../../../../../docs/13-internationalization.md)).
//!
//! A [`Catalogs`] holds Fluent catalogues per language. Text is looked up by message
//! ID in the exact language asked for, then in each shorter tag (`es-mx`, then `es`),
//! then in English, the source language every catalogue is written from. A message
//! may take named arguments and choose its words by plural category; a caller passes
//! the arguments, never pieces of a sentence to join.
//!
//! Each crate keeps its catalogues in `locales/<tag>/*.ftl`. Adding a language is
//! adding a folder: `app/build/locales.rs` lists the files at build time.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use fluent_bundle::{FluentArgs, FluentResource, FluentValue};
use fluent_syntax::ast;
use unic_langid::LanguageIdentifier;

/// The language catalogues are written in, and the last one every lookup tries.
pub const SOURCE_LANGUAGE: &str = "en";

/// A made-up language for checking layouts: English with accented letters and about
/// 40% more length, as translations often have. Never offered as a translation.
pub const PSEUDO_LANGUAGE: &str = "en-xa";

/// Canonical language, script and region casing for a catalogue locale.
/// Accepts underscores as separators for legacy OS and preference tags.
pub fn canonical_locale(tag: &str) -> Option<String> {
    tag.trim()
        .parse::<LanguageIdentifier>()
        .ok()
        .map(|id| id.to_string())
}

fn lookup_tag(tag: &str) -> String {
    tag.trim().replace('_', "-").to_ascii_lowercase()
}

type Bundle = fluent_bundle::concurrent::FluentBundle<Arc<FluentResource>>;

/// Fluent catalogues by lower-case language tag.
#[derive(Clone, Default)]
pub struct Catalogs {
    languages: BTreeMap<String, Language>,
    revision: u64,
}

#[derive(Clone)]
pub(crate) struct Language {
    id: LanguageIdentifier,
    resources: Vec<Arc<FluentResource>>,
    bundle: Arc<Bundle>,
    pseudo: bool,
}

/// A catalogue that could not be read, with the line it stopped at.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("line {line}: {reason}")]
pub struct CatalogError {
    pub line: usize,
    pub reason: String,
}

/// A named value a message places in its text or chooses its words by.
#[derive(Debug, Clone, PartialEq)]
pub enum Arg<'a> {
    Text(Cow<'a, str>),
    Number(f64),
}

impl<'a> From<&'a str> for Arg<'a> {
    fn from(text: &'a str) -> Self {
        Self::Text(Cow::Borrowed(text))
    }
}

impl From<String> for Arg<'_> {
    fn from(text: String) -> Self {
        Self::Text(Cow::Owned(text))
    }
}

impl From<f64> for Arg<'_> {
    fn from(number: f64) -> Self {
        Self::Number(number)
    }
}

macro_rules! counts {
    ($($kind:ty),*) => {$(
        impl From<$kind> for Arg<'_> {
            #[expect(
                clippy::cast_precision_loss,
                reason = "counts shown to a person are far below 2^52"
            )]
            fn from(count: $kind) -> Self {
                Self::Number(count as f64)
            }
        }
    )*};
}
counts!(usize, u64, i64);

impl From<u32> for Arg<'_> {
    fn from(count: u32) -> Self {
        Self::Number(f64::from(count))
    }
}

impl From<i32> for Arg<'_> {
    fn from(count: i32) -> Self {
        Self::Number(f64::from(count))
    }
}

impl fmt::Debug for Catalogs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Catalogs")
            .field("languages", &self.languages.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl Catalogs {
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    /// Catalogues read from `(language tag, Fluent text)` pairs, such as the list
    /// `app/build/locales.rs` makes. A pair that cannot be read is reported with its tag.
    pub fn from_sources(sources: &[(&str, &str)]) -> Result<Self, (String, CatalogError)> {
        let mut catalogs = Self::default();
        for (tag, text) in sources {
            catalogs
                .add(tag, text)
                .map_err(|error| ((*tag).to_owned(), error))?;
        }
        Ok(catalogs)
    }

    /// Adds the messages of `text` to language `locale`. A message already there is
    /// replaced. Nothing is added when the text has an error.
    pub fn add(&mut self, locale: &str, text: &str) -> Result<(), CatalogError> {
        let tag = lookup_tag(locale);
        let id: LanguageIdentifier = tag.parse().map_err(|_| CatalogError {
            line: 0,
            reason: "the language tag is not BCP 47 shaped".to_owned(),
        })?;
        let resource = FluentResource::try_new(text.to_owned()).map_err(|(_, errors)| {
            errors.first().map_or_else(
                || CatalogError {
                    line: 0,
                    reason: "the catalogue cannot be read".to_owned(),
                },
                |error| CatalogError {
                    line: line_of(text, error.pos.start),
                    reason: error.kind.to_string(),
                },
            )
        })?;
        let language = self
            .languages
            .entry(tag)
            .or_insert_with(|| Language::new(id, Vec::new(), false));
        language.resources.push(Arc::new(resource));
        language.rebuild();
        self.revision += 1;
        Ok(())
    }

    /// Adds [`PSEUDO_LANGUAGE`] from the English catalogues added so far.
    pub fn add_pseudo(&mut self) {
        if let Some(english) = self.languages.get(SOURCE_LANGUAGE) {
            let id = PSEUDO_LANGUAGE
                .parse()
                .unwrap_or_else(|_| english.id.clone());
            let pseudo = Language::new(id, english.resources.clone(), true);
            self.languages.insert(PSEUDO_LANGUAGE.to_owned(), pseudo);
            self.revision += 1;
        }
    }

    /// The languages there are, each with its name in itself (`language-name`).
    pub fn languages(&self) -> Vec<(&str, Cow<'_, str>)> {
        self.languages
            .iter()
            .map(|(tag, language)| {
                let name = language
                    .format("language-name", &[])
                    .unwrap_or(Cow::Borrowed(tag.as_str()));
                (tag.as_str(), name)
            })
            .collect()
    }

    /// Message `id` in `locale`, or in the nearest language that has it.
    pub fn message(&self, locale: &str, id: &str) -> Option<Cow<'_, str>> {
        self.format(locale, id, &[])
    }

    /// Message `id` in `locale`, or the nearest language that has it, with `args`.
    pub fn format(&self, locale: &str, id: &str, args: &[(&str, Arg<'_>)]) -> Option<Cow<'_, str>> {
        self.chain(locale)
            .find_map(|language| language.format(id, args))
    }

    /// Whether language `tag` itself, without falling back, has message `id`.
    pub fn has(&self, tag: &str, id: &str) -> bool {
        self.languages
            .get(&lookup_tag(tag))
            .is_some_and(|language| language.bundle.has_message(id))
    }

    /// The message IDs language `tag` itself has, sorted.
    pub fn ids(&self, tag: &str) -> Vec<&str> {
        let mut ids: Vec<&str> = self
            .languages
            .get(&lookup_tag(tag))
            .into_iter()
            .flat_map(Language::ids)
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    /// A search caption from the requested language or its regional parents.
    pub fn localized_message(&self, locale: &str, id: &str) -> Option<Cow<'_, str>> {
        let source = lookup_tag(locale).split('-').next() == Some(SOURCE_LANGUAGE);
        self.chain(locale)
            .filter(|language| source || !language.is_source())
            .find_map(|language| language.format(id, &[]))
    }

    /// The languages a lookup in `locale` tries, nearest first, each once.
    pub(crate) fn chain(&self, locale: &str) -> impl Iterator<Item = &Language> {
        let locale = lookup_tag(locale);
        let mut tags: Vec<String> = Vec::new();
        let mut tag = locale.as_str();
        while !tag.is_empty() {
            tags.push(tag.to_owned());
            tag = tag.rsplit_once('-').map_or("", |(shorter, _)| shorter);
        }
        if !tags.iter().any(|known| known == SOURCE_LANGUAGE) {
            tags.push(SOURCE_LANGUAGE.to_owned());
        }
        tags.into_iter().filter_map(|tag| self.languages.get(&tag))
    }
}

impl Language {
    pub(crate) fn is_source(&self) -> bool {
        self.id.language.as_str() == SOURCE_LANGUAGE && !self.pseudo
    }

    /// The IDs of the messages of this language itself, in file order.
    pub(crate) fn ids(&self) -> impl Iterator<Item = &str> {
        self.resources
            .iter()
            .flat_map(|resource| resource.entries())
            .filter_map(|entry| match entry {
                ast::Entry::Message(message) => Some(message.id.name),
                _ => None,
            })
    }

    fn new(id: LanguageIdentifier, resources: Vec<Arc<FluentResource>>, pseudo: bool) -> Self {
        let mut language = Self {
            bundle: Arc::new(Bundle::new_concurrent(vec![id.clone()])),
            id,
            resources,
            pseudo,
        };
        language.rebuild();
        language
    }

    fn rebuild(&mut self) {
        let mut bundle = Bundle::new_concurrent(vec![self.id.clone()]);
        // The editor draws text without bidirectional isolation marks.
        bundle.set_use_isolating(false);
        if self.pseudo {
            bundle.set_transform(Some(pseudo));
        }
        for resource in &self.resources {
            bundle.add_resource_overriding(Arc::clone(resource));
        }
        self.bundle = Arc::new(bundle);
    }

    /// Message `id` of this language alone.
    pub(crate) fn format(&self, id: &str, args: &[(&str, Arg<'_>)]) -> Option<Cow<'_, str>> {
        let pattern = self.bundle.get_message(id)?.value()?;
        let args = (!args.is_empty()).then(|| {
            let mut named = FluentArgs::with_capacity(args.len());
            for (name, value) in args {
                match value {
                    Arg::Text(text) => named.set(*name, FluentValue::from(text.to_string())),
                    Arg::Number(number) => named.set(*name, FluentValue::from(*number)),
                }
            }
            named
        });
        let mut errors = Vec::new();
        Some(
            self.bundle
                .format_pattern(pattern, args.as_ref(), &mut errors),
        )
    }
}

/// The line of byte `at` in `text`, from 1.
fn line_of(text: &str, at: usize) -> usize {
    text.get(..at)
        .map_or(0, |before| before.matches('\n').count())
        + 1
}

fn pseudo(text: &str) -> Cow<'_, str> {
    let mut out = String::with_capacity(text.len() * 2);
    let mut letters = 0;
    for letter in text.chars() {
        letters += usize::from(letter.is_alphabetic());
        out.push(match letter {
            'a' => 'á',
            'e' => 'é',
            'i' => 'í',
            'o' => 'ö',
            'u' => 'ü',
            'n' => 'ñ',
            'c' => 'ç',
            'A' => 'Å',
            'E' => 'É',
            'O' => 'Ø',
            'U' => 'Ü',
            other => other,
        });
    }
    out.extend(std::iter::repeat_n('~', letters * 2 / 5));
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EN: &str = "
language-name = English
greeting = Hello
selected =
    { $count ->
        [one] One node selected
       *[other] { $count } nodes selected
    }
";
    const ES: &str = "
language-name = Español
greeting = Hola
selected =
    { $count ->
        [one] Un nodo seleccionado
       *[other] { $count } nodos seleccionados
    }
";

    fn catalogs() -> Catalogs {
        let mut catalogs = Catalogs::from_sources(&[("en", EN), ("es", ES)]).unwrap();
        catalogs.add_pseudo();
        catalogs
    }

    #[test]
    fn a_lookup_falls_back_to_shorter_tags_then_english() {
        let catalogs = catalogs();
        assert_eq!(
            catalogs.message("es-MX", "greeting").as_deref(),
            Some("Hola")
        );
        assert_eq!(catalogs.message("fr", "greeting").as_deref(), Some("Hello"));
        assert_eq!(catalogs.message("fr", "missing"), None);
        assert!(catalogs.has("es", "greeting") && !catalogs.has("es-mx", "greeting"));
    }

    #[test]
    fn messages_choose_words_by_plural_category() {
        let catalogs = catalogs();
        let selected = |locale, count: usize| {
            catalogs
                .format(locale, "selected", &[("count", count.into())])
                .unwrap()
                .into_owned()
        };
        assert_eq!(selected("en", 1), "One node selected");
        assert_eq!(selected("en", 3), "3 nodes selected");
        assert_eq!(selected("es", 2), "2 nodos seleccionados");
    }

    #[test]
    fn languages_name_themselves_and_the_pseudo_language_is_longer() {
        let catalogs = catalogs();
        let names = catalogs.languages();
        assert!(names.contains(&("es", Cow::Borrowed("Español"))));
        let pseudo = catalogs.message(PSEUDO_LANGUAGE, "greeting").unwrap();
        assert!(pseudo.starts_with("Héllö") && pseudo.len() > "Hello".len());
        assert_eq!(
            catalogs.ids("es"),
            ["greeting", "language-name", "selected"]
        );
    }

    #[test]
    fn a_catalogue_with_an_error_adds_nothing() {
        let mut catalogs = Catalogs::default();
        assert_eq!(
            catalogs
                .add("en", "ok = fine\n\nBad Id = x")
                .unwrap_err()
                .line,
            3
        );
        assert!(catalogs.add("not a tag", "ok = x").is_err());
        assert!(catalogs.languages().is_empty());
    }
}
