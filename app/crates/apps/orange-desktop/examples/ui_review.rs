use orange_host::{Host, HostOptions};
use std::{collections::BTreeMap, fs};

fn new_document() -> String {
    let mut document: serde_json::Value =
        serde_json::from_str(&orange_host::new_document().unwrap()).unwrap();
    document["types"] = serde_json::json!([
        {"id":"30000000-0000-4000-8000-000000000011","name":"Review choice","kind":"enum","variants":[{"id":"value","name":"Value","payload":"core/i64@1"}]},
        {"id":"30000000-0000-4000-8000-000000000012","name":"Review record","kind":"record","fields":[{"id":"value","name":"Value","type":"core/i64@1"}]}
    ]);
    document.to_string()
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (review_host, _) = Host::with_services(
        HostOptions::default(),
        vec![
            orange_audio::package(),
            orange_files::package(),
            orange_net::package(),
        ],
    )?;
    let host = &review_host;
    fs::create_dir_all(".artifacts/ui-audit/graphs")?;
    let mut categories = BTreeMap::<String, Vec<String>>::new();
    for found in host.search_nodes("", "en") {
        categories
            .entry(found.category)
            .or_default()
            .push(found.definition);
    }
    let mut count = 0;
    for (category, definitions) in categories {
        let mut draft = host.draft(&new_document())?;
        let mut y = 0.0;
        let mut row_height = 0.0f32;
        for (index, definition) in definitions.iter().enumerate() {
            if index % 3 == 0 && index != 0 {
                y += row_height + 40.0;
                row_height = 0.0;
            }
            let node = host.add_node(&mut draft, definition, [(index % 3) as f32 * 310.0, y])?;
            let graph = host.draft_view(&draft)?;
            let view = graph.nodes.iter().find(|view| view.id == node).unwrap();
            row_height = row_height.max(orange_editor_ui::canvas::node_size(view).y);
            count += 1;
        }
        // Review diagrams never open real devices when Run is pressed.
        for mut connection in host.declarations(&draft).connections {
            if connection.adapter.starts_with("org.orange.audio/") {
                connection.settings = serde_json::json!({"device": orange_audio::NULL_DEVICE});
                host.declare_connection(&mut draft, &connection)?;
            }
        }
        fs::write(
            format!(
                ".artifacts/ui-audit/graphs/{}.json",
                category.to_lowercase().replace(' ', "-")
            ),
            draft.text()?,
        )?;
    }
    let mut draft = host.draft(&orange_host::new_document()?)?;
    let add = |draft: &mut orange_host::Draft, kind: &str, at| host.add_node(draft, kind, at);
    let a = add(&mut draft, "core/whole-number", [0.0, 0.0])?;
    let absolute = add(&mut draft, "org.orange.math/absolute", [310.0, 0.0])?;
    let b = add(&mut draft, "core/whole-number", [0.0, 160.0])?;
    let sum = add(&mut draft, "org.orange.math/add", [310.0, 160.0])?;
    let greater = add(&mut draft, "org.orange.math/greater", [0.0, 350.0])?;
    let manual = add(&mut draft, "core/on-manual", [310.0, 380.0])?;
    let _ = manual;
    host.set_input(&mut draft, (a, "value"), Some("-17"))?;
    host.set_input(&mut draft, (b, "value"), Some("31"))?;
    for ((from, output), (to, input)) in [
        ((a, "result"), (absolute, "value")),
        ((absolute, "result"), (sum, "a")),
        ((b, "result"), (sum, "b")),
        ((sum, "sum"), (greater, "a")),
    ] {
        host.connect(&mut draft, (from, output), (to, input))?;
    }
    host.set_input(&mut draft, (greater, "b"), Some("50"))?;
    fs::write(".artifacts/ui-audit/graphs/propagation.json", draft.text()?)?;
    let mut draft = host.draft(&orange_host::new_document()?)?;
    let input = host.add_node(&mut draft, orange_audio::AUDIO_SOURCE, [0.0, 150.0])?;
    let output = host.add_node(&mut draft, orange_audio::AUDIO_OUT, [310.0, 150.0])?;
    host.connect(&mut draft, (input, "stream"), (output, "stream"))?;
    for mut connection in host.declarations(&draft).connections {
        connection.settings = serde_json::json!({"device": orange_audio::NULL_DEVICE});
        host.declare_connection(&mut draft, &connection)?;
    }
    let problems = host.problems(&mut draft)?;
    assert!(problems.is_empty(), "{problems:?}");
    fs::write(
        ".artifacts/ui-audit/graphs/audio-routing.json",
        draft.text()?,
    )?;
    println!(
        "Rendered metadata fixtures cover {count} node definitions; runnable propagation and silent audio routing diagrams created."
    );
    Ok(())
}
