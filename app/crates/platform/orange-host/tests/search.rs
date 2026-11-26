//! The node search: ranking, concepts and languages (docs/22 §4).
use orange_host::{Host, HostOptions, Match};
use orange_model::document::NodeKind;

fn host() -> Host {
    Host::new(HostOptions::default()).unwrap().0
}

fn ids(found: &[orange_host::Found]) -> Vec<&str> {
    found
        .iter()
        .map(|found| found.definition.as_str())
        .collect()
}

#[test]
fn results_rank_exact_names_and_aliases_above_prefixes_and_concepts() {
    let host = host();
    let first = |search: &str| host.search_nodes(search, "en")[0].clone();
    assert_eq!(first("wait").matched, Some(Match::Exact));
    assert_eq!(first("wait").definition, "core/wait");
    assert_eq!(first("on ma").definition, "core/on-manual");
    assert_eq!(first("on ma").matched, Some(Match::NameStart));
    assert_eq!(first("integer").definition, "core/whole-number");
    assert_eq!(first("integer").matched, Some(Match::AliasExact));
    // "timer" is a word of On timer's name, before nodes that are only about time.
    let timer = host.search_nodes("timer", "en");
    assert_eq!(timer[0].definition, "core/on-timer");
    assert!(
        timer
            .iter()
            .any(|found| found.definition == "core/wait" && found.matched == Some(Match::Concept))
    );
    // An ID word is a description match; one typo still finds the node, last.
    assert_eq!(
        host.search_nodes("orange.math", "en")[0].matched,
        Some(Match::Description)
    );
    let typo = host.search_nodes("tiemr", "en");
    assert!(ids(&typo).contains(&"core/on-timer"));
    assert!(typo.iter().all(|found| found.matched == Some(Match::Typo)));
    assert!(host.search_nodes("zzzz qqqq", "en").is_empty());
    // Ranks never go down the list.
    let all = host.search_nodes("a", "en");
    assert!(
        all.windows(2)
            .all(|pair| pair[0].matched <= pair[1].matched)
    );
}

#[test]
fn trigger_words_find_every_entry_kind_in_english_and_spanish() {
    let host = host();
    let entries: Vec<String> = host
        .nodes()
        .filter(|info| info.definition.kind == NodeKind::Entry)
        .map(|info| info.definition.definition.id.as_str().to_owned())
        .collect();
    assert!(entries.len() >= 5);
    for (search, locale) in [
        ("trigger", "en"),
        ("when", "en"),
        ("event", "en"),
        ("disparador", "es"),
        ("cuando", "es"),
    ] {
        let found = host.search_nodes(search, locale);
        for entry in &entries {
            assert!(
                ids(&found).contains(&entry.as_str()),
                "{search} misses {entry}"
            );
        }
    }
    // Each result carries a category and a one-line purpose in the search's language.
    let manual = host
        .search_nodes("disparador", "es")
        .into_iter()
        .find(|found| found.definition == "core/on-manual")
        .unwrap();
    assert_eq!(manual.category, "Disparadores");
    assert_eq!(manual.purpose, "Empieza al pulsar Ejecutar.");
    let manual = &host.search_nodes("on manual", "en")[0];
    assert_eq!(manual.purpose, "Starts when you press Run.");
}

#[test]
fn spanish_words_without_accents_find_nodes_and_english_still_works() {
    let host = host();
    let first =
        |search: &str, locale: &str| host.search_nodes(search, locale)[0].definition.clone();
    assert_eq!(first("temporizador", "es"), "core/on-timer");
    assert_eq!(first("numero entero", "es"), "core/whole-number");
    assert_eq!(first("número entero", "es-MX"), "core/whole-number");
    assert_eq!(first("sumar", "es"), "org.orange.math/add");
    assert_eq!(first("integer", "es"), "core/whole-number");
    let text = host.search_nodes("texto", "es");
    assert!(ids(&text).contains(&"org.orange.text/trim"));
    // English searches do not see Spanish words.
    assert!(!ids(&host.search_nodes("temporizador", "en")).contains(&"core/on-timer"));
}

