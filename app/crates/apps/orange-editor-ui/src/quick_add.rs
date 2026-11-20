//! Adding a node where the pointer is, from a search: opened by letting a wire go on
//! empty canvas, which lists only the kinds that can take it and wires the new node in,
//! by Insert node… on a wire, which also wires the new node on to where the wire went,
//! or by Tab, Space or Add node… over the canvas, which lists every kind. The search is
//! the host's: names, aliases, categories, concepts and descriptions, in the editor's
//! language and English.
use super::*;

/// The search being shown, and where the node it adds goes.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct QuickAdd {
    /// The output the new node takes a wire from, when a wire was let go.
    from: Option<(NodeId, String)>,
    /// The input the new node's output is wired to, when it goes into a wire.
    into: Option<(NodeId, String)>,
    /// Where the node goes, in graph units.
    at: [f32; 2],
    /// Where the search is drawn, on the screen.
    screen: egui::Pos2,
    search: String,
    focused: bool,
    /// The choice the arrow keys are on.
    highlighted: usize,
    choices_key: Option<(String, String, u64)>,
    choices: Vec<QuickChoice>,
}

impl QuickAdd {
    pub(crate) fn invalidate(&mut self) {
        self.choices_key = None;
    }
}

/// One kind the search offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickChoice {
    pub definition: String,
    pub label: String,
    /// The input of the new node the wire goes to, when a wire was let go.
    pub input: Option<String>,
    pub category: String,
    /// One line on what it does, or empty.
    pub purpose: String,
}

/// Most kinds the search lists at once; typing narrows it.
const SHOWN: usize = 40;

impl Editor {
    /// Opens the search to add a node at `at` (graph units), drawn at `screen`. With
    /// `from`, it lists only the kinds that can take a wire from that output.
    pub fn open_quick_add(
        &mut self,
        from: Option<(NodeId, String)>,
        at: [f32; 2],
        screen: egui::Pos2,
    ) {
        self.quick = Some(QuickAdd {
            from,
            into: None,
            at,
            screen,
            search: String::new(),
            focused: false,
            highlighted: 0,
            choices_key: None,
            choices: Vec::new(),
        });
    }

    /// Opens the search to put a node into the wire `wire` at `at`: the chosen node
    /// takes the wire's output and its own output goes on to the wire's input.
    pub fn open_insert_node(&mut self, wire: canvas::WireEnds, at: [f32; 2], screen: egui::Pos2) {
        self.open_quick_add(Some(wire.0), at, screen);
        if let Some(quick) = &mut self.quick {
            quick.into = Some(wire.1);
        }
    }

    /// Whether the search is open.
    pub fn is_quick_add_open(&self) -> bool {
        self.quick.is_some()
    }

    /// What the open search offers for `search`, best first.
    pub fn quick_choices(&self, host: &Host, search: &str) -> Vec<QuickChoice> {
        let Some(quick) = &self.quick else {
            return Vec::new();
        };
        let found = host.search_nodes(search, self.language());
        match (&quick.from, &self.document) {
            (Some(from), Some(document)) => {
                let takers = host.takers(&document.draft, (from.0, &from.1));
                // The search's order, keeping the kinds that take the wire.
                found
                    .into_iter()
                    .filter_map(|found| {
                        let taker = takers
                            .iter()
                            .find(|taker| taker.definition == found.definition)?;
                        Some(QuickChoice {
                            definition: found.definition,
                            label: found.label,
                            input: Some(taker.input.clone()),
                            category: found.category,
                            purpose: found.purpose,
                        })
                    })
                    .collect()
            }
            _ => palette::search_with_presets(found, search, self.language())
                .into_iter()
                .map(|found| QuickChoice {
                    definition: found.definition,
                    label: found.label,
                    input: None,
                    category: found.category,
                    purpose: found.purpose,
                })
                .collect(),
        }
    }

    /// Adds the chosen kind where the search was opened and, when a wire was let go,
    /// wires it in; put into a wire, its first fitting output goes on to the wire's
    /// input. Closes the search. Returns the new node.
    pub fn pick_quick(&mut self, host: &mut Host, choice: &QuickChoice) -> Option<NodeId> {
        let quick = self.quick.take()?;
        if let (Some(from), Some(to)) = (quick.from.as_ref(), quick.into.as_ref()) {
            return self.add_node_into_wire(
                host,
                &choice.definition,
                quick.at,
                &(from.clone(), to.clone()),
            );
        }
        let node = self.add_node_at(host, &choice.definition, quick.at)?;
        if let (Some(from), Some(input)) = (&quick.from, &choice.input) {
            let _ = self.connect(host, (from.0, &from.1), (node, input));
        }
        self.canvas.select(Some(node));
        Some(node)
    }

