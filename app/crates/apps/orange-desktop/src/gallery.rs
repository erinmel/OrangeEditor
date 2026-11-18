//! A review gallery for card rendering (22 §3.1): every searchable node of every
//! package this application ships, one graph per package.

/// Writes one graph per package, every searchable node in a grid, into the folder
/// `ORANGE_GALLERY_DIR`.
#[test]
#[ignore = "writes scratch graphs"]
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
    let folder = std::env::var("ORANGE_GALLERY_DIR").unwrap();
    // A page of conversions, as search adds them.
    let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
    for (index, conversion) in host.conversions().iter().step_by(6).take(12).enumerate() {
        #[expect(clippy::cast_precision_loss, reason = "a small grid")]
        let at = [(index % 4) as f32 * 260.0, (index / 4) as f32 * 200.0];
        host.add_conversion(&mut draft, conversion, at).unwrap();
    }
    std::fs::write(
        format!("{folder}/conversions-0.json"),
        draft.text().unwrap(),
    )
    .unwrap();
    for (package, ids) in groups {
        for (page, chunk) in ids.chunks(12).enumerate() {
            let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
            for (index, id) in chunk.iter().enumerate() {
                #[expect(clippy::cast_precision_loss, reason = "a small grid")]
                let at = [(index % 4) as f32 * 260.0, (index / 4) as f32 * 330.0];
                if let Err(error) = host.add_node(&mut draft, id, at) {
                    eprintln!("{id}: {error}");
                }
            }
            let name = format!("{folder}/{}-{page}.json", package.replace('.', "-"));
            std::fs::write(name, draft.text().unwrap()).unwrap();
        }
    }
}
