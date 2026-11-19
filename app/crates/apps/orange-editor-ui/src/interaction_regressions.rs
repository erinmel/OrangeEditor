//! Regression checks exercise the editor through its editing and painting paths.
use super::*;

fn text_shapes(shapes: &[egui::epaint::ClippedShape]) -> Vec<&egui::epaint::TextShape> {
    fn collect<'a>(shape: &'a egui::Shape, out: &mut Vec<&'a egui::epaint::TextShape>) {
        match shape {
            egui::Shape::Text(text) => out.push(text),
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    collect(shape, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for shape in shapes {
        collect(&shape.shape, &mut out);
    }
    out
}

#[test]
fn toolbar_commands_share_one_row_and_window_controls_stay_at_the_right() {
    for width in [820.0, 1280.0, 1600.0] {
        for native in [false, true] {
            let (mut host, _) = Host::new(orange_host::HostOptions::default()).unwrap();
            let mut editor = Editor::new();
            editor.file_dialogs = true;
            editor.window_toolbar = native;
            let ctx = egui::Context::default();
            icons::install(&ctx);
            for _ in 0..3 {
                let output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 600.0),
                        )),
                        ..Default::default()
                    },
                    |ui| editor.toolbar(ui, &mut host),
                );
                let texts = text_shapes(&output.shapes);
                let at = |icon: char| {
                    texts
                        .iter()
                        .find(|text| text.galley.job.text == icon.to_string())
                        .unwrap()
                        .pos
                };
                let left = at(icons::NEW);
                for icon in [
                    icons::OPEN,
                    icons::SAVE,
                    icons::CHECK,
                    icons::RUN,
                    icons::STOP,
                    icons::SETTINGS,
                ] {
                    assert!(
                        (at(icon).y - left.y).abs() < 1.0,
                        "{width}, native={native}, {icon}"
                    );
                }
                if native {
                    assert!(at(icons::CLOSE).x > width - 50.0);
                }
                output.drop_without_applying_deltas();
            }
        }
    }
}

#[test]
fn editing_a_running_128_absolute_chain_repaints_the_new_values() {
    let (mut host, events) = Host::new(orange_host::HostOptions::default()).unwrap();
    let mut draft = host.draft(&host.bind(&tests::graph()).unwrap()).unwrap();
    let source = host
        .add_node(&mut draft, "core/whole-number", [-400.0, 0.0])
        .unwrap();
    host.set_input(&mut draft, (source, "value"), Some("-187"))
        .unwrap();
    let mut previous = source;
    for index in 0..128 {
        let at = if index == 127 {
            [0.0, 0.0]
        } else {
            [-800.0, 0.0]
        };
        let next = host
            .add_node(&mut draft, "org.orange.math/absolute", at)
            .unwrap();
        host.connect(&mut draft, (previous, "result"), (next, "value"))
            .unwrap();
        previous = next;
    }
    let mut editor = Editor::new();
    editor.open(&mut host, "chain.json", draft.text().unwrap());
    editor.run(&mut host);
    assert!(editor.is_running(), "{:?}", editor.problems());
    assert!(editor.set_input(&mut host, (source, "value"), Some("-188")));
    editor.take_events(&events);
    let graph = editor.graph().unwrap().clone();
    editor.canvas.focus(&graph, previous);
    let ctx = egui::Context::default();
    icons::install(&ctx);
    for _ in 0..3 {
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 600.0),
                )),
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    editor.canvas.show(ui, &graph, &[]);
                });
            },
        );
        let visible = text_shapes(&output.shapes)
            .iter()
            .filter(|text| text.galley.job.text == "188")
            .count();
        assert!(
            visible >= 2,
            "the last card must paint both received and produced values, got {visible}"
        );
        output.drop_without_applying_deltas();
    }
    editor.stop(&mut host);
}

