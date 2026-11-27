//! Structured editor diagnostics and the runtime log.
use super::*;
use crate::controls::{Action, Dropdown as _, Surface};
use orange_host::Diagnostic;
#[cfg(test)]
use orange_host::intrinsic;
use std::collections::{BTreeSet, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteLevel {
    Critical,
    Error,
    Warning,
    Info,
    Debug,
    Trace,
}

impl NoteLevel {
    const ALL: [Self; 6] = [
        Self::Critical,
        Self::Error,
        Self::Warning,
        Self::Info,
        Self::Debug,
        Self::Trace,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Critical => "CRITICAL",
            Self::Error => "ERROR",
            Self::Warning => "WARN",
            Self::Info => "INFO",
            Self::Debug => "DEBUG",
            Self::Trace => "TRACE",
        }
    }

    fn name(self) -> &'static str {
        t(match self {
            Self::Critical => "log-level-critical",
            Self::Error => "log-level-error",
            Self::Warning => "log-level-warning",
            Self::Info => "log-level-info",
            Self::Debug => "log-level-debug",
            Self::Trace => "log-level-trace",
        })
    }

    fn color(self, ui: &egui::Ui) -> egui::Color32 {
        match self {
            Self::Critical | Self::Error => ui.visuals().error_fg_color,
            Self::Warning => ui.visuals().warn_fg_color,
            Self::Info => ui.visuals().text_color(),
            Self::Debug | Self::Trace => ui.visuals().weak_text_color(),
        }
    }
}

#[derive(Debug)]
pub(super) struct LogLine {
    level: NoteLevel,
    pub(super) text: String,
    repeated: u32,
    display: Option<String>,
}

impl LogLine {
    pub(super) fn display(&self) -> &str {
        self.display.as_deref().unwrap_or(&self.text)
    }

    fn update_display(&mut self) {
        let labeled = match self.level {
            NoteLevel::Info => None,
            NoteLevel::Warning => Some(tf("log-warning", &[("text", self.text.as_str().into())])),
            NoteLevel::Error => Some(tf("log-error", &[("text", self.text.as_str().into())])),
            level => Some(format!("{} {}", level.label(), self.text)),
        };
        self.display = if self.repeated > 1 {
            Some(tf(
                "log-repeated",
                &[
                    ("line", labeled.as_deref().unwrap_or(&self.text).into()),
                    ("count", self.repeated.into()),
                ],
            ))
        } else {
            labeled
        };
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProblemLevel {
    Blocker,
    Warning,
    Draft,
}

impl ProblemLevel {
    const ALL: [Self; 3] = [Self::Blocker, Self::Warning, Self::Draft];

    pub(crate) fn label(self) -> &'static str {
        t(match self {
            Self::Blocker => "problem-blocker",
            Self::Warning => "problem-warning",
            Self::Draft => "problem-draft",
        })
    }

    fn hint(self) -> &'static str {
        t(match self {
            Self::Blocker => "problem-blocker-hint",
            Self::Warning => "problem-warning-hint",
            Self::Draft => "problem-draft-hint",
        })
    }

    pub(crate) fn glyph(self) -> char {
        match self {
            Self::Blocker => icons::ERROR,
            Self::Warning => icons::WARNING,
            Self::Draft => icons::EDIT,
        }
    }

    fn icon(self, ui: &egui::Ui) -> egui::RichText {
        let visuals = ui.visuals();
        let color = match self {
            Self::Blocker => visuals.error_fg_color,
            Self::Warning => visuals.warn_fg_color,
            Self::Draft => visuals.weak_text_color(),
        };
        icons::text(self.glyph()).color(color)
    }
}

#[derive(Debug)]
pub(super) struct ProblemLocation {
    pub(super) node: Option<NodeId>,
    pub(super) level: ProblemLevel,
    output: Option<orange_host::PortKey>,
    code: &'static str,
}

impl ProblemLocation {
    fn clipboard_line(&self, text: &str) -> String {
        if self.code.is_empty() {
            format!("[{}] {text}", self.level.label())
        } else {
            format!("[{}] {} — {text}", self.level.label(), self.code)
        }
    }
}