    /// Draws the open search over the canvas: a field to type in and the matching
    /// kinds, each with its category and what it is for. The arrow keys move, Enter
    /// takes the one they are on, Escape or a click elsewhere closes it.
    pub(crate) fn quick_add_popup(&mut self, ctx: &egui::Context, host: &mut Host) {
        let Some(quick) = &self.quick else {
            return;
        };
        let wired = quick.from.is_some();
        let mut search = quick.search.clone();
        let mut focused = quick.focused;
        let mut highlighted = quick.highlighted;
        let screen = quick.screen;
        let was_focused = quick.focused;
        let mut picked = None;
        let mut close = false;
        let mut dragged = false;
        let bounds = ctx.content_rect().shrink(6.0);
        let shown = egui::Area::new(egui::Id::unique("orange-quick-add"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen)
            .constrain_to(bounds)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(304.0_f32.min((bounds.width() - 20.0).max(48.0)));
                    ui.strong(if wired {
                        t("connect-a-node")
                    } else {
                        t("add-a-node")
                    });
                    // The arrows move the choice, not the caret.
                    let (up, down) = ui.input_mut(|input| {
                        (
                            input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                            input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                        )
                    });
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut search)
                            .hint_text(if wired {
                                t("quick-add-wire-hint")
                            } else {
                                t("quick-add-hint")
                            })
                            .desired_width(f32::INFINITY),
                    );
                    if field.changed() {
                        highlighted = 0;
                    }
                    let key = (
                        search.clone(),
                        self.language().to_owned(),
                        host.registry().generation(),
                    );
                    if self
                        .quick
                        .as_ref()
                        .is_some_and(|quick| quick.choices_key.as_ref() != Some(&key))
                    {
                        let choices = self.quick_choices(host, &search);
                        if let Some(quick) = &mut self.quick {
                            quick.choices_key = Some(key);
                            quick.choices = choices;
                        }
                    }
                    let choices = self
                        .quick
                        .as_ref()
                        .map_or(&[][..], |quick| quick.choices.as_slice());
                    let last = choices.len().min(SHOWN).saturating_sub(1);
                    highlighted = highlighted.min(last);
                    if down {
                        highlighted = (highlighted + 1).min(last);
                    }
                    if up {
                        highlighted = highlighted.saturating_sub(1);
                    }
                    if !focused {
                        if ui.input(|input| input.focused) {
                            field.request_focus();
                        }
                        focused = true;
                    }
                    let (enter, escape) = ui.input(|input| {
                        (
                            input.key_pressed(egui::Key::Enter),
                            input.key_pressed(egui::Key::Escape),
                        )
                    });
                    if escape {
                        close = true;
                    }
                    if enter {
                        picked = choices.get(highlighted).cloned();
                    }
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .max_height(320.0_f32.min((bounds.height() - 110.0).max(48.0)))
                        .show(ui, |ui| {
                            if choices.is_empty() {
                                ui.weak(t("quick-add-no-match"));
                            }
                            for (index, choice) in choices.iter().take(SHOWN).enumerate() {
                                let on = index == highlighted;
                                let target = choice.input.as_deref().map(port_label);
                                let detail = if choice.purpose.is_empty() {
                                    choice.category.clone()
                                } else {
                                    format!("{} · {}", choice.category, choice.purpose)
                                };
                                let draft = self.document.as_ref().map(|document| &document.draft);
                                let card = self
                                    .node_search_previews
                                    .then(|| self.previews.card(host, None, &choice.definition))
                                    .flatten();
                                let row = palette::result_row(
                                    ui,
                                    &choice.label,
                                    &detail,
                                    target.as_deref(),
                                    on,
                                    card,
                                );
                                if row.drag_started() {
                                    let card = self
                                        .previews
                                        .card(host, draft, &choice.definition)
                                        .cloned();
                                    row.dnd_set_drag_payload(palette::PaletteDrag {
                                        item: PaletteItem {
                                            id: choice.definition.clone(),
                                            label: choice.label.clone(),
                                            package: String::new(),
                                            purpose: choice.purpose.clone(),
                                        },
                                        card,
                                    });
                                    dragged = true;
                                }
                                if on && (up || down) {
                                    row.scroll_to_me(None);
                                }
                                if row.clicked() {
                                    picked = Some(choice.clone());
                                }
                            }
                        });
                });
            });
        if shown.response.clicked_elsewhere() && focused && was_focused {
            close = true;
        }
        if let Some(quick) = &mut self.quick {
            quick.search = search;
            quick.focused = focused;
            quick.highlighted = highlighted;
        }
        if dragged {
            self.quick = None;
            ctx.request_repaint();
        } else if let Some(choice) = picked {
            let _ = self.pick_quick(host, &choice);
        } else if close {
            self.quick = None;
        }
    }
}

#[cfg(test)]
mod presentation_tests {
    use super::*;
    #[test]
    fn sidebar_and_picker_share_preview_mode_without_editing_the_document() {
        let (mut host, _events) = Host::new(orange_host::HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        editor.open(
            &mut host,
            "graph.json",
            orange_host::new_document().unwrap(),
        );
        editor.search = "and".into();
        let before = editor.document.as_ref().unwrap().draft.text().unwrap();
        {
            for (picker, previews) in [(false, false), (false, true), (true, false), (true, true)] {
                editor.node_search_previews = previews;
                let context = egui::Context::default();
                if picker {
                    editor.open_quick_add(None, [0.0, 0.0], egui::pos2(10.0, 10.0));
                    editor.quick.as_mut().unwrap().search = "and".into();
                }
                // Areas settle their initial bounds on the next frame.
                for frame in 0..3 {
                    let mut output = context.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(330.0, 600.0),
                            )),
                            ..Default::default()
                        },
                        |ui| {
                            if picker {
                                editor.quick_add_popup(ui.ctx(), &mut host);
                            } else {
                                editor.nodes_panel(ui, &mut host);
                            }
                        },
                    );
                    output.textures_delta.clear();
                    if frame == 2 {
                        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "And")), "search result is visible, picker={picker}");
                        for label in ["a", "b", "result"] {
                            let drawn = output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == label));
                            assert_eq!(drawn, previews, "{label}, picker={picker}");
                        }
                    }
                    output.drop_without_applying_deltas();
                }
            }
        }
        assert_eq!(
            editor.document.as_ref().unwrap().draft.text().unwrap(),
            before
        );
        assert_eq!(editor.can_undo(), (false, false));
    }
}
