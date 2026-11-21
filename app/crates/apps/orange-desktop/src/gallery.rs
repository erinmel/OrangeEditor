//! A review gallery for card rendering (22 §3.1): every searchable node of every
//! package this application ships, one graph per package.

/// Builds and reopens the gallery graphs. Set `ORANGE_GALLERY_DIR` to keep copies.
#[test]
fn write_package_galleries() {
    let keyboard = crate::keyboard::Keyboard::default();
    let (host, _) = orange_host::Host::with_services(
        orange_host::HostOptions::default(),
        vec![
            keyboard.package().unwrap(),
            orange_net::package(),
            orange_files::package(),
            orange_audio::package(),
        ],
    )
    .unwrap();
    let mut ids: Vec<String> = host
        .nodes()
        .filter(|item| {
            !item
                .presentation
                .is_some_and(|presentation| presentation.hidden)
        })
        .map(|item| item.definition.definition.id.to_string())
        .collect();
    ids.sort();
    ids.dedup();
    let mut groups: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for id in ids {
        let package = id.split('/').next().unwrap_or_default().to_owned();
        groups.entry(package).or_default().push(id);
    }
    let folder = std::env::var_os("ORANGE_GALLERY_DIR").map(std::path::PathBuf::from);
    let check_graph = |name: &str, draft: &orange_host::Draft| {
        let text = draft.text().unwrap();
        let view = host.view(&text).unwrap();
        assert!(!view.nodes.is_empty(), "{name}");
        if let Some(folder) = &folder {
            std::fs::create_dir_all(folder).unwrap();
            std::fs::write(folder.join(name), text).unwrap();
        }
    };
    // A page of conversions, as search adds them.
    let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
    for (index, conversion) in host.conversions().iter().step_by(6).take(12).enumerate() {
        #[expect(clippy::cast_precision_loss, reason = "a small grid")]
        let at = [(index % 4) as f32 * 350.0, (index / 4) as f32 * 240.0];
        host.add_conversion(&mut draft, conversion, at).unwrap();
    }
    check_graph("conversions-0.json", &draft);
    // A frequency splitter with bands drawn out of order.
    let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
    let splitter = host
        .add_node(&mut draft, orange_audio::MULTIBAND_SPECTRUM, [0.0, 0.0])
        .unwrap();
    for range in [[2_000, 3_000], [60, 120], [400, 800], [8_000, 12_000]] {
        host.add_audio_band(&mut draft, splitter, range).unwrap();
    }
    check_graph("splitter-0.json", &draft);
    let mut fixture = host.draft(&orange_host::new_document().unwrap()).unwrap();
    for (name, shape) in [
        (
            "Sample record",
            serde_json::json!({"kind":"record","fields":[
            {"id":"level","name":"Level","type":"core/i64@1"}]}),
        ),
        (
            "Sample enum",
            serde_json::json!({"kind":"enum","variants":[
            {"id":"level","name":"Level","payload":"core/i64@1"},
            {"id":"empty","name":"Empty"}]}),
        ),
    ] {
        let mut decl = shape;
        decl["id"] = serde_json::json!(host.new_type_id(&fixture).unwrap());
        decl["name"] = serde_json::json!(name);
        host.declare_type(&mut fixture, serde_json::from_value(decl).unwrap())
            .unwrap();
    }
    let gallery_template = fixture.text().unwrap();
    for (package, ids) in groups {
        for (page, chunk) in ids.chunks(12).enumerate() {
            let mut draft = host.draft(&gallery_template).unwrap();
            for (index, id) in chunk.iter().enumerate() {
                #[expect(clippy::cast_precision_loss, reason = "a small grid")]
                let at = [(index % 4) as f32 * 385.0, (index / 4) as f32 * 445.0];
                host.add_node(&mut draft, id, at).unwrap_or_else(|error| {
                    panic!("could not place searchable node {id}: {error}")
                });
            }
            let name = format!("{}-{page}.json", package.replace('.', "-"));
            check_graph(&name, &draft);
        }
    }
}