/// A problem as the Problems panel and the canvas marks say it: the message, then the
/// card, port and variable it is about by their names. The code stays searchable in the
/// panel without leading the line.
pub(super) fn problem_text(diagnostic: &Diagnostic, names: &Names) -> String {
    let subject = &diagnostic.subject;
    let mut place = Vec::new();
    if let Some(node) = &subject.node {
        place.push(names.node(node));
    }
    if let Some(port) = &subject.port {
        place.push(port_label(&port.to_string()));
    }
    if let Some(variable) = &subject.variable {
        place.push(names.variable(variable));
    }
    let message = diagnostic
        .message_key
        .strip_prefix("core/")
        .and_then(|key| i18n::catalogs().message(&i18n::language(), key))
        .unwrap_or_else(|| diagnostic.message.as_str().into());
    if place.is_empty() {
        message.into_owned()
    } else {
        format!("{message} — {}", place.join(" · "))
    }
}

fn top_node(diagnostic: &Diagnostic) -> Option<NodeId> {
    diagnostic
        .subject
        .node
        .filter(|_| diagnostic.subject.function.is_none())
}

/// Authoring errors expected while filling out a node. Graph structure, provider
/// integrity, declarations and edge errors keep their normal validation severity.
fn can_be_draft(diagnostic: &Diagnostic) -> bool {
    diagnostic.subject.edge.is_none()
        && matches!(
            diagnostic.code.as_str(),
            "W-UNREACHABLE"
                | "E-INPUT-REQUIRED"
                | "E-TYPE-AMBIGUOUS"
                | "E-CONFIG-INVALID"
                | "E-VARIABLE-UNKNOWN"
                | "E-CONNECTION-UNKNOWN"
                | "E-EVENT-UNKNOWN"
                | "E-CALL-TARGET-MISSING"
                | "E-CALL-TARGET-UNKNOWN"
                | "E-NOMINAL-CONFIG"
                | "E-REGISTER-CONFIG"
                | "E-WATCH-CONFIG"
                | "E-TIMER-CONFIG"
                | "E-MANUAL-PAYLOAD"
                | "E-ACTIVATION-POLICY"
                | "E-LOOP-UNBOUNDED"
                | "E-ZONE-STRUCTURE"
        )
}

impl Document {
    /// Draft cards may have incoming data if they are actions with no execution
    /// wire or consumers. Passive sources and sinks can run on data alone.
    fn draft_nodes(&self, diagnostics: &[Diagnostic]) -> BTreeSet<NodeId> {
        let mut candidates = BTreeSet::new();
        let mut required = HashSet::new();
        for diagnostic in diagnostics {
            let Some(node) = top_node(diagnostic) else {
                continue;
            };
            let draft = can_be_draft(diagnostic);
            if diagnostic.severity == orange_host::Severity::Error && !draft {
                required.insert(node);
            }
            if draft {
                candidates.insert(node);
            }
        }
        if candidates.is_empty() {
            return candidates;
        }
        let mut data_only_actions = HashSet::new();
        let mut visible_drafts = BTreeSet::new();
        for node in &self.graph.nodes {
            if candidates.contains(&node.id) {
                visible_drafts.insert(node.id);
            }
            if !node.entry && node.ports.iter().any(|port| port.input && port.exec) {
                data_only_actions.insert(node.id);
            }
        }
        for wire in &self.graph.wires {
            required.insert(wire.from.0);
            if wire.exec || !data_only_actions.contains(&wire.to.0) {
                required.insert(wire.to.0);
            }
        }
        required.extend(
            self.draft
                .document()
                .exports()
                .iter()
                .map(|export| export.node),
        );
        visible_drafts.retain(|node| !required.contains(node));
        visible_drafts
    }

    pub(super) fn set_problem_error(&mut self, error: &HostError) {
        self.run_blocked = true;
        match error {
            HostError::Invalid(diagnostics) => {
                let mut marks = canvas::Marks::default();
                for diagnostic in diagnostics {
                    self.record_problem(diagnostic, None, false, &mut marks);
                }
            }
            other => {
                self.problems.push(other.to_string());
                self.problem_locations.push(ProblemLocation {
                    node: None,
                    output: None,
                    level: ProblemLevel::Blocker,
                    code: "",
                });
            }
        }
    }

