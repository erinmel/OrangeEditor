//! Adding a node where the pointer is, from a search: opened by letting a wire go on
//! empty canvas, which lists only the kinds that can take it and wires the new node in,
//! or by Tab or Space over the canvas, which lists every kind.
use super::*;

/// The search being shown, and where the node it adds goes.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct QuickAdd {
    /// The output the new node takes a wire from, when a wire was let go.
    from: Option<(NodeId, String)>,
    /// Where the node goes, in graph units.
    at: [f32; 2],
    /// Where the search is drawn, on the screen.
    screen: egui::Pos2,
    search: String,
    focused: bool,
}

/// One kind the search offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickChoice {
    pub definition: String,
    pub label: String,
    /// The input of the new node the wire goes to, when a wire was let go.
    pub input: Option<String>,
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
            at,
            screen,
            search: String::new(),
            focused: false,
        });
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
        let words: Vec<String> = search
            .to_lowercase()
            .split_whitespace()
            .map(str::to_owned)
            .collect();
        let matches = |label: &str, definition: &str| {
            let text = format!("{} {}", label.to_lowercase(), definition.to_lowercase());
            words.iter().all(|word| text.contains(word.as_str()))
        };
        match (&quick.from, &self.document) {
            (Some(from), Some(document)) => {
                let mut found: Vec<QuickChoice> = host
                    .takers(&document.draft, (from.0, &from.1))
                    .into_iter()
                    .filter(|taker| matches(&taker.label, &taker.definition))
                    .map(|taker| QuickChoice {
                        definition: taker.definition,
                        label: taker.label,
                        input: Some(taker.input),
                    })
                    .collect();
                // The best match first: "wait" puts Wait before Await event.
                if !search.trim().is_empty() {
                    found.sort_by_key(|choice| crate::palette::rank(&choice.label, &[], search));
                }
                found
            }
            _ => palette(host, search)
                .into_iter()
                .flat_map(|(_, items)| items)
                .map(|item| QuickChoice {
                    definition: item.id,
                    label: item.label,
                    input: None,
                })
                .collect(),
        }
    }

    /// Adds the chosen kind where the search was opened and, when a wire was let go,
    /// wires it in. Closes the search. Returns the new node.
    pub fn pick_quick(&mut self, host: &mut Host, choice: &QuickChoice) -> Option<NodeId> {
        let quick = self.quick.take()?;
        let node = self.add_node_at(host, &choice.definition, quick.at)?;
        if let (Some(from), Some(input)) = (&quick.from, &choice.input) {
            let _ = self.connect(host, (from.0, &from.1), (node, input));
        }
        self.canvas.select(Some(node));
        Some(node)
    }

    /// Draws the open search over the canvas: a field to type in and the matching
    /// kinds. Enter takes the first, Escape or a click elsewhere closes it.
    pub(crate) fn quick_add_popup(&mut self, ctx: &egui::Context, host: &mut Host) {
        let Some(quick) = &self.quick else {
            return;
        };
        let choices = self.quick_choices(host, &quick.search);
        let wired = quick.from.is_some();
        let mut search = quick.search.clone();
        let mut focused = quick.focused;
        let mut picked = None;
        let mut close = false;
        let shown = egui::Area::new(egui::Id::unique("orange-quick-add"))
            .order(egui::Order::Foreground)
            .fixed_pos(quick.screen)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(240.0);
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut search)
                            .hint_text(if wired {
                                "Node to wire in…"
                            } else {
                                "Add a node…"
                            })
                            .desired_width(f32::INFINITY),
                    );
                    if !focused {
                        field.request_focus();
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
                        picked = choices.first().cloned();
                    }
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .max_height(280.0)
                        .show(ui, |ui| {
                            if choices.is_empty() {
                                ui.weak("Nothing matches.");
                            }
                            for choice in choices.iter().take(SHOWN) {
                                let text = match &choice.input {
                                    Some(input) => format!("{}  → {input}", choice.label),
                                    None => choice.label.clone(),
                                };
                                if ui
                                    .selectable_label(false, text)
                                    .on_hover_text(choice.definition.as_str())
                                    .clicked()
                                {
                                    picked = Some(choice.clone());
                                }
                            }
                        });
                });
            });
        if shown.response.clicked_elsewhere() && focused && quick.focused {
            close = true;
        }
        if let Some(quick) = &mut self.quick {
            quick.search = search;
            quick.focused = focused;
        }
        if let Some(choice) = picked {
            let _ = self.pick_quick(host, &choice);
        } else if close {
            self.quick = None;
        }
    }
}
