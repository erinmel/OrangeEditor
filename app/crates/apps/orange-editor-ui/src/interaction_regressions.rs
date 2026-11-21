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

fn toolbar_button_centers(shapes: &[egui::epaint::ClippedShape]) -> Vec<egui::Pos2> {
    let mut centers: Vec<_> = shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Rect(rect) if rect.rect.size() == egui::vec2(32.0, 28.0) => {
                Some(rect.rect.center())
            }
            _ => None,
        })
        .collect();
    centers.sort_by(|a, b| a.x.total_cmp(&b.x));
    centers
}

#[test]
fn toolbar_commands_share_one_row_and_window_controls_stay_at_the_right() {
    for width in [640.0, 820.0, 1280.0, 1600.0] {
        for native in [false, true] {
            if native && width < 820.0 {
                continue;
            }
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
                    let text = texts
                        .iter()
                        .find(|text| text.galley.job.text == icon.to_string())
                        .unwrap();
                    text.pos + text.galley.mesh_bounds.center().to_vec2()
                };
                let left = at(icons::NEW);
                for icon in [
                    icons::OPEN,
                    icons::SAVE,
                    icons::UNDO,
                    icons::REDO,
                    icons::DISCONNECT,
                    icons::FIT,
                    icons::GRID,
                    icons::CHECK,
                    icons::RUN,
                    icons::SETTINGS,
                ] {
                    assert!(
                        (at(icon).y - left.y).abs() < 1.0,
                        "{width}, native={native}, {icon}"
                    );
                }
                let controls = toolbar_button_centers(&output.shapes);
                assert_eq!(controls.len(), if native { 19 } else { 16 });
                assert!(controls.iter().all(|at| (at.y - left.y).abs() < 1.0));
                assert!(at(icons::REDO).x < controls[6].x);
                assert!(at(icons::GRID).x < at(icons::CHECK).x);
                assert!(
                    texts
                        .iter()
                        .all(|text| text.galley.job.text != icons::STOP.to_string())
                );
                if native {
                    assert!(at(icons::CLOSE).x > width - 50.0);
                }
                output.drop_without_applying_deltas();
            }
        }
    }
}

#[test]
fn settings_stays_visible_when_edit_commands_overflow_at_high_interface_scale() {
    let (mut host, _) = Host::new(orange_host::HostOptions::default()).unwrap();
    let mut editor = Editor::new();
    editor.file_dialogs = true;
    let ctx = egui::Context::default();
    let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(410.0, 410.0));
    let mut frame = |events| {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(viewport),
                events,
                ..Default::default()
            },
            |ui| editor.toolbar(ui, &mut host),
        );
        let shapes = std::mem::take(&mut output.shapes);
        output.drop_without_applying_deltas();
        shapes
    };
    for _ in 0..3 {
        frame(vec![]);
    }
    let shapes = frame(vec![]);
    let icon = text_shapes(&shapes)
        .into_iter()
        .find(|line| line.galley.text() == icons::SETTINGS.to_string())
        .unwrap();
    let at = icon.pos + icon.galley.mesh_bounds.center().to_vec2();
    assert!(at.x > viewport.right() - 50.0);
    assert!(viewport.contains(at));
    frame(vec![egui::Event::PointerMoved(at)]);
    for pressed in [true, false] {
        frame(vec![egui::Event::PointerButton {
            button: egui::PointerButton::Primary,
            pos: at,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }]);
    }
    assert!(editor.settings_open);
}