    pub(super) fn record_problem(
        &mut self,
        diagnostic: &Diagnostic,
        wire: Option<canvas::WireEnds>,
        draft: bool,
        marks: &mut canvas::Marks,
    ) {
        let node = top_node(diagnostic);
        let level = if draft && can_be_draft(diagnostic) {
            ProblemLevel::Draft
        } else if diagnostic.severity == orange_host::Severity::Error {
            ProblemLevel::Blocker
        } else {
            ProblemLevel::Warning
        };
        let error = level == ProblemLevel::Blocker;
        self.run_blocked |= error;
        let text = problem_text(diagnostic, &self.names);
        if let Some(wire) = wire {
            canvas::Marks::add(&mut marks.wires, wire, error, text.clone());
        } else if let Some(node) = node {
            canvas::Marks::add(&mut marks.nodes, node, error, text.clone());
            if let Some(port) = &diagnostic.subject.port {
                canvas::Marks::add(
                    &mut marks.ports,
                    (node, port.to_string(), diagnostic.subject.port_direction),
                    error,
                    text.clone(),
                );
            }
        }
        self.problem_locations.push(ProblemLocation {
            node,
            output: diagnostic
                .subject
                .port
                .as_ref()
                .filter(|_| {
                    diagnostic.subject.port_direction != Some(orange_host::PortDirection::Input)
                })
                .cloned(),
            level,
            code: diagnostic.code.as_str(),
        });
        self.problems.push(text);
    }

    pub(super) fn check_problems(&mut self, host: &Host) -> canvas::Marks {
        let mut marks = canvas::Marks::default();
        self.problems.clear();
        self.problem_locations.clear();
        self.run_blocked = false;
        self.run_omitted.clear();
        let diagnostics = match host.diagnostics(&mut self.draft) {
            Ok(diagnostics) => diagnostics,
            Err(error) => {
                self.set_problem_error(&error);
                return marks;
            }
        };
        let drafts = self.draft_nodes(&diagnostics);
        self.run_omitted.extend(drafts.iter().copied());
        for diagnostic in &diagnostics {
            let draft = top_node(diagnostic).is_some_and(|node| drafts.contains(&node));
            let wire = host.wire_of(&self.draft, diagnostic);
            self.record_problem(diagnostic, wire, draft, &mut marks);
        }
        // Removing a draft wire can change inference. Removing an isolated zone
        // marker can affect its partner. Check the remaining graph in either case.
        if !self.run_blocked
            && (self
                .graph
                .wires
                .iter()
                .any(|wire| drafts.contains(&wire.to.0))
                || diagnostics.iter().any(|diagnostic| {
                    diagnostic.code.as_str() == "E-ZONE-STRUCTURE"
                        && top_node(diagnostic).is_some_and(|node| drafts.contains(&node))
                }))
        {
            let projected = self
                .runtime_text(host)
                .and_then(|text| host.draft(&text))
                .and_then(|mut draft| host.problems(&mut draft));
            match projected {
                Ok(problems) => {
                    for diagnostic in problems
                        .iter()
                        .filter(|diagnostic| diagnostic.severity == orange_host::Severity::Error)
                    {
                        self.record_problem(diagnostic, None, false, &mut marks);
                    }
                }
                Err(error) => self.set_problem_error(&error),
            }
        }
        marks
    }
}

impl Editor {
    /// Records a graph message and folds consecutive repeats at the same level.
    pub fn note(&mut self, level: NoteLevel, text: &str) {
        if self.last_note
            && let Some(last) = self.log.back_mut()
            && last.level == level
            && last.text == text
        {
            last.repeated = last.repeated.saturating_add(1);
            last.update_display();
            return;
        }
        self.say_at(level, text.to_owned());
        if let Some(line) = self.log.back_mut() {
            line.update_display();
        }
        self.last_note = true;
    }

    /// Reports messages dropped from the graph's bounded log queue.
    pub fn notes_dropped(&mut self, count: u64) {
        if count > 0 {
            self.say_at(
                NoteLevel::Warning,
                tf("log-dropped", &[("count", count.into())]),
            );
        }
    }

    pub(super) fn say(&mut self, line: String) {
        self.say_at(NoteLevel::Info, line);
    }

    pub(super) fn say_at(&mut self, level: NoteLevel, text: String) {
        self.last_note = false;
        if self.log.len() >= MAX_LOG_LINES {
            let _ = self.log.pop_front();
        }
        self.log.push_back(LogLine {
            level,
            text,
            repeated: 1,
            display: None,
        });
    }

