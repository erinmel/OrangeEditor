//! Structured editor diagnostics and the runtime log.
use super::*;
use crate::controls::{Action, Dropdown as _, Surface};
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

    fn count_label(self, count: usize) -> String {
        let key = match self {
            Self::Blocker => "problem-blockers",
            Self::Warning => "problem-warnings",
            Self::Draft => "problem-drafts",
        };
        tf(key, &[("count", count.into())])
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
    code: &'static str,
}

fn top_node(diagnostic: &orange_host::Diagnostic) -> Option<NodeId> {
    diagnostic
        .subject
        .node
        .filter(|_| diagnostic.subject.function.is_none())
}

/// Authoring errors expected while filling out a node. Graph structure, provider
/// integrity, declarations and edge errors keep their normal validation severity.
fn can_be_draft(diagnostic: &orange_host::Diagnostic) -> bool {
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
    fn draft_nodes(&self, diagnostics: &[orange_host::Diagnostic]) -> BTreeSet<NodeId> {
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
                    level: ProblemLevel::Blocker,
                    code: "",
                });
            }
        }
    }

    pub(super) fn record_problem(
        &mut self,
        diagnostic: &orange_host::Diagnostic,
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
        let text = describe(diagnostic, &self.names);
        if let Some(wire) = wire {
            canvas::Marks::add(&mut marks.wires, wire, error, text.clone());
        } else if let Some(node) = node {
            canvas::Marks::add(&mut marks.nodes, node, error, text.clone());
            if let Some(port) = &diagnostic.subject.port {
                canvas::Marks::add(
                    &mut marks.ports,
                    (node, port.to_string()),
                    error,
                    text.clone(),
                );
            }
        }
        self.problem_locations.push(ProblemLocation {
            node,
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
        egui::Frame::new().inner_margin(8).show(ui, |ui| {
            ui.set_min_height(ui.available_height());
            if self.log_events {
                self.log_content(ui);
            } else {
                self.problems_content(ui);
            }
        });
    }

    fn log_content(&mut self, ui: &mut egui::Ui) {
        search_field(ui, "log-search", &mut self.log_search, t("search-log"));
        ui.horizontal_wrapped(|ui| {
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
                egui::WidgetInfo::selected(egui::Role::CheckBox, true, self.live_trace, trace_label)
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
        });
        ui.separator();
        let query = self.log_search.trim().to_lowercase();
        egui::ScrollArea::vertical()
            .id_salt("editor-events")
            .stick_to_bottom(true)
            .show(ui, |ui| {
                let mut any = false;
                for line in &self.log {
                    if self.log_filter.is_some_and(|level| level != line.level)
                        || !contains_query(
                            &query,
                            &[&line.text, line.level.label(), line.level.name()],
                        )
                    {
                        continue;
                    }
                    any = true;
                    ui.horizontal_top(|ui| {
                        ui.add_sized(
                            [74.0, 18.0],
                            egui::Label::new(
                                egui::RichText::new(line.level.label())
                                    .monospace()
                                    .color(line.level.color(ui)),
                            ),
                        );
                        let shown = if line.repeated > 1 {
                            tf(
                                "log-repeated",
                                &[
                                    ("line", line.text.as_str().into()),
                                    ("count", line.repeated.into()),
                                ],
                            )
                        } else {
                            line.text.clone()
                        };
                        ui.add(
                            egui::Label::new(egui::RichText::new(shown).monospace())
                                .wrap()
                                .selectable(true),
                        );
                    });
                }
                if !any {
                    ui.weak(t(if self.log.is_empty() {
                        "no-log-messages"
                    } else {
                        "no-log-results"
                    }));
                }
            });
    }

    fn problems_content(&mut self, ui: &mut egui::Ui) {
        let Some(document) = &self.document else {
            ui.weak(t("no-problems"));
            return;
        };
        search_field(
            ui,
            "problems-search",
            &mut self.problems_search,
            t("search-problems"),
        );
        let levels = std::iter::once(None).chain(ProblemLevel::ALL.map(Some));
        ui.horizontal_wrapped(|ui| {
            for level in levels {
                let (button, label) = match level {
                    None => (
                        egui::Button::new(t("filter-all")),
                        t("filter-all").to_owned(),
                    ),
                    Some(level) => {
                        let count = document
                            .problem_locations
                            .iter()
                            .filter(|item| item.level == level)
                            .count();
                        let label = level.count_label(count);
                        (egui::Button::new((level.icon(ui), label.clone())), label)
                    }
                };
                let selected = self.problems_filter == level;
                let response =
                    controls::button(ui, Surface::Panel, Action::Toggle(selected), true, button);
                response.widget_info(|| {
                    egui::WidgetInfo::selected(egui::Role::RadioButton, true, selected, &label)
                });
                if let Some(level) = level {
                    response.clone().on_hover_text(level.hint());
                }
                if response.clicked() {
                    self.problems_filter = level;
                }
            }
        });
        ui.separator();
        let query = self.problems_search.trim().to_lowercase();
        let mut focus = None;
        egui::ScrollArea::vertical()
            .id_salt("editor-problems")
            .show(ui, |ui| {
                let mut any = false;
                for level in ProblemLevel::ALL {
                    if self.problems_filter.is_some_and(|wanted| level != wanted) {
                        continue;
                    }
                    for (text, location) in
                        document.problems.iter().zip(&document.problem_locations)
                    {
                        if location.level != level
                            || !contains_query(&query, &[text, level.label(), location.code])
                        {
                            continue;
                        }
                        any = true;
                        ui.horizontal_top(|ui| {
                            ui.label(level.icon(ui)).on_hover_text(level.hint());
                            if let Some(node) = location.node {
                                if ui
                                    .add(egui::Button::new(text).wrap().frame(false))
                                    .on_hover_text(t("show-node"))
                                    .clicked()
                                {
                                    focus = Some(node);
                                }
                            } else {
                                ui.add(egui::Label::new(text).wrap());
                            }
                        });
                    }
                }
                if !any {
                    ui.weak(t(if document.problems.is_empty() {
                        "no-problems"
                    } else {
                        "no-problem-results"
                    }));
                }
            });
        if let Some(node) = focus {
            self.canvas.focus(&document.graph, node);
        }
    }
}
