//! A saved graph is untrusted input. Corrupted copies of the shipped example graphs go
//! through checking, drafting, opening and running their manual entries, and each path
//! answers with a value or a typed error: nothing panics on any thread.
//!
//! `ORANGE_HOSTILE_CASES` raises the number of corrupted copies per graph (default 40) and
//! `ORANGE_HOSTILE_SEED` replays one reported case.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use orange_host::{Host, HostOptions};
use serde_json::{Value as Json, json};

fn seeds() -> Vec<(String, Json)> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../apps/orange-cli/graphs");
    let mut graphs: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .map(|path| {
            let text = std::fs::read_to_string(&path).unwrap();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            (name, serde_json::from_str(&text).unwrap())
        })
        .collect();
    graphs.sort_by(|a, b| a.0.cmp(&b.0));
    assert!(!graphs.is_empty(), "example graphs are found");
    graphs
}

/// A small deterministic generator, so a failing seed replays exactly.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % n.max(1) as u64).unwrap()
    }
}

/// Every value in the tree with its path, parents before children.
fn paths(value: &Json, prefix: &mut Vec<Json>, out: &mut Vec<Vec<Json>>) {
    out.push(prefix.clone());
    match value {
        Json::Object(map) => {
            for (key, child) in map {
                prefix.push(json!(key));
                paths(child, prefix, out);
                prefix.pop();
            }
        }
        Json::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                prefix.push(json!(index));
                paths(child, prefix, out);
                prefix.pop();
            }
        }
        _ => {}
    }
}

fn at<'a>(value: &'a mut Json, path: &[Json]) -> &'a mut Json {
    path.iter().fold(value, |node, step| match step {
        Json::String(key) => &mut node[key.as_str()],
        Json::Number(index) => &mut node[usize::try_from(index.as_u64().unwrap()).unwrap()],
        _ => unreachable!(),
    })
}

fn get<'a>(value: &'a Json, path: &[Json]) -> &'a Json {
    path.iter().fold(value, |node, step| match step {
        Json::String(key) => &node[key.as_str()],
        Json::Number(index) => &node[usize::try_from(index.as_u64().unwrap()).unwrap()],
        _ => unreachable!(),
    })
}

/// Replacements chosen to reach edge cases: wrong kinds, extreme numbers, odd text.
fn hostile(rng: &mut Rng, donor: &Json) -> Json {
    match rng.below(14) {
        0 => Json::Null,
        1 => json!(true),
        2 => json!(-1),
        3 => json!(i64::MAX),
        4 => json!(i64::MIN),
        5 => json!(u64::MAX),
        6 => json!(1e308),
        7 => json!(""),
        8 => json!("\u{0}\u{202e}ñ😀"),
        9 => json!("x".repeat(4096)),
        10 => json!([]),
        11 => json!({}),
        12 => json!([donor.clone(), donor.clone()]),
        _ => donor.clone(),
    }
}

fn corrupt(rng: &mut Rng, seed: &Json) -> Json {
    let mut graph = seed.clone();
    for _ in 0..=rng.below(3) {
        let mut all = Vec::new();
        paths(&graph, &mut Vec::new(), &mut all);
        all.remove(0); // keep the root an object so the envelope is reached
        let target = all[rng.below(all.len())].clone();
        // Splice a value from elsewhere in the same graph: ids, types and ports that are
        // valid in one place and wrong in another. Mostly a value of the same JSON kind,
        // so the document still parses and the later stages are reached.
        let kind = std::mem::discriminant(get(&graph, &target));
        let same: Vec<_> = all
            .iter()
            .filter(|path| std::mem::discriminant(get(&graph, path)) == kind)
            .collect();
        let donor_path = if rng.below(4) == 0 || same.is_empty() {
            &all[rng.below(all.len())]
        } else {
            same[rng.below(same.len())]
        };
        let donor = get(&graph, donor_path).clone();
        if rng.below(3) != 0 && std::mem::discriminant(&donor) == kind {
            *at(&mut graph, &target) = donor;
            continue;
        }
        let (last, parent_path) = target.split_last().unwrap();
        let parent = at(&mut graph, parent_path);
        match (rng.below(4), parent, last) {
            (0, Json::Object(map), Json::String(key)) => {
                map.remove(key);
            }
            (0, Json::Array(items), Json::Number(index)) => {
                items.remove(usize::try_from(index.as_u64().unwrap()).unwrap());
            }
            (1, Json::Array(items), Json::Number(index)) => {
                let index = usize::try_from(index.as_u64().unwrap()).unwrap();
                let copy = items[index].clone();
                items.insert(index, copy);
            }
            (_, parent, _) => {
                let slot = match last {
                    Json::String(key) => &mut parent[key.as_str()],
                    Json::Number(index) => {
                        &mut parent[usize::try_from(index.as_u64().unwrap()).unwrap()]
                    }
                    _ => unreachable!(),
                };
                *slot = hostile(rng, &donor);
            }
        }
    }
    graph
}