#[test]
fn the_top_toolbar_exposes_grid_and_group_controls_at_narrow_width() {
    let (mut host, _) = Host::new(orange_host::HostOptions::default()).unwrap();
    let mut editor = Editor::new();
    editor.open_untitled(&mut host, orange_host::new_document().unwrap());
    let first = editor.add_node(&mut host, "core/whole-number").unwrap();
    let second = editor.add_node(&mut host, "core/whole-number").unwrap();
    editor.canvas.select_many(&[first, second]);
    let ctx = egui::Context::default();
    theme::apply(&ctx);
    let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(490.0, 600.0));
    let frame = |editor: &mut Editor, host: &mut Host, events| {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(viewport),
                events,
                ..Default::default()
            },
            |ui| editor.toolbar(ui, host),
        );
        let shapes = std::mem::take(&mut output.shapes);
        output.drop_without_applying_deltas();
        shapes
    };
    let at = |shapes: &[egui::epaint::ClippedShape], label: &str| {
        let text = text_shapes(shapes)
            .into_iter()
            .find(|shape| shape.galley.text() == label)
            .unwrap_or_else(|| panic!("missing toolbar control {label}"));
        let bounds = text.galley.mesh_bounds.translate(text.pos.to_vec2());
        assert!(viewport.contains_rect(bounds), "{label}: {bounds:?}");
        bounds.center()
    };
    for _ in 0..3 {
        frame(&mut editor, &mut host, vec![]);
    }
    let shapes = frame(&mut editor, &mut host, vec![]);
    for icon in [icons::DISCONNECT, icons::FIT, icons::GRID, icons::SETTINGS] {
        at(&shapes, &icon.to_string());
    }
    assert!(
        text_shapes(&shapes)
            .iter()
            .all(|text| text.galley.text() != t("grid-spacing"))
    );
    let click_at = |editor: &mut Editor, host: &mut Host, pos| {
        orange_testkit::ui::click(pos, egui::PointerButton::Primary, |events| {
            frame(editor, host, events);
        });
    };
    let click = |editor: &mut Editor, host: &mut Host, label: &str| {
        let position = at(&frame(editor, host, vec![]), label);
        click_at(editor, host, position);
    };
    let controls = toolbar_button_centers(&shapes);
    assert_eq!(controls.len(), 13);
    click_at(&mut editor, &mut host, controls[8]);
    assert!(!editor.canvas.snap);
    click(&mut editor, &mut host, &icons::GRID.to_string());
    click(&mut editor, &mut host, t("grid-lines"));
    assert_eq!(editor.canvas.grid_style, canvas::GridStyle::Lines);
    click(&mut editor, &mut host, t("grid-hidden"));
    assert_eq!(editor.canvas.grid_style, canvas::GridStyle::Hidden);
    click(&mut editor, &mut host, t("grid-dots"));
    at(&frame(&mut editor, &mut host, vec![]), "24");
    click(&mut editor, &mut host, &icons::GRID.to_string());
    click_at(&mut editor, &mut host, controls[3]);
    assert_eq!(editor.groups().len(), 1);
    click_at(&mut editor, &mut host, controls[4]);
    assert!(editor.groups().is_empty());
}

#[test]
fn toolbar_hover_fills_the_button_around_its_centered_icon() {
    let ctx = egui::Context::default();
    icons::install(&ctx);
    theme::apply(&ctx);
    ctx.style_mut_of(ctx.theme(), |style| style.animation_time = 0.0);
    let mut rects = [egui::Rect::NOTHING; 4];
    for hovered in [None, Some(0), Some(1), Some(2), Some(3)] {
        let events = hovered
            .map(|index: usize| egui::Event::PointerMoved(rects[index].center()))
            .into_iter()
            .collect();
        let output = ctx.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| {
                ui.horizontal(|ui| {
                    rects = [
                        icons::button(ui, true, icons::SETTINGS, "Settings", "").rect,
                        icons::run_button(ui, true, false).rect,
                        icons::run_button(ui, true, true).rect,
                        icons::button(ui, false, icons::SAVE, "Save", "").rect,
                    ];
                });
            },
        );
        for (index, icon) in [icons::SETTINGS, icons::RUN, icons::STOP, icons::SAVE]
            .into_iter()
            .enumerate()
        {
            let rect = rects[index];
            let text = text_shapes(&output.shapes)
                .into_iter()
                .find(|text| text.galley.job.text == icon.to_string())
                .unwrap();
            let center = text.pos + text.galley.mesh_bounds.center().to_vec2();
            assert!(
                center.distance(rect.center()) < 0.01,
                "{icon} {center:?} vs {rect:?}"
            );
            let fill = orange_testkit::ui::rect_fill(&output.shapes, rect);
            let stop_red = egui::Color32::from_rgb(180, 40, 57);
            let expected = match (index, hovered == Some(index)) {
                (2, true) => stop_red.gamma_multiply(1.18),
                (2, false) => stop_red,
                (0 | 1, true) => ctx.style_of(ctx.theme()).visuals.widgets.hovered.bg_fill,
                _ => egui::Color32::TRANSPARENT,
            };
            assert_eq!(
                fill.unwrap_or(egui::Color32::TRANSPARENT),
                expected,
                "{icon}"
            );
        }
        output.drop_without_applying_deltas();
    }
}

