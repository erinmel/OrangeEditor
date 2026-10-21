//! Structural checks of the G-suite fixtures in `conformance/` (WU-00/00a). They prove the
//! fixture structure, package snapshots and rule mappings agree. Expected diagnostics
//! and traces are NOT executed here: that needs the validator and interpreter (00d–00f).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use orange_model::document::{NodeKind, ResolvedPort};
use orange_model::from_json;
use orange_model::signature;
use serde_json::Value as Json;
use sha2::{Digest, Sha256};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn json(path: &Path) -> Json {
    serde_json::from_str(&read(path)).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Fixture files by G id, from `conformance/g-suite/G-NN-<slug>.json`.
fn fixtures() -> BTreeMap<String, (PathBuf, Json)> {
    let dir = repo().join("conformance/g-suite");
    let mut out = BTreeMap::new();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let bytes = name.as_bytes();
        assert!(
            bytes.len() > 10
                && name.ends_with(".json")
                && bytes.starts_with(b"G-")
                && bytes[2..4].iter().all(u8::is_ascii_digit)
                && bytes[4] == b'-',
            "{name}: expected G-NN-<slug>.json"
        );
        let id = name[..4].to_owned();
        let fixture = json(&path);
        assert!(
            out.insert(id, (path, fixture)).is_none(),
            "duplicate fixture for {name}"
        );
    }
    out
}

/// G ids listed in the 11 §18 table, and diagnostic codes listed in the 11 §17 catalog.
fn doc_catalogs() -> (BTreeSet<String>, BTreeSet<String>) {
    let doc = read(&repo().join("docs/11-graph-language.md"));
    let section = |start: &str, end: &str| {
        doc.split(start)
            .nth(1)
            .unwrap()
            .split(end)
            .next()
            .unwrap()
            .to_owned()
    };
    let suite = section("## 18. Conformance suite", "Requirements:");
    let ids = suite
        .lines()
        .filter_map(|l| l.strip_prefix("| G-"))
        .map(|rest| format!("G-{}", &rest[..2]))
        .collect();
    let catalog = section("## 17. Diagnostics catalog", "## 18.");
    let codes = catalog
        .split('`')
        .filter(|token| (token.starts_with("E-") || token.starts_with("W-")) && token.len() > 2)
        .map(str::to_owned)
        .collect();
    (ids, codes)
}

fn package() -> (Json, String) {
    let path = repo().join("conformance/packages/org.orange.test.json");
    let digest = Sha256::digest(std::fs::read(&path).unwrap());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    (json(&path), format!("sha256:{hex}"))
}