/// How far one document got: checked without a host error, drafted, opened as a flow.
#[derive(Default, Debug)]
struct Reach {
    checked: u32,
    clean: u32,
    drafted: u32,
    opened: u32,
}

fn exercise(host: &mut Host, text: &str, reach: &mut Reach) {
    if let Ok(diagnostics) = host.check(text) {
        reach.checked += 1;
        reach.clean += u32::from(diagnostics.is_empty());
    }
    if let Ok(draft) = host.draft(text) {
        reach.drafted += 1;
        let _ = draft.text();
    }
    if let Ok(flow) = host.open(text) {
        reach.opened += 1;
        // Fire every manual entry so the runtime executes the corrupted graph.
        let graph: Json = serde_json::from_str(text).unwrap();
        let manual = graph["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|node| {
                (node["definition_id"] == "core/on-manual")
                    .then(|| node["id"].as_str()?.parse().ok())
                    .flatten()
            });
        for node in manual {
            let _ = host.trigger(flow, node, None);
        }
        std::thread::sleep(Duration::from_millis(2));
        let _ = host.close(flow);
    }
}

/// Panics anywhere, including flow and worker threads where the host turns them into
/// faults, are recorded so the test still sees them.
static PANICS: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn record_panics() {
    std::panic::set_hook(Box::new(|info| {
        let thread = std::thread::current().name().unwrap_or("?").to_owned();
        PANICS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(format!("[{thread}] {info}"));
    }));
}

fn take_panics() -> Vec<String> {
    std::mem::take(&mut *PANICS.lock().unwrap_or_else(PoisonError::into_inner))
}

#[test]
fn corrupted_saved_graphs_fail_with_typed_errors() {
    let cases: u64 = std::env::var("ORANGE_HOSTILE_CASES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(40);
    let only: Option<u64> = std::env::var("ORANGE_HOSTILE_SEED")
        .ok()
        .and_then(|value| value.parse().ok());
    record_panics();
    let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
    let mut failures = Vec::new();
    let mut reach = Reach::default();
    for (graph_index, (name, seed)) in seeds().iter().enumerate() {
        // The unmodified graph must not panic either.
        exercise(&mut host, &seed.to_string(), &mut Reach::default());
        for panic in take_panics() {
            failures.push(format!("{name} unmodified: {panic}"));
        }
        for case in 0..cases {
            let key = (graph_index as u64) << 32 | case;
            if only.is_some_and(|only| only != key) {
                continue;
            }
            let mut rng = Rng(key.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            let text = corrupt(&mut rng, seed).to_string();
            let unwound = catch_unwind(AssertUnwindSafe(|| {
                exercise(&mut host, &text, &mut reach);
            }))
            .is_err();
            for panic in take_panics() {
                failures.push(format!("{name} ORANGE_HOSTILE_SEED={key}: {panic}"));
            }
            if unwound {
                // A panic may leave the host inconsistent; continue from a fresh one.
                host = Host::new(HostOptions::default()).unwrap().0;
            }
        }
    }
    let _ = std::panic::take_hook();
    eprintln!("{reach:?}");
    assert!(
        reach.opened > 0,
        "some corrupted graphs still open: {reach:?}"
    );
    assert!(failures.is_empty(), "panics:\n{}", failures.join("\n"));
}