#[test]
fn a_standalone_audio_trigger_can_be_finished_while_the_graph_runs() {
    let (mut host, _events) = Host::with_services(
        orange_host::HostOptions::default(),
        vec![orange_audio::package()],
    )
    .unwrap();
    let mut editor = Editor::new();
    editor.open(
        &mut host,
        "repair-audio.json",
        orange_host::new_document().unwrap(),
    );
    let meter = editor
        .add_node(&mut host, orange_audio::NOISE_LEVEL)
        .unwrap();
    let document = editor.document.as_ref().unwrap();
    assert!(!document.run_blocked);
    assert_eq!(document.run_omitted, [meter]);
    assert!(
        document
            .problem_locations
            .iter()
            .any(|problem| problem.level == ProblemLevel::Draft)
    );
    editor.run(&mut host);
    assert!(editor.is_running(), "{:?}", editor.problems());
    let flow = editor.document.as_ref().unwrap().flow.as_ref().unwrap().0;
    let source = editor
        .add_node(&mut host, orange_audio::AUDIO_SOURCE)
        .unwrap();
    let mut input = editor
        .document
        .as_ref()
        .unwrap()
        .declared
        .connections
        .iter()
        .find(|connection| connection.adapter == orange_audio::INPUT_ADAPTER)
        .unwrap()
        .clone();
    input.settings = serde_json::json!({"device": orange_audio::NULL_DEVICE});
    editor.declare_connection(&mut host, &input).unwrap();
    assert!(!editor.problems().is_empty());
    assert!(
        editor
            .graph()
            .unwrap()
            .nodes
            .iter()
            .any(|node| node.id == meter)
    );

    assert!(editor.connect(&mut host, (source, "stream"), (meter, "stream")));
    assert!(editor.problems().is_empty(), "{:?}", editor.problems());
    assert_eq!(
        editor.document.as_ref().unwrap().flow.as_ref().unwrap().0,
        flow
    );
    let start = std::time::Instant::now();
    loop {
        let reading = host
            .inspect(flow, move |run| {
                run.live_port_value(meter, &"level".parse().unwrap())
                    .cloned()
            })
            .unwrap();
        if reading == Some(orange_host::Value::F64(orange_audio::SILENCE_DB)) {
            break;
        }
        assert!(
            start.elapsed().as_secs_f32() < 3.0,
            "the repaired meter never received audio"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    editor.stop(&mut host);
}

#[test]
fn a_wired_audio_trigger_blocks_run_until_its_input_is_supplied() {
    let (mut host, _) = Host::with_services(
        orange_host::HostOptions::default(),
        vec![orange_audio::package()],
    )
    .unwrap();
    let mut editor = Editor::new();
    editor.open_untitled(&mut host, orange_host::new_document().unwrap());
    let meter = editor
        .add_node(&mut host, orange_audio::NOISE_LEVEL)
        .unwrap();
    let sequence = editor.add_node(&mut host, "core/sequence").unwrap();
    assert!(editor.connect(&mut host, (meter, "then"), (sequence, "in")));
    let document = editor.document.as_ref().unwrap();
    assert!(document.run_blocked);
    assert!(!document.run_omitted.contains(&meter));
    assert!(
        editor.canvas.marks().ports[&(
            meter,
            "stream".to_owned(),
            Some(orange_host::PortDirection::Input),
        )]
            .error
    );
    editor.run(&mut host);
    assert!(!editor.is_running());
}

#[test]
fn an_unwired_timer_with_incomplete_settings_is_also_a_draft() {
    let (mut host, _) = Host::new(orange_host::HostOptions::default()).unwrap();
    let mut editor = Editor::new();
    editor.open_untitled(&mut host, orange_host::new_document().unwrap());
    let timer = editor.add_node(&mut host, "core/on-timer").unwrap();
    assert!(editor.set_setting(&mut host, timer, "/interval", Some(serde_json::json!("0"))));
    let document = editor.document.as_ref().unwrap();
    assert_eq!(document.run_omitted, [timer]);
    assert!(!document.run_blocked);

    let sequence = editor.add_node(&mut host, "core/sequence").unwrap();
    assert!(editor.connect(&mut host, (timer, "then"), (sequence, "in")));
    assert!(editor.document.as_ref().unwrap().run_blocked);
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
fn a_request_card_switches_saved_profiles_from_its_dropdown() {
    let (mut host, _) = Host::with_services(
        orange_host::HostOptions::default(),
        vec![orange_net::package()],
    )
    .unwrap();
    let mut editor = Editor::new();
    editor.open_untitled(&mut host, orange_host::new_document().unwrap());
    let mut connection = |name: &str, adapter: &str, settings: serde_json::Value| {
        editor
            .declare_connection(
                &mut host,
                &orange_host::Connection {
                    id: None,
                    name: name.into(),
                    adapter: adapter.into(),
                    scope: orange_host::ConnectionScope::Graph,
                    settings,
                },
            )
            .unwrap()
    };
    let first = connection(
        "Primary API",
        orange_net::HTTP_ADAPTER,
        serde_json::json!({"base_url": "http://127.0.0.1:12345"}),
    );
    let second = connection(
        "Backup API",
        orange_net::HTTP_ADAPTER,
        serde_json::json!({"base_url": "http://127.0.0.1:12346"}),
    );
    connection(
        "WebSocket profile",
        orange_net::WS_ADAPTER,
        serde_json::json!({"url": "ws://127.0.0.1:12347"}),
    );
    let request = editor
        .add_node(&mut host, orange_net::HTTP_REQUEST)
        .unwrap();
    let _ = editor.set_setting(&mut host, request, "/connection", Some(first.into()));
    let graph = editor.graph().unwrap().clone();
    editor.canvas.focus(&graph, request);
    let ctx = egui::Context::default();
    icons::install(&ctx);
    let mut frame = |events| {
        let mut output = ctx.run_ui(
            egui::RawInput {
                events,
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 700.0),
                )),
                ..Default::default()
            },
            |ui| editor.canvas_panel(ui, &mut host),
        );
        output.textures_delta.clear();
        output
    };
    let position = |output: &egui::FullOutput, label| {
        text_shapes(&output.shapes)
            .into_iter()
            .find(|text| text.galley.job.text == label)
            .map(|text| text.pos + text.galley.rect.center().to_vec2())
    };
    let click = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    let control = position(&frame(vec![]), "Primary API").unwrap();
    frame(vec![
        egui::Event::PointerMoved(control),
        click(control, true),
    ]);
    frame(vec![click(control, false)]);
    let list = frame(vec![]);
    assert!(position(&list, "WebSocket profile").is_none());
    let option = position(&list, "Backup API").unwrap();
    frame(vec![egui::Event::PointerMoved(option), click(option, true)]);
    frame(vec![click(option, false)]);
    let result = frame(vec![]);
    assert!(position(&result, "Backup API").is_some());
    assert!(position(&result, "Primary API").is_none());
    assert_eq!(
        editor.graph().unwrap().nodes[0].settings["connection"],
        second
    );
    assert!(editor.node_config_menu.is_none());
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
    let mic = editor
        .add_node(&mut host, orange_audio::AUDIO_SOURCE)
        .unwrap();
    let band = editor
        .add_node(&mut host, orange_audio::AUDIO_SOURCE)
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
        .add_node(&mut draft, orange_audio::MULTIBAND_SPECTRUM, [250.0, 0.0])
        .unwrap();
    host.connect(&mut draft, (input, "stream"), (spectrum, "stream"))
        .unwrap();
    host.add_audio_band(&mut draft, spectrum, [80, 120])
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
    let card = original
        .nodes
        .iter()
        .find(|node| node.id == spectrum)
        .unwrap();
    assert!(!card.ports.iter().any(|port| port.id == "band_stream#0"));
    assert_eq!(original.wires, graph.wires);
}