#[test]
fn changing_one_audio_card_to_loopback_leaves_the_other_source_unchanged() {
    let (mut host, _) = Host::with_services(
        orange_host::HostOptions::default(),
        vec![orange_audio::package()],
    )
    .unwrap();
    let mut editor = Editor::new();
    editor.open(
        &mut host,
        "audio.json",
        orange_host::new_document().unwrap(),
    );
    let mic = editor.add_node(&mut host, orange_audio::AUDIO_IN).unwrap();
    let band = editor
        .add_node(&mut host, orange_audio::ON_BAND_LEVEL)
        .unwrap();
    let pointer = |editor: &Editor, id| {
        editor
            .graph()
            .unwrap()
            .nodes
            .iter()
            .find(|node| node.id == id)
            .unwrap()
            .settings["connection"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    assert_eq!(pointer(&editor, mic), pointer(&editor, band));
    editor.prepare_devices(&host);
    let original = pointer(&editor, mic);
    editor.choose_device(&mut host, band, Some("loopback:".into()));
    assert_eq!(pointer(&editor, mic), original);
    assert_ne!(pointer(&editor, band), original);
    let source = editor
        .declarations()
        .unwrap()
        .connections
        .iter()
        .find(|connection| connection.id.as_deref() == Some(original.as_str()))
        .unwrap();
    assert_ne!(source.settings["device"], "loopback:");
}

/// A wire let go between a whole number and a decimal input waits for its conversion;
/// picking one puts the conversion node between them with its two wires.
#[test]
fn a_wire_between_two_number_kinds_offers_conversions_and_takes_the_chosen_one() {
    let (mut host, _) = Host::new(orange_host::HostOptions::default()).unwrap();
    let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
    let whole = host
        .add_node(&mut draft, "core/whole-number", [0.0, 0.0])
        .unwrap();
    let negate = host
        .add_node(&mut draft, "org.orange.math/negate", [300.0, 0.0])
        .unwrap();
    host.choose_type(&mut draft, negate, "T", Some("core/f64@1"))
        .unwrap();
    let mut editor = Editor::new();
    editor.open_untitled(&mut host, draft.text().unwrap());
    assert!(!editor.connect(&mut host, (whole, "result"), (negate, "value")));
    let choices = editor.conversion_choice.as_ref().unwrap().choices.clone();
    assert!(!choices.is_empty());
    assert!(editor.connect_chosen(&mut host, &choices[0]));
    assert!(editor.conversion_choice.is_none());
    let graph = editor.graph().unwrap();
    assert_eq!(graph.nodes.len(), 3);
    assert_eq!(graph.wires.len(), 2);
}

#[test]
fn audio_search_finds_bilingual_intents_and_band_trigger_adds_one_card() {
    let (mut host, _) = Host::with_services(
        orange_host::HostOptions::default(),
        vec![orange_audio::package()],
    )
    .unwrap();
    for (query, locale, definition) in [
        ("volume", "en", orange_audio::LIVE_LEVEL),
        ("audio in", "en", orange_audio::AUDIO_SOURCE),
        ("entrada de audio", "es", orange_audio::AUDIO_SOURCE),
        ("bombo", "es", orange_audio::TRANSIENT),
        ("fft", "en", orange_audio::MULTIBAND_SPECTRUM),
        ("frequency splitter", "en", orange_audio::MULTIBAND_SPECTRUM),
        ("audio spectrum", "en", orange_audio::MULTIBAND_SPECTRUM),
        ("multibanda", "es", orange_audio::MULTIBAND_SPECTRUM),
        (
            "divisor de frecuencias",
            "es",
            orange_audio::MULTIBAND_SPECTRUM,
        ),
        ("audio level trigger", "en", orange_audio::NOISE_LEVEL),
        ("noise level", "en", orange_audio::NOISE_LEVEL),
        ("ruido", "es", orange_audio::NOISE_LEVEL),
    ] {
        assert!(
            host.search_nodes(query, locale)
                .iter()
                .any(|item| item.definition == definition && !item.purpose.is_empty()),
            "search for {query:?} must find {definition} with a purpose"
        );
    }
    let trigger = host.search_nodes("band trigger", "en").remove(0);
    assert_eq!(trigger.definition, orange_audio::MULTIBAND_SPECTRUM);
    let mut editor = Editor::new();
    editor.open(
        &mut host,
        "audio.json",
        orange_host::new_document().unwrap(),
    );
    let node = editor
        .add_node_at(&mut host, &trigger.definition, [0.0, 0.0])
        .unwrap();
    let graph = editor.graph().unwrap();
    assert_eq!(graph.nodes.len(), 1);
    assert_eq!(graph.nodes[0].id, node);
    assert!(graph.wires.is_empty());
}

#[test]
fn an_original_spectrum_adds_a_filtered_output_without_disconnecting_its_input() {
    let (host, _) = Host::with_services(
        orange_host::HostOptions::default(),
        vec![orange_audio::package()],
    )
    .unwrap();
    let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
    let input = host
        .add_node(&mut draft, orange_audio::AUDIO_SOURCE, [0.0, 0.0])
        .unwrap();
    let spectrum = host
        .add_node(&mut draft, orange_audio::AUDIO_SPECTRUM, [250.0, 0.0])
        .unwrap();
    host.connect(&mut draft, (input, "stream"), (spectrum, "stream"))
        .unwrap();
    host.add_filtered_audio_band(&mut draft, spectrum, [80, 120])
        .unwrap();
    let graph = host.draft_view(&draft).unwrap();
    assert_eq!(graph.nodes.len(), 2);
    let card = graph.nodes.iter().find(|node| node.id == spectrum).unwrap();
    assert_eq!(card.definition, orange_audio::MULTIBAND_SPECTRUM);
    assert!(card.ports.iter().any(
        |port| port.id == "band_stream#0" && port.label.as_deref() == Some("80\u{2013}120 Hz")
    ),);
    assert!(
        graph
            .wires
            .iter()
            .any(|wire| wire.from.0 == input && wire.to.0 == spectrum)
    );
    assert!(host.check(&draft.text().unwrap()).unwrap().is_empty());
    assert!(draft.undo());
    let original = host.draft_view(&draft).unwrap();
    assert_eq!(
        original
            .nodes
            .iter()
            .find(|node| node.id == spectrum)
            .unwrap()
            .definition,
        orange_audio::AUDIO_SPECTRUM
    );
    assert_eq!(original.wires, graph.wires);
}