#[test]
fn regional_search_accepts_japanese_substrings_and_localized_configuration() {
    let host = host();
    for (query, locale, id) in [
        ("足し算", "ja-JP", "org.orange.math/add"),
        (
            "リストの長さ",
            "ja-JP",
            "org.orange.collections/list-length",
        ),
        ("フィールドを取得", "ja-JP", "org.orange.json/get"),
        ("ＪＳＯＮ パス", "ja-JP", "org.orange.json/get-path"),
        ("integer", "ja-JP", "core/whole-number"),
        ("nu\u{301}mero entero", "es-MX", "core/whole-number"),
    ] {
        assert!(
            ids(&host.search_nodes(query, locale)).contains(&id),
            "{query} misses {id}"
        );
    }
}

#[test]
fn concepts_group_nodes_by_purpose() {
    let host = host();
    let has = |search: &str, id: &str| ids(&host.search_nodes(search, "en")).contains(&id);
    assert!(has("compare", "org.orange.math/greater"));
    assert!(!has("compare", "org.orange.math/add"));
    assert!(has("math", "org.orange.math/add"));
    assert!(has("loop", "core/for-each"));
    assert_eq!(
        ids(&host.search_nodes("for loop", "en")).first(),
        Some(&"core/count")
    );
    assert_eq!(
        ids(&host.search_nodes("bucle for", "es")).first(),
        Some(&"core/count")
    );
    assert!(has("loop", "core/while"));
    assert!(has("variable", "core/set-variable"));
    assert!(has("variable", "core/on-variable-changed"));
    assert!(has("time", "core/now"));
    assert!(has("text", "org.orange.text/split"));
    // Every word typed must match.
    assert!(has("math add", "org.orange.math/add"));
    assert!(!has("math add", "org.orange.math/subtract"));
}

#[test]
fn a_language_adds_words_without_changing_ids() {
    let mut host = host();
    assert!(host.search_nodes("déclencheur", "fr").is_empty());
    host.add_catalog(
        "fr",
        "concept-trigger = déclencheur, quand\nalias-core-wait = attendre",
    )
    .unwrap();
    let found = host.search_nodes("declencheur", "fr");
    assert!(ids(&found).contains(&"core/on-timer"));
    assert_eq!(
        host.search_nodes("attendre", "fr")[0].definition,
        "core/wait"
    );
    // Purposes fall back to English.
    assert!(!found[0].purpose.is_empty());
    assert!(host.add_catalog("fr", "no equals sign").is_err());
}

#[test]
fn node_category_and_port_names_follow_the_language_and_fall_back_to_english() {
    let host = host();
    let found = |search: &str, locale: &str, definition: &str| {
        host.search_nodes(search, locale)
            .into_iter()
            .find(|found| found.definition == definition)
            .unwrap()
    };
    let manual = found("", "es-MX", "core/on-manual");
    assert_eq!(
        (manual.label.as_str(), manual.category.as_str()),
        ("Manual", "Disparadores")
    );
    let add = found("", "es", "org.orange.math/add");
    assert_eq!(
        (add.label.as_str(), add.category.as_str()),
        ("Sumar", "Matemáticas")
    );
    let english = found("", "en", "org.orange.math/add");
    assert_eq!(
        (english.label.as_str(), english.category.as_str()),
        ("Add", "Math")
    );
    // A category keeps its search words across languages: "datos" finds JSON nodes.
    assert!(ids(&host.search_nodes("datos", "es")).contains(&"org.orange.json/get"));
    let ports = host.port_names("es");
    assert!(ports.contains(&("result".to_owned(), "resultado".to_owned())));
    assert!(ports.contains(&("failed".to_owned(), "Falló".to_owned())));
    assert!(ports.contains(&("cancelled".to_owned(), "Cancelado".to_owned())));
    assert!(
        host.port_names("en")
            .contains(&("completed".to_owned(), "Done".to_owned()))
    );
}