#[test]
fn a_disconnected_get_allows_run_and_an_exported_get_blocks_it() {
    let (mut host, _) = Host::new(orange_host::HostOptions::default()).unwrap();
    let mut editor = Editor::new();
    editor.open_untitled(&mut host, orange_host::new_document().unwrap());
    let get = editor.add_node(&mut host, "core/get-variable").unwrap();
    assert!(!editor.document.as_ref().unwrap().run_blocked);
    assert!(
        editor
            .document
            .as_ref()
            .unwrap()
            .problem_locations
            .iter()
            .all(|problem| problem.level == ProblemLevel::Draft)
    );
    assert!(!editor.canvas.marks().nodes[&get].error);
    editor.run(&mut host);
    assert!(editor.is_running(), "{:?}", editor.problems());
    editor.stop(&mut host);

    let saved = editor.document.as_ref().unwrap().draft.text().unwrap();
    let mut graph: serde_json::Value = serde_json::from_str(&saved).unwrap();
    graph["exports"] = serde_json::json!([{"id": "value", "node": get, "port": "value"}]);
    editor.open_untitled(&mut host, graph.to_string());
    assert!(editor.document.as_ref().unwrap().run_blocked);
    assert!(
        editor
            .document
            .as_ref()
            .unwrap()
            .problem_locations
            .iter()
            .any(|problem| problem.level == ProblemLevel::Blocker)
    );
    editor.run(&mut host);
    assert!(!editor.is_running());
}

