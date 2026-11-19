//! Structured editor diagnostics and the runtime log.
use super::*;

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

#[derive(Debug)]
pub(super) struct ProblemLocation {
    node: Option<NodeId>,
    error: bool,
}

pub(super) fn top_node(diagnostic: &orange_host::Diagnostic) -> Option<NodeId> {
    diagnostic
        .subject
        .node
        .filter(|_| diagnostic.subject.function.is_none())
}

pub(super) fn missing_splitter_stream(diagnostic: &orange_host::Diagnostic) -> bool {
    diagnostic.code.as_str() == "E-INPUT-REQUIRED"
        && diagnostic
            .subject
            .port
            .as_ref()
            .is_some_and(|port| port.port().as_str() == "stream")
}

impl Document {
    pub(super) fn set_problem_error(&mut self, error: &HostError) {
        self.run_blocked = true;
        self.problems = Editor::problems_of(error, &self.names);
        self.problem_locations = self
            .problems
            .iter()
            .map(|_| ProblemLocation {
                node: None,
                error: true,
            })
            .collect();
    }

    pub(super) fn record_problem(
        &mut self,
        diagnostic: &orange_host::Diagnostic,
        wire: Option<canvas::WireEnds>,
        advisory: bool,
        marks: &mut canvas::Marks,
    ) {
        let node = top_node(diagnostic);
        let error = diagnostic.severity == orange_host::Severity::Error && !advisory;
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
        self.problem_locations.push(ProblemLocation { node, error });
        self.problems.push(text);
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
        ui.set_min_height(ui.available_height());
        if self.log_events {
            self.log_content(ui);
        } else {
            self.problems_content(ui);
        }
    }

    fn log_content(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("log-level-filter")
                .selected_text(self.log_filter.map_or("ALL", NoteLevel::label))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.log_filter, None, "ALL");
                    for level in NoteLevel::ALL {
                        ui.selectable_value(&mut self.log_filter, Some(level), level.label());
                    }
                });
            ui.checkbox(&mut self.live_trace, t("live-trace"))
                .on_hover_text(t("live-trace-hint"));
            if ui
                .add_enabled(!self.log.is_empty(), egui::Button::new(t("clear-log")))
                .clicked()
            {
                self.log.clear();
                self.last_note = false;
            }
        });
        search_field(ui, "log-search", &mut self.log_search, t("search-log"));
        let query = self.log_search.trim().to_lowercase();
        egui::ScrollArea::vertical()
            .id_salt("editor-events")
            .stick_to_bottom(true)
            .show(ui, |ui| {
                let mut any = false;
                for line in &self.log {
                    if self.log_filter.is_some_and(|level| level != line.level)
                        || !contains_query(&query, &[&line.text, line.level.label()])
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
                        ui.add(egui::Label::new(egui::RichText::new(shown).monospace()).wrap());
                    });
                }
                if !any {
                    ui.weak(t("no-log-results"));
                }
            });
    }

    fn problems_content(&mut self, ui: &mut egui::Ui) {
        let Some(document) = &self.document else {
            ui.weak(t("no-problems"));
            return;
        };
        let errors = document
            .problem_locations
            .iter()
            .filter(|problem| problem.error)
            .count();
        let warnings = document.problems.len().saturating_sub(errors);
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut self.problems_filter, None, t("filter-all"));
            for (error, count, key, level, icon) in [
                (
                    true,
                    errors,
                    "problem-errors",
                    NoteLevel::Error,
                    icons::ERROR,
                ),
                (
                    false,
                    warnings,
                    "problem-warnings",
                    NoteLevel::Warning,
                    icons::WARNING,
                ),
            ] {
                ui.selectable_value(
                    &mut self.problems_filter,
                    Some(error),
                    (
                        icons::text(icon).color(level.color(ui)),
                        tf(key, &[("count", count.into())]),
                    ),
                );
            }
            ui.add(
                egui::TextEdit::singleline(&mut self.problems_search)
                    .id_salt("problems-search")
                    .hint_text(t("search-problems"))
                    .desired_width(ui.available_width().clamp(116.0, 340.0)),
            );
        });
        let query = self.problems_search.trim().to_lowercase();
        let mut focus = None;
        egui::ScrollArea::vertical()
            .id_salt("editor-problems")
            .show(ui, |ui| {
                let mut any = false;
                for (text, location) in document.problems.iter().zip(&document.problem_locations) {
                    let error = location.error;
                    if self.problems_filter.is_some_and(|wanted| error != wanted)
                        || !contains_query(&query, &[text])
                    {
                        continue;
                    }
                    any = true;
                    ui.horizontal_top(|ui| {
                        ui.label(
                            icons::text(if error { icons::ERROR } else { icons::WARNING }).color(
                                if error {
                                    NoteLevel::Error
                                } else {
                                    NoteLevel::Warning
                                }
                                .color(ui),
                            ),
                        );
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