#[test]
fn every_fixture_is_well_formed() {
    let (package, digest) = package();
    let definitions: BTreeSet<&str> = package["definitions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["definition_id"].as_str().unwrap())
        .collect();
    let rules = json(&repo().join("conformance/rules.json"));
    let rule_ids: BTreeSet<&str> = rules["rules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    let (_, codes) = doc_catalogs();

    for (id, (path, fixture)) in fixtures() {
        let at = path.display();
        assert_eq!(fixture["format"], "orange.conformance", "{at}");
        assert_eq!(fixture["format_version"], 1, "{at}");
        assert_eq!(
            fixture["id"],
            id.as_str(),
            "{at}: id must match the file name"
        );
        for rule in fixture["rules"]
            .as_array()
            .unwrap_or_else(|| panic!("{at}: rules"))
        {
            assert!(
                rule_ids.contains(rule.as_str().unwrap()),
                "{at}: unknown rule {rule}"
            );
        }
        let cases = fixture["cases"]
            .as_array()
            .unwrap_or_else(|| panic!("{at}: cases"));
        assert!(!cases.is_empty(), "{at}: no cases");
        let mut names = BTreeSet::new();
        for case in cases {
            let name = case["name"]
                .as_str()
                .unwrap_or_else(|| panic!("{at}: case name"));
            assert!(names.insert(name), "{at}: duplicate case {name}");
            let at = format!("{at} / {name}");
            let document =
                from_json(&case["document"].to_string()).unwrap_or_else(|e| panic!("{at}: {e}"));
            assert!(
                case["expect"]["diagnostics"].is_array() || case["expect"]["trace"].is_object(),
                "{at}: expect needs diagnostics or a trace"
            );
            assert_eq!(
                case["run"].is_object(),
                case["expect"]["trace"].is_object(),
                "{at}: run and trace go together"
            );
            if case["expect"]["behaviour"].is_object() && !has_structured_expectation(case) {
                for field in ["owner", "reason"] {
                    assert!(
                        case["deferred"][field]
                            .as_str()
                            .is_some_and(|s| !s.trim().is_empty()),
                        "{at}: prose-only assertions need deferred.{field}"
                    );
                }
            }
            for diagnostic in case["expect"]["diagnostics"]
                .as_array()
                .into_iter()
                .flatten()
            {
                let code = diagnostic["code"].as_str().unwrap();
                assert!(
                    codes.contains(code),
                    "{at}: {code} is not in the 11 §17 catalog"
                );
            }
            let mut uses_test_package = false;
            for body in document.bodies() {
                for node in body.nodes.values() {
                    if node.definition.id.is_core() {
                        continue;
                    }
                    uses_test_package = true;
                    let definition = node.definition.id.as_str();
                    assert!(
                        definitions.contains(definition),
                        "{at}: {definition} is not in org.orange.test"
                    );
                    let key = node
                        .signature
                        .as_ref()
                        .unwrap_or_else(|| panic!("{at}: {} has no signature", node.id));
                    assert!(
                        document.signatures().contains_key(key),
                        "{at}: missing snapshot {key}"
                    );
                    assert_eq!(
                        node.provider.as_ref().map(|p| p.package.as_str()),
                        Some("org.orange.test"),
                        "{at}"
                    );
                    let provider = node.provider.as_ref().unwrap();
                    assert_eq!(provider.version, "0.1.0", "{at}: provider version");
                    assert_eq!(provider.id, "reference", "{at}: provider id");
                }
            }
            if uses_test_package {
                let lock = document
                    .package_lock
                    .iter()
                    .find(|l| l.id == "org.orange.test");
                assert_eq!(
                    lock.map(|l| l.content_digest.as_str()),
                    Some(digest.as_str()),
                    "{at}: stale package digest"
                );
            }
        }
    }
}

fn has_structured_expectation(case: &Json) -> bool {
    let expected = &case["expect"];
    expected["diagnostics"]
        .as_array()
        .is_some_and(|d| !d.is_empty())
        || [
            "trace",
            "registry",
            "document",
            "choice",
            "exports",
            "change_set",
        ]
        .iter()
        .any(|field| !expected[field].is_null())
}

/// Check definition → type substitution → stored snapshot independently of the future
/// validator. Intentional drift must be asserted on that exact node, never skipped for
/// an entire fixture file.
#[test]
fn package_signatures_match_their_definitions() {
    let (package, _) = package();
    let definitions: BTreeMap<&str, (NodeKind, Vec<ResolvedPort>)> = package["definitions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|definition| {
            (
                definition["definition_id"].as_str().unwrap(),
                (
                    serde_json::from_value(definition["kind"].clone()).unwrap(),
                    serde_json::from_value(definition["ports"].clone()).unwrap(),
                ),
            )
        })
        .collect();
    let mut failures = Vec::new();
    for (id, (_, fixture)) in fixtures() {
        for case in fixture["cases"].as_array().unwrap() {
            let document = from_json(&case["document"].to_string()).unwrap();
            for body in document.bodies() {
                for node in body.nodes.values() {
                    let Some((kind, ports)) = definitions.get(node.definition.id.as_str()) else {
                        continue;
                    };
                    let stored = &document.signatures()[node.signature.as_ref().unwrap()];
                    let projected = signature::project(*kind, ports.clone(), &node.type_args);
                    let drift = signature::compare(stored, &projected);
                    let expects_drift = case["expect"]["diagnostics"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|d| {
                            d["code"] == "E-SIGNATURE-DRIFT"
                                && d["subject"]["node"].as_str() == Some(&node.id.to_string())
                        });
                    if drift.is_some() != expects_drift {
                        failures.push(format!("{id} / {} / {}: {drift:?}", case["name"], node.id));
                    }
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "snapshot mismatches:\n{}",
        failures.join("\n")
    );
}

#[test]
fn every_g_case_and_rule_is_mapped_in_both_directions() {
    let (suite, _) = doc_catalogs();
    let fixtures = fixtures();
    let files: BTreeSet<&String> = fixtures.keys().collect();
    let missing: Vec<&String> = suite.iter().filter(|id| !files.contains(id)).collect();
    assert!(missing.is_empty(), "G cases without a fixture: {missing:?}");
    let extra: Vec<&&String> = files.iter().filter(|id| !suite.contains(**id)).collect();
    assert!(
        extra.is_empty(),
        "fixtures without an 11 §18 entry: {extra:?}"
    );

    let rules = json(&repo().join("conformance/rules.json"));
    let mut rule_ids = BTreeSet::new();
    for rule in rules["rules"].as_array().unwrap() {
        assert!(
            rule_ids.insert(rule["id"].as_str().unwrap()),
            "duplicate rule {}",
            rule["id"]
        );
        let cases = rule["cases"].as_array().unwrap();
        assert!(!cases.is_empty(), "{} has no case", rule["id"]);
        for case in cases {
            assert!(
                suite.contains(case.as_str().unwrap()),
                "{}: {case} is not in 11 §18",
                rule["id"]
            );
        }
    }
    for (id, (_, fixture)) in &fixtures {
        let actual: Vec<&str> = fixture["rules"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r.as_str().unwrap())
            .collect();
        let expected: BTreeSet<&str> = rules["rules"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["cases"].as_array().unwrap().iter().any(|c| c == id))
            .map(|r| r["id"].as_str().unwrap())
            .collect();
        assert_eq!(
            actual,
            expected.into_iter().collect::<Vec<_>>(),
            "{id}: rules must be the sorted inverse mapping"
        );
    }
}

fn nanoseconds(value: &Json, at: &str) -> u64 {
    let text = value
        .as_str()
        .unwrap_or_else(|| panic!("{at}: time must be a decimal string"));
    assert!(
        !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()),
        "{at}: invalid time {text}"
    );
    text.parse()
        .unwrap_or_else(|e| panic!("{at}: time out of range: {e}"))
}

/// This checks the fixture script, not a scheduler. In particular, traces cannot rely
/// on an implicit clock advance that a future runner would have to guess.
#[test]
fn run_scripts_and_replay_expectations_are_consistent() {
    for (id, (_, fixture)) in fixtures() {
        for case in fixture["cases"].as_array().unwrap() {
            let at = format!("{id} / {}", case["name"]);
            if !case["run"].is_object() {
                assert!(
                    case["replay"].is_null(),
                    "{at}: replay needs a recording run"
                );
                continue;
            }
            let mut clock = 0_u64;
            for step in case["run"]["steps"].as_array().unwrap() {
                let fields = step.as_object().unwrap();
                assert_eq!(fields.len(), 1, "{at}: each step has exactly one operation");
                let (op, value) = fields.iter().next().unwrap();
                match op.as_str() {
                    "advance" | "stall" => {
                        clock = clock
                            .checked_add(nanoseconds(value, &at))
                            .expect("clock overflow")
                    }
                    "trigger" | "cancel" | "edit" => assert!(value.is_object(), "{at}: {op}"),
                    "run" => assert!(value.as_u64().is_some_and(|n| n > 0), "{at}: run number"),
                    "stop" => assert!(
                        matches!(value.as_str(), Some("graceful" | "cancel")),
                        "{at}: stop mode"
                    ),
                    _ => panic!("{at}: unknown run operation {op}"),
                }
            }
            assert!(
                matches!(
                    case["expect"]["trace"]["match"].as_str(),
                    Some("exact" | "subsequence")
                ),
                "{at}: trace match mode"
            );
            let events = case["expect"]["trace"]["events"].as_array().unwrap();
            assert!(
                !events.is_empty(),
                "{at}: a runtime fixture needs observable events"
            );
            let mut previous = 0;
            for event in events {
                let time = nanoseconds(&event["t"], &at);
                assert!(
                    time >= previous && time <= clock,
                    "{at}: trace time {time} outside script clock 0..{clock} or out of order"
                );
                previous = time;
            }
            assert_eq!(
                case["replay"].is_object(),
                case["expect"]["replay_run"].is_object(),
                "{at}: replay request and expectation go together"
            );
            if case["replay"].is_object() {
                assert_eq!(case["replay"]["source"], "recorded-run", "{at}");
                assert_eq!(case["expect"]["replay_run"]["trace"], "identical", "{at}");
                assert_eq!(
                    case["expect"]["replay_run"]["counters"]["remote_sends"], 0,
                    "{at}: replay must never resend effects"
                );
            }
        }
    }
}

fn json_examples(document: &str) -> Vec<Json> {
    read(&repo().join(document))
        .split("```json")
        .skip(1)
        .map(|block| {
            serde_json::from_str(block.split("```").next().unwrap().trim())
                .unwrap_or_else(|e| panic!("{document}: JSON example: {e}"))
        })
        .collect()
}

#[test]
fn documentation_json_examples_parse_and_repeated_definitions_agree() {
    let data = json_examples("docs/03-data-and-execution.md");
    let _language = json_examples("docs/11-graph-language.md");
    let layout = json_examples("docs/12-graph-data-structures.md");
    let add = |blocks: Vec<Json>| {
        blocks
            .into_iter()
            .find(|block| block["definition_id"] == "org.orange.math/add")
            .unwrap()
    };
    assert_eq!(
        add(data),
        add(layout),
        "03 and 12 must describe the same definition"
    );
}