    /// Draws the current diagnostics tab in the supplied container.
    pub fn log_panel(&mut self, ui: &mut egui::Ui) {
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(8, 6))
            .show(ui, |ui| {
                ui.set_min_height(ui.available_height());
                if self.log_events {
                    self.log_content(ui);
                } else {
                    self.problems_content(ui);
                }
            });
    }

    fn log_content(&mut self, ui: &mut egui::Ui) {
        orange_ui_controls::action_row(
            ui,
            120.0,
            |ui| {
                let selection = self.log_filter.map_or_else(
                    || t("log-all-levels").to_owned(),
                    |level| tf("log-level-selected", &[("level", level.name().into())]),
                );
                egui::ComboBox::from_id_salt("log-level-filter")
                    .width(124.0)
                    .selected_text(selection)
                    .show_select(ui, Surface::Panel, |ui| {
                        ui.selectable_value(&mut self.log_filter, None, t("log-all-levels"));
                        for level in NoteLevel::ALL {
                            ui.selectable_value(&mut self.log_filter, Some(level), level.name());
                        }
                    });
                let trace_label = t(if self.live_trace {
                    "live-trace-on"
                } else {
                    "live-trace-off"
                });
                let trace = controls::button(
                    ui,
                    Surface::Panel,
                    Action::Toggle(self.live_trace),
                    true,
                    egui::Button::new((icons::text(icons::WATCH), trace_label)),
                )
                .on_hover_text(t("live-trace-hint"));
                trace.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::Role::CheckBox,
                        true,
                        self.live_trace,
                        trace_label,
                    )
                });
                if trace.clicked() {
                    self.live_trace = !self.live_trace;
                }
                if controls::button(
                    ui,
                    Surface::Panel,
                    Action::Menu,
                    !self.log.is_empty(),
                    egui::Button::new((icons::centered(icons::DELETE), t("clear-log"))),
                )
                .on_hover_text(t("clear-log-hint"))
                .clicked()
                {
                    self.log.clear();
                    self.last_note = false;
                }
            },
            |ui| search_field(ui, "log-search", &mut self.log_search, t("search-log")),
        );
        ui.separator();
        let query = orange_host::SearchQuery::new(&self.log_search);
        let mut text = String::new();
        let mut levels = Vec::new();
        for line in &self.log {
            if self.log_filter.is_some_and(|level| level != line.level)
                || !query.contains([line.text.as_str(), line.level.label(), line.level.name()])
            {
                continue;
            }
            if !text.is_empty() {
                text.push('\n');
            }
            let start = text.len();
            text.push_str(line.level.label());
            text.push_str("  ");
            levels.push((start..text.len(), line.level));
            if line.repeated > 1 {
                text.push_str(&tf(
                    "log-repeated",
                    &[
                        ("line", line.text.as_str().into()),
                        ("count", line.repeated.into()),
                    ],
                ));
            } else {
                text.push_str(&line.text);
            }
        }
        if text.is_empty() {
            ui.weak(t(if self.log.is_empty() {
                "no-log-messages"
            } else {
                "no-log-results"
            }));
            return;
        }
        egui::ScrollArea::vertical()
            .id_salt("editor-events")
            .stick_to_bottom(true)
            .show(ui, |ui| {
                let mut layout = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, width: f32| {
                    let mut job = egui::text::LayoutJob::default();
                    job.wrap.max_width = width;
                    let format = |color| egui::TextFormat {
                        font_id: egui::TextStyle::Monospace.resolve(ui.style()),
                        color,
                        ..Default::default()
                    };
                    let normal = format(ui.visuals().text_color());
                    let text = buffer.as_str();
                    let mut end = 0;
                    for (range, level) in &levels {
                        job.append(&text[end..range.start], 0.0, normal.clone());
                        job.append(&text[range.clone()], 0.0, format(level.color(ui)));
                        end = range.end;
                    }
                    job.append(&text[end..], 0.0, normal);
                    ui.fonts_mut(|fonts| fonts.layout_job(job))
                };
                ui.add(
                    egui::TextEdit::multiline(&mut text.as_str())
                        .id(ui.make_persistent_id((
                            "editor-log-text",
                            self.log_filter.map(NoteLevel::label),
                            self.log_search.trim(),
                        )))
                        .layouter(&mut layout)
                        .desired_rows(1)
                        .desired_width(f32::INFINITY)
                        .frame(egui::Frame::NONE),
                );
            });
    }

    fn problems_content(&mut self, ui: &mut egui::Ui) {
        let Some(document) = &self.document else {
            ui.weak(t("no-problems"));
            return;
        };
        orange_ui_controls::action_row(
            ui,
            120.0,
            |ui| {
                let filter_text = self
                    .problems_filter
                    .map_or(t("filter-all"), ProblemLevel::label);
                egui::ComboBox::from_id_salt("problems-filter")
                    .width(96.0)
                    .selected_text(filter_text)
                    .show_select(ui, Surface::Panel, |ui| {
                        ui.selectable_value(&mut self.problems_filter, None, t("filter-all"));
                        for level in ProblemLevel::ALL {
                            ui.selectable_value(
                                &mut self.problems_filter,
                                Some(level),
                                level.label(),
                            );
                        }
                    });
            },
            |ui| {
                search_field(
                    ui,
                    "problems-search",
                    &mut self.problems_search,
                    t("search-problems"),
                )
            },
        );
        let query = orange_host::SearchQuery::new(&self.problems_search);
        let visible = ui
            .horizontal(|ui| {
                let mut visible = Vec::new();
                for level in ProblemLevel::ALL {
                    if self.problems_filter.is_some_and(|wanted| level != wanted) {
                        continue;
                    }
                    visible.extend(
                        document
                            .problems
                            .iter()
                            .enumerate()
                            .zip(&document.problem_locations)
                            .filter(|((_, text), location)| {
                                location.level == level
                                    && query.contains([text.as_str(), level.label(), location.code])
                            })
                            .map(|((index, text), location)| (index, text, location)),
                    );
                }
                if !self.selected_problems.is_empty() {
                    let clear = controls::button(
                        ui,
                        Surface::Panel,
                        Action::Menu,
                        true,
                        egui::Button::new(t("clear-selection")),
                    );
                    if clear.clicked() {
                        self.selected_problems.clear();
                    }
                    let copy_selected = controls::button(
                        ui,
                        Surface::Panel,
                        Action::Menu,
                        true,
                        egui::Button::new(t("copy-selected-problems")),
                    );
                    if copy_selected.clicked() {
                        let lines = self
                            .selected_problems
                            .iter()
                            .filter_map(|index| {
                                document
                                    .problems
                                    .get(*index)
                                    .zip(document.problem_locations.get(*index))
                                    .map(|(text, location)| location.clipboard_line(text))
                            })
                            .collect::<Vec<_>>();
                        ui.ctx().copy_text(lines.join("\n"));
                    }
                }
                visible
            })
            .inner;
        if !self.selected_problems.is_empty()
            && ui.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::C))
        {
            let lines = self
                .selected_problems
                .iter()
                .filter_map(|index| {
                    document
                        .problems
                        .get(*index)
                        .zip(document.problem_locations.get(*index))
                        .map(|(text, location)| {
                            let severity = location.level.label();
                            let code = if location.code.is_empty() {
                                String::new()
                            } else {
                                format!(" [{}]", location.code)
                            };
                            let node = location
                                .node
                                .map(|node| document.names.node(&node))
                                .filter(|name| !name.is_empty())
                                .map(|name| format!("Node: {name}\n"))
                                .unwrap_or_default();
                            format!("{severity}{code}\n{node}{text}")
                        })
                })
                .collect::<Vec<_>>();
            ui.ctx().copy_text(lines.join("\n\n"));
        }
        ui.separator();
        let mut focus = None;
        egui::ScrollArea::vertical()
            .id_salt("editor-problems")
            .show(ui, |ui| {
                if visible.is_empty() {
                    ui.weak(t(if document.problems.is_empty() {
                        "no-problems"
                    } else {
                        "no-problem-results"
                    }));
                }
                for (index, text, location) in visible {
                    let level = location.level;
                    ui.horizontal_top(|ui| {
                        let mut selected = self.selected_problems.contains(&index);
                        let _ = orange_ui_controls::checkbox(
                            ui,
                            Surface::Panel,
                            orange_ui_controls::CheckStyle::Simple,
                            &mut selected,
                            "",
                        );
                        if selected {
                            self.selected_problems.insert(index);
                        } else {
                            self.selected_problems.remove(&index);
                        }
                        ui.label(level.icon(ui)).on_hover_text(level.hint());
                        if let Some(node) = location.node {
                            let label = t("show-node");
                            let response = controls::button(
                                ui,
                                Surface::Panel,
                                Action::Menu,
                                true,
                                egui::Button::new(icons::centered(icons::FIT)),
                            )
                            .on_hover_text(label);
                            response.widget_info(|| {
                                egui::WidgetInfo::labeled(egui::Role::Button, true, label)
                            });
                            if response.clicked() {
                                focus =
                                    Some((node, location.output.as_ref().map(ToString::to_string)));
                            }
                        }
                        ui.add(egui::Label::new(text).wrap().selectable(true));
                    });
                }
            });
        if let Some((node, port)) = focus {
            self.canvas.focus_problem(&document.graph, node, port);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_selection_survives_scrolling_and_copies_multiple_lines() {
        let mut editor = Editor::new();
        for index in 0..96 {
            editor.say(format!("event {index}"));
        }
        let context = egui::Context::default();
        icons::install(&context);
        let mut frame = |events| {
            context.run_ui(
                egui::RawInput {
                    events,
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(480.0, 240.0),
                    )),
                    ..Default::default()
                },
                |ui| editor.log_content(ui),
            )
        };
        let log_position = |output: &egui::FullOutput| {
            output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text().starts_with("INFO  event 0") => {
                        Some((text.pos, shape.clip_rect.center().y))
                    }
                    _ => None,
                })
                .unwrap()
        };
        frame(vec![]).drop_without_applying_deltas();
        let output = frame(vec![]);
        let (original, y) = log_position(&output);
        output.drop_without_applying_deltas();
        let pointer = egui::pos2(original.x + 24.0, y);
        for pressed in [true, false] {
            frame(vec![
                egui::Event::PointerMoved(pointer),
                egui::Event::PointerButton {
                    pos: pointer,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ])
            .drop_without_applying_deltas();
        }
        frame(vec![egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        }])
        .drop_without_applying_deltas();
        frame(vec![egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, 140.0),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers: egui::Modifiers::NONE,
        }])
        .drop_without_applying_deltas();
        let output = frame(vec![egui::Event::Copy]);
        assert_ne!(log_position(&output).0.y, original.y);
        assert!(
            output
                .platform_output
                .commands
                .iter()
                .any(|command| matches!(
                    command,
                    egui::OutputCommand::CopyText(text)
                        if text.starts_with("INFO  event 0\nINFO  event 1\n")
                            && text.ends_with("INFO  event 95")
                ))
        );
        output.drop_without_applying_deltas();
    }

    #[test]
    fn changing_language_updates_existing_problems() {
        let (mut host, _) = Host::new(orange_host::HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        editor.set_language("en");
        editor.open_untitled(&mut host, orange_host::new_document().unwrap());
        editor.add_node(&mut host, intrinsic::SET_VARIABLE).unwrap();
        assert!(
            editor
                .problems()
                .iter()
                .any(|text| text.starts_with("Variable reference"))
        );

        editor.set_language("es");
        egui::Context::default()
            .run_ui(egui::RawInput::default(), |ui| editor.ui(ui, &mut host))
            .drop_without_applying_deltas();
        assert!(
            editor
                .problems()
                .iter()
                .any(|text| text.starts_with("Falta la referencia a la variable"))
        );
    }

    #[test]
    fn problems_can_be_multi_selected_and_bulk_copy_respects_filters() {
        let (mut host, _) = Host::new(orange_host::HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        editor.open_untitled(&mut host, orange_host::new_document().unwrap());
        let node = editor.add_node(&mut host, intrinsic::WHOLE_NUMBER).unwrap();
        editor.canvas.select(Some(node));
        editor.panels.log = true;
        let document = editor.document.as_mut().unwrap();
        document.problems = ["Missing output", "Missing input", "Invalid value"]
            .map(String::from)
            .into();
        document.problem_locations = [
            (ProblemLevel::Warning, "W-OUTPUT"),
            (ProblemLevel::Blocker, "E-INPUT"),
            (ProblemLevel::Blocker, "E-VALUE"),
        ]
        .map(|(level, code)| ProblemLocation {
            node: Some(node),
            output: None,
            level,
            code,
        })
        .into();
        editor.problems_filter = Some(ProblemLevel::Blocker);
        editor.problems_search = "missing".to_owned();

        let context = egui::Context::default();
        icons::install(&context);
        let mut frame = |events| {
            context.run_ui(
                egui::RawInput {
                    events,
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 800.0),
                    )),
                    ..Default::default()
                },
                |ui| editor.ui(ui, &mut host),
            )
        };
        frame(vec![]).drop_without_applying_deltas();
        let output = frame(vec![]);
        let message = {
            let bounds = |label| {
                output.shapes.iter().find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == label => {
                        Some(text.galley.rect.translate(text.pos.to_vec2()))
                    }
                    _ => None,
                })
            };
            bounds("Missing input")
        };
        output.drop_without_applying_deltas();
        let click = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let message = message.unwrap();
        let pos = message.left_center() - egui::vec2(8.0, 0.0);
        frame(vec![egui::Event::PointerMoved(pos)]).drop_without_applying_deltas();
        frame(vec![click(pos, true)]).drop_without_applying_deltas();
        frame(vec![click(pos, false)]).drop_without_applying_deltas();
    }
}
