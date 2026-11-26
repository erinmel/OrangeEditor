//! Adding a node where the pointer is, from a search: opened by letting a wire go on
//! empty canvas, which lists only the kinds that can take it and wires the new node in,
//! by Insert node… on a wire, which also wires the new node on to where the wire went,
//! or by Tab, Space or Add node… over the canvas, which lists every kind. The search is
//! the host's: names, aliases, categories, concepts and descriptions, in the editor's
//! language and the bundled search languages.
use super::*;
use orange_host::intrinsic;

fn wire_member(definition: &str) -> Option<(&'static str, &'static str, &str, &str)> {
    for (prefix, node, key) in [
        (FIELD_GET, GET_FIELD, "field"),
        (FIELD_SET, SET_FIELD, "field"),
        (VARIANT_PAYLOAD, ENUM_PAYLOAD, "variant"),
    ] {
        if let Some((ty, member)) = definition
            .strip_prefix(prefix)
            .and_then(|rest| rest.split_once('|'))
        {
            return Some((node, key, ty, member));
        }
    }
    None
}

fn typed_wire_choices(host: &Host, draft: &Draft, ty: &str, search: &str) -> Vec<QuickChoice> {
    if ty.starts_with("core/") || ty.contains(['$', '<']) {
        return Vec::new();
    }
    let named_type = type_name(ty);
    let query = orange_host::SearchQuery::new(search);
    let mut choices = Vec::new();
    let mut offer = |definition, label: String, input, purpose: &str, aliases: &str| {
        if query.matches([label.as_str(), named_type.as_str(), aliases]) {
            choices.push(QuickChoice {
                definition,
                label,
                input: Some(input),
                output: None,
                category: t("wire-type-actions").to_owned(),
                purpose: purpose.to_owned(),
            });
        }
    };
    if let Some(variants) = host.enum_choices(draft, ty) {
        offer(
            format!("{ENUM_SWITCH}{ty}"),
            tf("wire-switch-enum", &[("type", named_type.as_str().into())]),
            "key".to_owned(),
            t("wire-switch-enum-help"),
            "switch branch enum casos bifurcar",
        );
        for choice in variants.into_iter().filter(|choice| choice.payload) {
            offer(
                format!("{VARIANT_PAYLOAD}{ty}|{}", choice.id),
                tf(
                    "wire-enum-payload",
                    &[("variant", choice.name.as_str().into())],
                ),
                "value".to_owned(),
                t("wire-enum-payload-help"),
                "enum payload carga datos",
            );
        }
    } else if let Some(fields) = host.record_fields(draft, ty) {
        for (id, name) in fields {
            offer(
                format!("{FIELD_GET}{ty}|{id}"),
                tf("wire-get-field", &[("field", name.as_str().into())]),
                "record".to_owned(),
                t("wire-get-field-help"),
                "get read field record leer campo registro",
            );
            offer(
                format!("{FIELD_SET}{ty}|{id}"),
                tf("wire-set-field", &[("field", name.as_str().into())]),
                "record".to_owned(),
                t("wire-set-field-help"),
                "set change field record cambiar campo registro",
            );
        }
    }
    choices
}

/// The search being shown, and where the node it adds goes.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct QuickAdd {
    /// The output the new node takes a wire from, when a wire was let go.
    from: Option<(NodeId, String)>,
    /// The input the new node's output is wired to, when it goes into a wire.
    into: Option<(NodeId, String)>,
    splice: Option<orange_host::EdgeId>,
    /// Where the node goes, in graph units.
    at: [f32; 2],
    /// Where the search is drawn, on the screen.
    screen: egui::Pos2,
    search: String,
    focused: bool,
    /// The choice the arrow keys are on.
    highlighted: usize,
    choices_key: Option<(String, String, (u64, u64))>,
    choices: Vec<QuickChoice>,
    splice_conversions: Option<(u64, Vec<orange_host::Conversion>)>,
}

impl QuickAdd {
    pub(crate) fn invalidate(&mut self) {
        self.choices_key = None;
        self.splice_conversions = None;
    }
}