#[test]
fn a_splitter_without_audio_warns_without_blocking_a_manual_flow() {
    let (mut host, _) = Host::with_services(
        orange_host::HostOptions::default(),
        vec![orange_audio::package(), orange_net::package()],
    )
    .unwrap();
    let mut editor = Editor::new();
    editor.open_untitled(&mut host, orange_host::new_document().unwrap());
    let request = editor
        .add_node(&mut host, "org.orange.net/http-request")
        .unwrap();
    assert!(!editor.canvas.marks().nodes[&request].error);
    let splitter = editor
        .add_node(&mut host, orange_audio::MULTIBAND_SPECTRUM)
        .unwrap();
    assert!(!editor.document.as_ref().unwrap().run_blocked);
    assert!(!editor.canvas.marks().nodes[&splitter].error);
    let manual = editor.add_node(&mut host, "core/on-manual").unwrap();
    editor.run(&mut host);
    assert!(editor.is_running(), "{:?}", editor.problems());
    editor.stop(&mut host);
    let invalid = editor.add_node(&mut host, "core/set-variable").unwrap();
    assert!(editor.connect(&mut host, (manual, "then"), (invalid, "in")));
    // An unresolved variable on the execution path blocks Run.
    assert!(editor.document.as_ref().unwrap().run_blocked);
    assert!(!editor.canvas.marks().nodes[&splitter].error);
    assert!(editor.canvas.marks().nodes[&invalid].error);
    editor.run(&mut host);
    assert!(!editor.is_running());
}

#[test]
fn the_log_filters_messages_and_exposes_trace_and_clear_actions() {
    let mut editor = Editor::new();
    editor.log_events = true;
    editor.note(NoteLevel::Info, "request sent");
    editor.note(NoteLevel::Warning, "request will retry");
    editor.log_search = "warning retry".to_owned();
    let ctx = egui::Context::default();
    icons::install(&ctx);
    apply_theme(&ctx);
    let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(370.0, 340.0));
    let frame = |editor: &mut Editor, events| {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(viewport),
                events,
                ..Default::default()
            },
            |ui| editor.log_panel(ui),
        );
        let shapes = std::mem::take(&mut output.shapes);
        output.drop_without_applying_deltas();
        shapes
    };
    let click = |editor: &mut Editor, at| {
        frame(editor, vec![egui::Event::PointerMoved(at)]);
        for pressed in [true, false] {
            frame(
                editor,
                vec![egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
    };
    frame(&mut editor, vec![]);
    let shapes = frame(&mut editor, vec![]);
    let texts = text_shapes(&shapes);
    assert!(
        texts
            .iter()
            .any(|text| text.galley.text().contains("request will retry"))
    );
    assert!(
        !texts
            .iter()
            .any(|text| text.galley.text() == "request sent")
    );
    let at = |label: &str| {
        let text = texts
            .iter()
            .find(|text| text.galley.text() == label)
            .unwrap();
        let center = text.pos + text.galley.mesh_bounds.center().to_vec2();
        assert!(viewport.contains(center), "{label}");
        center
    };
    let search_clear = at(&icons::CLOSE.to_string());
    let trace = at(t("live-trace-off"));
    let level_filter = at(t("log-all-levels"));
    click(&mut editor, search_clear);
    assert!(editor.log_search.is_empty());
    click(&mut editor, trace);
    assert!(editor.live_trace);

    click(&mut editor, level_filter);
    let shapes = frame(&mut editor, vec![]);
    let warning = text_shapes(&shapes)
        .into_iter()
        .find(|text| text.galley.text() == t("log-level-warning"))
        .unwrap();
    click(
        &mut editor,
        warning.pos + warning.galley.mesh_bounds.center().to_vec2(),
    );
    assert_eq!(editor.log_filter, Some(NoteLevel::Warning));

    let shapes = frame(&mut editor, vec![]);
    let texts = text_shapes(&shapes);
    assert!(
        !texts
            .iter()
            .any(|text| text.galley.text() == "request sent")
    );
    assert!(
        texts
            .iter()
            .any(|text| text.galley.text() == t("live-trace-on"))
    );
    let clear = texts
        .iter()
        .find(|text| text.galley.text() == t("clear-log"))
        .unwrap();
    click(
        &mut editor,
        clear.pos + clear.galley.mesh_bounds.center().to_vec2(),
    );
    assert!(editor.log().next().is_none());
}