/// One kind the search offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickChoice {
    pub definition: String,
    pub label: String,
    /// The input of the new node the wire goes to, when a wire was let go.
    pub input: Option<String>,
    /// The output of the new node the wire comes from, when a wire dragged back from an
    /// input was let go.
    pub output: Option<String>,
    pub category: String,
    /// One line on what it does, or empty.
    pub purpose: String,
}

/// Most kinds the search lists at once; typing narrows it.
const SHOWN: usize = 40;
const ENUM_SWITCH: &str = "wire:switch-enum:";
const GET_FIELD: &str = "org.orange.collections/get-field";
const SET_FIELD: &str = "org.orange.collections/set-field";
const ENUM_PAYLOAD: &str = "org.orange.collections/enum-payload";
const FIELD_GET: &str = "wire:get-field:";
const FIELD_SET: &str = "wire:set-field:";
const VARIANT_PAYLOAD: &str = "wire:enum-payload:";

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
            splice: None,
            at,
            screen,
            search: String::new(),
            focused: false,
            highlighted: 0,
            choices_key: None,
            choices: Vec::new(),
            splice_conversions: None,
        });
    }

    /// Opens the search to put a node into the wire `wire` at `at`: the chosen node
    /// takes the wire's output and its own output goes on to the wire's input.
    pub fn open_insert_node(&mut self, wire: canvas::WireEnds, at: [f32; 2], screen: egui::Pos2) {
        let splice = self.document.as_ref().and_then(|document| {
            Host::splice_edge(
                &document.draft,
                (wire.0.0, &wire.0.1),
                (wire.1.0, &wire.1.1),
            )
        });
        self.open_quick_add(Some(wire.0), at, screen);
        if let Some(quick) = &mut self.quick {
            quick.into = Some(wire.1);
            quick.splice = splice;
        }
    }

    /// Opens the search to add a node at `at` that feeds input `to`: it lists only the
    /// kinds with an output that fits, and wires the chosen node's output into it.
    pub fn open_quick_add_into(&mut self, to: (NodeId, String), at: [f32; 2], screen: egui::Pos2) {
        self.open_quick_add(None, at, screen);
        if let Some(quick) = &mut self.quick {
            quick.into = Some(to);
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
        if quick.from.iter().chain(&quick.into).any(|(node, _)| {
            self.document
                .as_ref()
                .and_then(|doc| doc.node_view(*node))
                .is_none_or(|node| node.missing.is_some())
        }) {
            return Vec::new();
        }
        // Dragged back from an input: the kinds with an output that feeds it.
        if let (None, Some(into), Some(document)) = (&quick.from, &quick.into, &self.document) {
            let givers = host.givers(&document.draft, (into.0, &into.1));
            let mut choices: Vec<_> = host
                .search_nodes(search, self.language())
                .into_iter()
                .filter_map(|found| {
                    let giver = givers
                        .iter()
                        .find(|giver| giver.definition == found.definition)?;
                    Some(QuickChoice {
                        output: Some(giver.output.clone()),
                        definition: found.definition,
                        label: found.label,
                        input: None,
                        category: found.category,
                        purpose: found.purpose,
                    })
                })
                .collect();
            let target = document.node_view(into.0).and_then(|node| {
                node.ports
                    .iter()
                    .find(|port| port.input && !port.exec && port.id == into.1)?
                    .ty
                    .as_deref()
            });
            if let Some(target) = target {
                choices.extend(
                    palette::conversion_items(host, search, self.language(), None, Some(target))
                        .into_iter()
                        .map(|item| QuickChoice {
                            output: Some("result".into()),
                            input: None,
                            definition: item.id,
                            label: item.label,
                            category: t("conversions").to_owned(),
                            purpose: item.purpose,
                        }),
                );
            }
            return choices;
        }
        let mut choices: Vec<_> = match (&quick.from, &self.document) {
            // Into a wire: the kinds that take its output and go on into its input, with
            // the same type parameters on both sides.
            (Some(from), Some(document)) if quick.into.is_some() => {
                let Some(into) = &quick.into else {
                    return Vec::new();
                };
                let Some(wire) = quick.splice else {
                    return Vec::new();
                };
                let fits =
                    host.splice_takers(&document.draft, (from.0, &from.1), (into.0, &into.1));
                let fits: HashMap<_, _> = fits
                    .iter()
                    .map(|fit| (fit.definition.as_str(), fit))
                    .collect();
                let mut choices: Vec<_> = host
                    .search_nodes(search, self.language())
                    .into_iter()
                    .filter_map(|found| {
                        let fit = fits.get(found.definition.as_str())?;
                        Some(QuickChoice {
                            output: Some(fit.output.clone()),
                            definition: found.definition,
                            label: found.label,
                            input: Some(fit.input.clone()),
                            category: found.category,
                            purpose: found.purpose,
                        })
                    })
                    .collect();
                let converters = match &quick.splice_conversions {
                    Some((generation, conversions))
                        if *generation == host.registry().generation() =>
                    {
                        std::borrow::Cow::Borrowed(conversions.as_slice())
                    }
                    _ => std::borrow::Cow::Owned(host.splice_conversions(&document.draft, wire)),
                };
                choices.extend(
                    palette::matching_conversions(converters.iter(), search, self.language())
                        .into_iter()
                        .map(|item| QuickChoice {
                            output: Some("result".into()),
                            definition: item.id,
                            label: item.label,
                            input: Some("value".into()),
                            category: t("conversions").to_owned(),
                            purpose: item.purpose,
                        }),
                );
                choices
            }
            (Some(from), Some(document)) => {
                let takers = host.takers(&document.draft, (from.0, &from.1));
                // The search's order, keeping the kinds that take the wire.
                host.search_nodes(search, self.language())
                    .into_iter()
                    .filter_map(|found| {
                        let taker = takers
                            .iter()
                            .find(|taker| taker.definition == found.definition)?;
                        Some(QuickChoice {
                            output: None,
                            definition: found.definition,
                            label: found.label,
                            input: Some(taker.input.clone()),
                            category: found.category,
                            purpose: found.purpose,
                        })
                    })
                    .collect()
            }
            _ => palette::discover(host, search, self.language())
                .into_iter()
                .map(|found| QuickChoice {
                    output: None,
                    label: found.label,
                    definition: found.definition,
                    input: None,
                    category: found.category,
                    purpose: found.purpose,
                })
                .collect(),
        };
        if quick.into.is_some() {
            return choices;
        }
        let from_type = quick.from.as_ref().and_then(|(id, port)| {
            self.document
                .as_ref()?
                .node_view(*id)?
                .ports
                .iter()
                .find(|output| !output.input && output.id == *port)?
                .ty
                .as_deref()
        });
        if let (Some(document), Some(ty)) = (&self.document, from_type) {
            let contextual = typed_wire_choices(host, &document.draft, ty, search);
            if contextual
                .iter()
                .any(|choice| choice.definition.starts_with(ENUM_SWITCH))
            {
                choices.retain(|choice| choice.definition != intrinsic::SWITCH);
            }
            if let Some(index) = choices
                .iter()
                .position(|choice| choice.definition == intrinsic::COALESCE)
            {
                let mut fallback = choices.remove(index);
                fallback.input = Some("value".to_owned());
                choices.insert(0, fallback);
            }
            choices.splice(0..0, contextual);
        }
        if quick.from.is_none() || from_type.is_some() {
            choices.extend(
                palette::conversion_items(host, search, self.language(), from_type, None)
                    .into_iter()
                    .map(|item| QuickChoice {
                        output: None,
                        definition: item.id,
                        label: item.label,
                        input: from_type.map(|_| "value".to_owned()),
                        category: t("conversions").to_owned(),
                        purpose: item.purpose,
                    }),
            );
        }
        choices
    }

    /// Adds the chosen kind where the search was opened, wiring it in when requested.
    /// A successful placement closes the search and returns the new node.
    pub fn pick_quick(&mut self, host: &mut Host, choice: &QuickChoice) -> Option<NodeId> {
        let quick = self.quick.take()?;
        let placed = self.place_quick(host, choice, &quick);
        if placed.is_none() {
            self.quick = Some(quick);
        }
        placed
    }

    fn place_quick(
        &mut self,
        host: &mut Host,
        choice: &QuickChoice,
        quick: &QuickAdd,
    ) -> Option<NodeId> {
        if let (Some(from), Some(to)) = (quick.from.as_ref(), quick.into.as_ref()) {
            let current = Host::splice_edge(self.editing()?, (from.0, &from.1), (to.0, &to.1));
            if current.is_none() || current != quick.splice {
                let _ = self.edited(
                    host,
                    Err(HostError::Document(t("splice-wire-changed").into())),
                );
                return None;
            }
            if choice.definition.starts_with(palette::CONVERSION) {
                let draft = self.editing()?;
                let wire = current?;
                let available = match &quick.splice_conversions {
                    Some((_, conversions)) => std::borrow::Cow::Borrowed(conversions.as_slice()),
                    None => std::borrow::Cow::Owned(host.splice_conversions(draft, wire)),
                };
                let converter = available
                    .iter()
                    .find(|candidate| palette::conversion_id(candidate) == choice.definition)?;
                let result = host.splice_conversion(draft, wire, converter, quick.at);
                return self.placed(host, result.map(|node| (node, None)));
            }
            return self.add_node_into_wire(
                host,
                &choice.definition,
                quick.at,
                &(from.clone(), to.clone()),
            );
        }
        if let (None, Some(into), Some(output)) = (&quick.from, &quick.into, &choice.output) {
            // Placing and wiring into the input are one action, as from an output.
            let draft = self.editing()?;
            let result = host.grouped(draft, |host, draft| {
                let placed = place_node(host, draft, &choice.definition, quick.at)?;
                if host.connect(draft, (placed.0, output), (into.0, &into.1))? {
                    Ok(placed)
                } else {
                    Err(HostError::Document("the wire could not be made".to_owned()))
                }
            });
            return self.placed(host, result);
        }
        let (Some(from), Some(input)) = (&quick.from, &choice.input) else {
            return self.add_node_at(host, &choice.definition, quick.at);
        };
        // Placing and wiring are one action: one undo takes both back, and a wire the
        // language refuses, or a document changed since the search opened, leaves the
        // graph, the selection and the history as they were.
        let typed = self
            .document
            .as_ref()
            .and_then(|doc| doc.node_view(from.0))
            .and_then(|node| {
                node.ports
                    .iter()
                    .find(|port| !port.input && port.id == from.1)
            })
            .and_then(|port| port.ty.clone());
        let draft = self.editing()?;
        let result = host.grouped(draft, |host, draft| {
            let member = wire_member(&choice.definition);
            let enum_type = choice.definition.strip_prefix(ENUM_SWITCH);
            if let Some(expected) = member.map(|(_, _, ty, _)| ty).or(enum_type)
                && typed.as_deref() != Some(expected)
            {
                return Err(HostError::Document(t("wire-type-changed").into()));
            }
            let definition = if let Some((definition, _, _, _)) = member {
                definition
            } else if enum_type.is_some() {
                intrinsic::SWITCH
            } else {
                &choice.definition
            };
            let placed = place_node(host, draft, definition, quick.at)?;
            if let Some((_, key, ty, value)) = member {
                let _ = host.set_config(
                    draft,
                    placed.0,
                    &serde_json::json!({"type": ty, key: value}),
                )?;
            }
            if !host.connect(draft, (from.0, &from.1), (placed.0, input))? {
                return Err(HostError::Document("the wire could not be made".to_owned()));
            }
            if enum_type.is_some() {
                let _ = host.add_enum_cases(draft, placed.0)?;
            }
            Ok(placed)
        });
        self.placed(host, result)
    }

    /// Draws the open search over the canvas: a field to type in and the matching
    /// kinds, each with its category and what it is for. The arrow keys move, Enter
    /// takes the one they are on, Escape or a click elsewhere closes it.
    pub(crate) fn quick_add_popup(&mut self, ctx: &egui::Context, host: &mut Host) {
        let Some(quick) = &self.quick else {
            return;
        };
        let wired = quick.from.is_some() || quick.into.is_some();
        let mut search = quick.search.clone();
        let mut focused = quick.focused;
        let mut highlighted = quick.highlighted;
        let screen = quick.screen;
        let was_focused = quick.focused;
        let mut picked = None;
        let mut close = false;
        let mut dragged = false;
        let bounds = ctx.content_rect().shrink(6.0);
        let shown = orange_ui_controls::anchored_popup(
            ctx,
            egui::Id::unique("orange-quick-add"),
            screen,
            304.0,
            |ui| {
                ui.strong(if wired {
                    t("connect-a-node")
                } else {
                    t("add-a-node")
                });
                // The arrows move the choice, not the caret.
                let [up, down] =
                    controls::consume_plain_keys(ui, [egui::Key::ArrowUp, egui::Key::ArrowDown]);
                let field = controls::search(
                    ui,
                    "quick-add-search",
                    &mut search,
                    if wired {
                        t("quick-add-wire-hint")
                    } else {
                        t("quick-add-hint")
                    },
                );
                if field.changed() {
                    highlighted = 0;
                }
                let key = (
                    search.clone(),
                    self.language().to_owned(),
                    host.search_revision(),
                );
                if let Some(quick) = &mut self.quick
                    && let (Some(wire), Some(document)) = (quick.splice, &self.document)
                    && quick
                        .splice_conversions
                        .as_ref()
                        .is_none_or(|(generation, _)| *generation != key.2.0)
                {
                    quick.splice_conversions =
                        Some((key.2.0, host.splice_conversions(&document.draft, wire)));
                }
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
                            let target = match (choice.input.as_deref(), choice.output.as_deref()) {
                                (Some(input), Some(output)) => {
                                    Some(format!("{} → {}", port_label(input), port_label(output)))
                                }
                                (input, output) => input.or(output).map(port_label),
                            };
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
                                Some(&choice.definition),
                            );
                            if !wired && row.drag_started() {
                                let card =
                                    self.previews.card(host, draft, &choice.definition).cloned();
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
            },
        );
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

#[cfg(test)]
mod staging_tests {
    use super::*;

    fn editor_with(host: &mut Host) -> (Editor, NodeId) {
        let mut editor = Editor::new();
        editor.open(host, "graph.json", orange_host::new_document().unwrap());
        let trigger = editor.add_node(host, intrinsic::ON_MANUAL).unwrap();
        (editor, trigger)
    }

    #[test]
    fn typed_wires_configure_enum_cases_and_record_members_in_one_undo_step() {
        use serde_json::json;
        let (mut host, _) = Host::new(orange_host::HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        let mut document: serde_json::Value =
            serde_json::from_str(&orange_host::new_document().unwrap()).unwrap();
        document["types"] = json!([
            {"id": "6c0e4a52-8b1f-4a5e-9c3d-1e2f3a4b5c6d", "name":"Mood", "kind":"enum",
                "variants":[{"id":"calm", "name":"Calm"}, {"id":"message", "name":"Message","payload":"core/string@1"}]},
            {"id": "6c0e4a52-8b1f-4a5e-9c3d-1e2f3a4b5c6e", "name":"Profile", "kind":"record",
                "fields":[{"id":"display", "name":"Display name", "type":"core/string@1", "optional":true}]}
        ]);
        editor.open_untitled(&mut host, document.to_string());
        for (ty, literal, search, prefix, kind) in [
            (
                "local/t6c0e4a528b1f4a5e9c3d1e2f3a4b5c6d@1",
                json!({"variant":"calm"}),
                "switch mood",
                ENUM_SWITCH,
                intrinsic::SWITCH,
            ),
            (
                "local/t6c0e4a528b1f4a5e9c3d1e2f3a4b5c6d@1",
                json!({"variant":"calm"}),
                "message payload",
                VARIANT_PAYLOAD,
                ENUM_PAYLOAD,
            ),
            (
                "local/t6c0e4a528b1f4a5e9c3d1e2f3a4b5c6e@1",
                json!({}),
                "get display",
                FIELD_GET,
                GET_FIELD,
            ),
            (
                "local/t6c0e4a528b1f4a5e9c3d1e2f3a4b5c6e@1",
                json!({}),
                "set display",
                FIELD_SET,
                SET_FIELD,
            ),
        ] {
            let source = editor.add_node(&mut host, intrinsic::VALUE).unwrap();
            assert!(editor.choose_type(&mut host, source, "T", Some(ty)));
            assert!(editor.set_input(
                &mut host,
                (source, "value"),
                Some(&json!({"type":ty, "value":literal}).to_string())
            ));
            let before = editor.graph().unwrap().wires.len();
            editor.open_quick_add(
                Some((source, "result".into())),
                [320.0, 0.0],
                egui::Pos2::ZERO,
            );
            let choice = editor
                .quick_choices(&host, search)
                .into_iter()
                .find(|choice| choice.definition.starts_with(prefix))
                .expect(search);
            let node = editor.pick_quick(&mut host, &choice).expect(search);
            let graph = editor.graph().unwrap();
            let node = graph.nodes.iter().find(|item| item.id == node).unwrap();
            assert_eq!(node.definition, kind);
            if kind == intrinsic::SWITCH {
                assert!(
                    node.ports
                        .iter()
                        .any(|port| port.label.as_deref() == Some("Calm"))
                );
                assert!(
                    node.ports
                        .iter()
                        .any(|port| port.label.as_deref() == Some("Message"))
                );
            } else if kind == ENUM_PAYLOAD {
                assert_eq!(node.settings["variant"], "message");
            } else {
                assert_eq!(node.settings["field"], "display");
            }
            assert_eq!(graph.wires.len(), before + 1);
            editor.undo(&mut host);
            assert_eq!(editor.graph().unwrap().wires.len(), before);

            editor.open_quick_add(
                Some((source, "result".into())),
                [320.0, 0.0],
                egui::Pos2::ZERO,
            );
            assert!(editor.set_input(&mut host, (source, "value"), None));
            assert!(editor.choose_type(&mut host, source, "T", Some("core/bool@1")));
            let before_retry = editor.text().unwrap();
            assert!(editor.pick_quick(&mut host, &choice).is_none());
            assert_eq!(editor.text().unwrap(), before_retry);
        }
    }

    #[test]
    fn an_optional_wire_offers_fallback_first_and_wires_the_optional_input() {
        let (mut host, _) = Host::new(orange_host::HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        editor.open_untitled(&mut host, orange_host::new_document().unwrap());
        let source = editor.add_node(&mut host, intrinsic::VALUE).unwrap();
        assert!(editor.choose_type(&mut host, source, "T", Some("core/option@1<core/string@1>")));
        editor.open_quick_add(
            Some((source, "result".into())),
            [320.0, 0.0],
            egui::Pos2::ZERO,
        );
        let choices = editor.quick_choices(&host, "");
        assert_eq!(choices[0].definition, intrinsic::COALESCE);
        let added = editor.pick_quick(&mut host, &choices[0]).unwrap();
        assert!(
            editor
                .graph()
                .unwrap()
                .wires
                .iter()
                .any(|wire| wire.to == (added, "value".into()))
        );
    }

    /// Letting a wire go and choosing a kind places and wires the node as one action:
    /// one undo takes both back.
    #[test]
    fn a_dropped_wire_and_its_choice_are_one_undo_step() {
        let (mut host, _events) = Host::new(orange_host::HostOptions::default()).unwrap();
        let (mut editor, trigger) = editor_with(&mut host);
        let nodes = |editor: &Editor| editor.graph().unwrap().nodes.len();
        let wires = |editor: &Editor| editor.graph().unwrap().wires.len();
        editor.open_quick_add(
            Some((trigger, "then".to_owned())),
            [320.0, 0.0],
            egui::pos2(10.0, 10.0),
        );
        let choice = editor
            .quick_choices(&host, "sequence")
            .into_iter()
            .find(|choice| choice.definition == intrinsic::SEQUENCE)
            .expect("a sequence takes a step");
        let node = editor.pick_quick(&mut host, &choice).unwrap();
        assert_eq!((nodes(&editor), wires(&editor)), (2, 1));
        assert_eq!(editor.canvas().selected(), Some(node));
        editor.undo(&mut host);
        assert_eq!((nodes(&editor), wires(&editor)), (1, 0));
        editor.redo(&mut host);
        assert_eq!((nodes(&editor), wires(&editor)), (2, 1));
    }

    /// A choice whose wire is refused leaves the graph, the selection and the history
    /// as they were.
    #[test]
    fn a_refused_choice_changes_nothing() {
        let (mut host, _events) = Host::new(orange_host::HostOptions::default()).unwrap();
        let (mut editor, trigger) = editor_with(&mut host);
        editor.canvas().select(Some(trigger));
        let before = editor.document.as_ref().unwrap().draft.can_undo();
        editor.open_quick_add(
            Some((trigger, "then".to_owned())),
            [320.0, 0.0],
            egui::pos2(10.0, 10.0),
        );
        let refused = QuickChoice {
            definition: intrinsic::SEQUENCE.to_owned(),
            label: "Sequence".to_owned(),
            input: Some("no-such-port".to_owned()),
            output: None,
            category: String::new(),
            purpose: String::new(),
        };
        assert!(editor.pick_quick(&mut host, &refused).is_none());
        assert_eq!(editor.graph().unwrap().nodes.len(), 1);
        assert_eq!(editor.canvas().selected(), Some(trigger));
        assert_eq!(editor.document.as_ref().unwrap().draft.can_undo(), before);
    }
    /// A wire dragged back from an input lists only kinds with an output that feeds it,
    /// and the choice is placed and wired as one action.
    #[test]
    fn a_wire_dragged_back_from_an_input_offers_what_feeds_it() {
        let (mut host, _events) = Host::new(orange_host::HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        editor.open(
            &mut host,
            "graph.json",
            orange_host::new_document().unwrap(),
        );
        let sequence = editor.add_node(&mut host, intrinsic::SEQUENCE).unwrap();
        let nodes = |editor: &Editor| editor.graph().unwrap().nodes.len();
        editor.open_quick_add_into(
            (sequence, "in".to_owned()),
            [-320.0, 0.0],
            egui::pos2(10.0, 10.0),
        );
        let all = editor.quick_choices(&host, "");
        assert!(!all.is_empty() && all.iter().all(|choice| choice.output.is_some()));
        let manual = all
            .iter()
            .find(|choice| choice.definition == intrinsic::ON_MANUAL)
            .cloned()
            .expect("a trigger starts a step");
        let node = editor.pick_quick(&mut host, &manual).unwrap();
        assert!(
            editor
                .graph()
                .unwrap()
                .wires
                .iter()
                .any(|wire| wire.from == (node, "then".to_owned()) && wire.to.0 == sequence)
        );
        assert_eq!(nodes(&editor), 2);
        editor.undo(&mut host);
        assert_eq!(nodes(&editor), 1);
        assert!(editor.graph().unwrap().wires.is_empty());
    }

    #[test]
    fn a_converter_can_feed_a_value_input_in_one_undo_step() {
        let (mut host, _) = Host::new(orange_host::HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        editor.open_untitled(&mut host, orange_host::new_document().unwrap());
        let target = editor
            .add_node_at(&mut host, intrinsic::TEXT, [0.0, 0.0])
            .unwrap();
        editor.open_quick_add_into((target, "value".into()), [-320.0, 0.0], egui::Pos2::ZERO);
        let choices = editor.quick_choices(&host, "convert");
        let mut converters = choices
            .iter()
            .filter(|choice| choice.definition.starts_with(palette::CONVERSION));
        let first = converters.next().unwrap();
        for choice in std::iter::once(first).chain(converters) {
            let converter = palette::conversion_named(&host, &choice.definition).unwrap();
            assert_eq!(converter.to, "core/string@1");
            assert_eq!(choice.output.as_deref(), Some("result"));
        }
        let placed = editor.pick_quick(&mut host, first).unwrap();
        let graph = editor.graph().unwrap();
        assert_eq!(graph.wires[0].from, (placed, "result".into()));
        assert_eq!(graph.wires[0].to, (target, "value".into()));
        editor.undo(&mut host);
        let graph = editor.graph().unwrap();
        assert_eq!(graph.nodes.len(), 1);
        assert!(graph.wires.is_empty());
    }

    /// A conversion chosen for a dropped value wire is placed and wired as one action.
    #[test]
    fn a_conversion_for_a_dropped_wire_is_one_undo_step() {
        let (mut host, _events) = Host::new(orange_host::HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        editor.open(
            &mut host,
            "graph.json",
            orange_host::new_document().unwrap(),
        );
        let number = editor
            .add_node_at(&mut host, intrinsic::NUMBER, [0.0, 0.0])
            .unwrap();
        editor.open_quick_add(
            Some((number, "result".into())),
            [300.0, 0.0],
            egui::Pos2::ZERO,
        );
        let conversion = editor
            .quick_choices(&host, "duration milliseconds")
            .into_iter()
            .find(|choice| {
                choice
                    .definition
                    .starts_with("convert:core/f64-to-duration")
            })
            .unwrap();
        let node = editor.pick_quick(&mut host, &conversion).unwrap();
        assert!(
            editor
                .graph()
                .unwrap()
                .wires
                .iter()
                .any(|wire| wire.from.0 == number && wire.to.0 == node)
        );
        editor.undo(&mut host);
        assert_eq!(editor.graph().unwrap().nodes.len(), 1);
        assert!(editor.graph().unwrap().wires.is_empty());
    }

    #[test]
    fn a_converter_can_be_selected_for_a_generic_splice() {
        let (mut host, _events) = Host::new(orange_host::HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        editor.open(
            &mut host,
            "graph.json",
            orange_host::new_document().unwrap(),
        );
        let whole = editor.add_node(&mut host, intrinsic::WHOLE_NUMBER).unwrap();
        let target = editor
            .add_node(&mut host, "org.orange.math/absolute")
            .unwrap();
        assert!(editor.connect(&mut host, (whole, "result"), (target, "value")));
        let wire = ((whole, "result".into()), (target, "value".into()));
        editor.open_insert_node(wire.clone(), [150.0, 0.0], egui::Pos2::ZERO);
        let conversion = editor
            .quick_choices(&host, "convert")
            .into_iter()
            .find(|choice| choice.definition.starts_with("convert:core/i64-to-f64"))
            .expect("a generic numeric destination can re-infer as a decimal");
        let node = editor.pick_quick(&mut host, &conversion).unwrap();
        assert_eq!(editor.canvas().selected(), Some(node));
        assert_eq!(editor.graph().unwrap().wires.len(), 2);
        editor.undo(&mut host);
        assert_eq!(editor.graph().unwrap().wires.len(), 1);

        editor.open_insert_node(wire, [150.0, 0.0], egui::Pos2::ZERO);
        assert!(editor.disconnect_wire(&mut host, (whole, "result"), (target, "value")));
        assert!(editor.connect(&mut host, (whole, "result"), (target, "value")));
        let before = editor.document.as_ref().unwrap().draft.text().unwrap();
        assert!(editor.pick_quick(&mut host, &conversion).is_none());
        assert!(editor.is_quick_add_open());
        assert_eq!(
            editor.document.as_ref().unwrap().draft.text().unwrap(),
            before
        );
    }
}
