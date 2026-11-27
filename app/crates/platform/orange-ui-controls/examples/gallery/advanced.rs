use std::time::Duration;

use egui::{Button, Response, Ui};
use orange_ui_controls::{
    Action, Divider, Dropdown, Surface, UiTokens, button, components,
    navigation::{self, CalendarDate, MultiSelect},
    patterns::{self, CheckState, OptionRow, OptionStyle, Tone},
};

#[derive(Clone, Copy)]
pub(super) enum Glyph {
    Home,
    Wave,
    Files,
    Layers,
    Search,
    Settings,
    Menu,
    More,
    Sun,
    Moon,
    Copy,
    Edit,
    Play,
    Pause,
    Close,
    Trash,
    Browse,
}

impl Glyph {
    #[cfg(feature = "icons")]
    fn icon(self) -> orange_ui_controls::icons::UiIcon {
        use orange_ui_controls::icons::{Lucide, UiIcon, phosphor};
        match self {
            Self::Home => UiIcon::Lucide(Lucide::House),
            Self::Wave => UiIcon::Phosphor(phosphor::WAVEFORM),
            Self::Files => UiIcon::Lucide(Lucide::Folder),
            Self::Layers => UiIcon::Lucide(Lucide::Layers),
            Self::Search => UiIcon::Material(orange_ui_controls::icons::SEARCH),
            Self::Settings => UiIcon::Phosphor(phosphor::GEAR),
            Self::More => UiIcon::Material('\u{e5d3}'),
            Self::Menu => UiIcon::Lucide(Lucide::Menu),
            Self::Sun => UiIcon::Lucide(Lucide::Sun),
            Self::Moon => UiIcon::Lucide(Lucide::Moon),
            Self::Copy => UiIcon::Lucide(Lucide::Copy),
            Self::Edit => UiIcon::Material(orange_ui_controls::icons::EDIT),
            Self::Play => UiIcon::Lucide(Lucide::Play),
            Self::Pause => UiIcon::Lucide(Lucide::Pause),
            Self::Close => UiIcon::Lucide(Lucide::X),
            Self::Trash => UiIcon::Lucide(Lucide::Trash),
            Self::Browse => UiIcon::Lucide(Lucide::FolderOpen),
        }
    }

    pub(super) fn atom(self) -> egui::Atom<'static> {
        #[cfg(feature = "icons")]
        {
            self.icon().atom()
        }
        #[cfg(not(feature = "icons"))]
        {
            egui::Atom::paint(egui::Vec2::splat(16.0), move |ui, args| {
                self.draw(ui, args.rect, args.fallback_text_color)
            })
        }
    }
    pub(super) fn draw(self, ui: &Ui, rect: egui::Rect, ink: egui::Color32) {
        #[cfg(feature = "icons")]
        orange_ui_controls::icons::draw_centered_icon(ui, rect, self.icon(), ink);
        #[cfg(not(feature = "icons"))]
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            match self {
                Self::Home => "H",
                Self::Wave => "~",
                Self::Files => "F",
                Self::Layers => "=",
                Self::Search => "?",
                Self::Settings => "S",
                Self::More => "...",
                Self::Menu => "=",
                Self::Sun => "+",
                Self::Moon => "-",
                Self::Copy => "C",
                Self::Edit => "E",
                Self::Play => ">",
                Self::Pause => "II",
                Self::Close => "x",
                Self::Trash => "D",
                Self::Browse => "F",
            },
            egui::FontId::proportional(14.0),
            ink,
        );
    }
}

pub(super) fn icon(ui: &mut Ui, glyph: Glyph) -> Response {
    let (rect, response) = ui.allocate_exact_size(egui::Vec2::splat(18.0), egui::Sense::hover());
    glyph.draw(ui, rect, ui.visuals().text_color());
    response
}

pub(super) fn icon_button(ui: &mut Ui, glyph: Glyph, label: &str) -> Response {
    orange_ui_controls::icon_button(
        ui,
        ui.spacing().interact_size.y + 4.0,
        None,
        None,
        |ui, rect, ink| glyph.draw(ui, rect, ink),
    )
    .accessible_name(label)
    .on_hover_text(label)
}

pub(super) fn search_field(
    ui: &mut Ui,
    id: &str,
    value: &mut String,
    hint: &str,
    width: f32,
) -> Response {
    orange_ui_controls::search_sized(
        ui,
        id,
        value,
        hint,
        Glyph::Search.atom(),
        Glyph::Close.atom(),
        "Clear search",
        width,
    )
}
const SECTIONS: [(&str, Glyph); 5] = [
    ("Appearance", Glyph::Home),
    ("Controls", Glyph::Layers),
    ("Audio", Glyph::Wave),
    ("Files", Glyph::Files),
    ("Patterns", Glyph::Settings),
];

#[derive(Default)]
pub(super) struct Shell {
    active: usize,
    sidebar: bool,
    groups_collapsed: [bool; 2],
    destination: Option<usize>,
    search: bool,
    query: String,
}

impl Shell {
    pub(super) fn jump_to_files(&mut self) {
        self.destination = Some(3);
    }

    pub(super) fn header(
        &mut self,
        ui: &mut Ui,
        tokens: UiTokens,
        audio: &mut super::audio::Preview,
    ) {
        ui.horizontal_wrapped(|ui| {
            if ui.available_width() >= 540.0
                && icon_button(ui, Glyph::Menu, "Toggle section sidebar").clicked()
            {
                self.sidebar = !self.sidebar;
            }
            icon(ui, Glyph::Wave);
            ui.heading("Orange UI");
            if icon_button(ui, Glyph::Search, "Search components (Ctrl+K)").clicked() {
                self.search = !self.search;
            }
            ui.menu_button((Glyph::Settings.atom(), "Settings"), |ui| {
                if ui.button("Appearance and density").clicked() {
                    self.destination = Some(0);
                    ui.close();
                }
            });
            if ui
                .button(if audio.playing() {
                    "Pause transport"
                } else {
                    "Start transport"
                })
                .clicked()
            {
                audio.toggle_playback();
            }
            patterns::badge(
                ui,
                tokens,
                if audio.playing() {
                    Tone::Success
                } else {
                    Tone::Info
                },
                if audio.playing() { "Running" } else { "Ready" },
            );
        });
        if let Some(index) = navigation::breadcrumbs(ui, &["Projects", "Audio", "Session 01"]) {
            self.destination = Some([0, 2][index]);
        }
        if ui.available_width() < 500.0 {
            ui.menu_button((Glyph::Menu.atom(), SECTIONS[self.active].0), |ui| {
                for (index, &(label, glyph)) in SECTIONS.iter().enumerate() {
                    if ui
                        .add(
                            Button::new((glyph.atom(), label))
                                .frame(false)
                                .selected(index == self.active),
                        )
                        .clicked()
                    {
                        self.destination = Some(index);
                        ui.close();
                    }
                }
            });
        } else if !self.sidebar {
            ui.horizontal_wrapped(|ui| {
                for (index, &(label, glyph)) in SECTIONS.iter().enumerate() {
                    let atoms = (glyph.atom(), label);
                    if button(
                        ui,
                        Surface::Panel,
                        Action::Toggle(self.active == index),
                        true,
                        Button::new(atoms),
                    )
                    .clicked()
                    {
                        self.destination = Some(index);
                    }
                }
            });
        }
        let key = egui::KeyboardShortcut::new(egui::Modifiers::CTRL, egui::Key::K);
        let shortcut = ui.input_mut(|input| input.consume_shortcut(&key));
        self.search |= shortcut;
        if self.search {
            let field = search_field(
                ui,
                "global-search",
                &mut self.query,
                "Find a gallery section",
                f32::INFINITY,
            );
            if shortcut {
                field.request_focus();
            }
            ui.horizontal_wrapped(|ui| {
                let query = self.query.to_lowercase();
                for (i, &(label, _)) in SECTIONS
                    .iter()
                    .enumerate()
                    .filter(|(_, (label, _))| label.to_lowercase().contains(&query))
                {
                    if ui.button(label).clicked() {
                        self.destination = Some(i);
                        self.search = false;
                    }
                }
            });
        }
    }

    pub(super) fn sidebar(&mut self, ui: &mut Ui, tokens: UiTokens) {
        if self.sidebar && ui.available_width() >= 540.0 {
            egui::Panel::left("section-sidebar")
                .resizable(false)
                .default_size(176.0)
                .frame(
                    egui::Frame::new()
                        .fill(tokens.theme.bg_surface())
                        .inner_margin(8),
                )
                .show(ui, |ui| {
                    ui.set_min_width(152.0);
                    ui.spacing_mut().indent = 12.0;
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for (group, (name, group_icon, indices)) in [
                        ("Gallery", Glyph::Layers, &[0usize, 1, 4][..]),
                        ("Media", Glyph::Wave, &[2usize, 3][..]),
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        if navigation::sidebar_row(
                            ui,
                            name,
                            false,
                            Some(!self.groups_collapsed[group]),
                            |ui, r, c| group_icon.draw(ui, r, c),
                        )
                        .clicked()
                        {
                            self.groups_collapsed[group] = !self.groups_collapsed[group];
                        }
                        if !self.groups_collapsed[group] {
                            ui.indent(("destinations", group), |ui| {
                                for &index in indices {
                                    let (label, glyph) = SECTIONS[index];
                                    if navigation::sidebar_row(
                                        ui,
                                        label,
                                        self.active == index,
                                        None,
                                        |ui, r, c| glyph.draw(ui, r, c),
                                    )
                                    .clicked()
                                    {
                                        self.destination = Some(index);
                                    }
                                }
                            });
                        }
                    }
                });
        }
    }
    pub(super) fn section(&mut self, ui: &mut Ui, index: usize, label: &str) {
        let response = ui.heading(label);
        if self.destination == Some(index) {
            response.scroll_to_me(Some(egui::Align::Min));
            self.destination = None;
            self.active = index;
        }
    }

    pub(super) fn footer(&mut self, ui: &mut Ui, tokens: UiTokens) {
        if ui.available_width() < 500.0 {
            ui.horizontal_wrapped(|ui| {
                for index in [0, 2, 3] {
                    let (label, glyph) = SECTIONS[index];
                    if ui
                        .add(Button::new((glyph.atom(), label)).selected(self.active == index))
                        .clicked()
                    {
                        self.destination = Some(index);
                    }
                }
            });
            return;
        }
        ui.horizontal_wrapped(|ui| {
            patterns::badge(ui, tokens, Tone::Success, "Synced");
            ui.monospace("48 kHz / 256 spl");
            ui.weak("CPU demo");
            ui.allocate_ui(egui::vec2(62.0, 0.0), |ui| {
                ui.set_width(62.0);
                patterns::progress(ui, tokens, Some(0.23), "CPU demo load 23%");
            });
            for (label, delta) in [("−", -0.1), ("+", 0.1)] {
                if ui
                    .button(label)
                    .accessible_name(if delta > 0.0 { "Zoom in" } else { "Zoom out" })
                    .clicked()
                {
                    ui.ctx()
                        .set_zoom_factor((ui.ctx().zoom_factor() + delta).clamp(0.7, 1.7));
                }
            }
        });
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Filter {
    All,
    Wav,
    Presets,
}

#[derive(Clone, Copy)]
enum Dialog {
    Confirm,
    Form,
    Settings,
}

struct Toast {
    id: u64,
    label: &'static str,
    tone: Tone,
    started: f64,
    lifetime: patterns::ToastLifetime,
}

pub(super) struct Catalog {
    radio: [usize; 3],
    checks: [bool; 3],
    mixed: CheckState,
    switches: [bool; 4],
    page: usize,
    sort: Option<(usize, bool)>,
    order: [usize; 4],
    step: usize,
    dropdown: MultiSelect,
    combo: usize,
    combo_query: String,
    query: String,
    expanded: bool,
    filter: Filter,
    modal: Option<Dialog>,
    tab: usize,
    draft: String,
    project: String,
    notes: String,
    bold: bool,
    checklist: bool,
    date: CalendarDate,
    time: [u8; 3],
    bars: [u16; 2],
    timestamp: Option<f32>,
    toasts: Vec<Toast>,
    next_toast: u64,
    deleted: bool,
}

impl Default for Catalog {
    fn default() -> Self {
        Self {
            radio: [0; 3],
            checks: [true, false, true],
            mixed: CheckState::Mixed,
            switches: [true, false, true, false],
            page: 0,
            sort: None,
            order: [0, 1, 2, 3],
            step: 1,
            dropdown: MultiSelect::default(),
            combo: 0,
            combo_query: String::new(),
            query: String::new(),
            expanded: false,
            filter: Filter::All,
            modal: None,
            tab: 0,
            draft: String::new(),
            project: "Session 01".into(),
            notes: "Check room tone before the first take.".into(),
            bold: false,
            checklist: false,
            date: CalendarDate::new(2026, 11, 22),
            time: [0, 0, 42],
            bars: [1, 1],
            timestamp: None,
            toasts: Vec::new(),
            next_toast: 0,
            deleted: false,
        }
    }
}

impl Catalog {
    pub(super) fn take_timestamp(&mut self) -> Option<f32> {
        self.timestamp.take()
    }

    pub(super) fn selections(&mut self, ui: &mut Ui, tokens: UiTokens) {
        ui.heading("Selection controls");
        for (index, (title, style, rows)) in [
            (
                "Inline radio",
                OptionStyle::Inline,
                [("Stereo", ""), ("Mono", "")],
            ),
            (
                "Description radio",
                OptionStyle::Description,
                [
                    ("Local", "Use the recordings in this project."),
                    ("Remote", "Use shared session files."),
                ],
            ),
            (
                "Selection cards",
                OptionStyle::Card,
                [
                    ("Studio", "Low-latency monitoring"),
                    ("Field", "Portable recording"),
                ],
            ),
        ]
        .into_iter()
        .enumerate()
        {
            ui.strong(title);
            for (choice, (label, description)) in rows.into_iter().enumerate() {
                let option = OptionRow {
                    title: label,
                    description: (!description.is_empty()).then_some(description),
                    shortcut: None,
                    style,
                    exclusive: true,
                };
                if option
                    .show(ui, tokens, self.radio[index] == choice, |ui| {
                        icon(ui, [Glyph::Wave, Glyph::Files][choice]);
                    })
                    .clicked()
                {
                    self.radio[index] = choice;
                }
            }
        }
        Divider::Section.show(ui, tokens);
        ui.heading("Checkboxes and switches");
        ui.checkbox(&mut self.checks[0], "Standard checkbox");
        patterns::tri_checkbox(ui, tokens, "Mixed selection", &mut self.mixed);
        for (index, style, title, description, shortcut) in [
            (1, OptionStyle::Chip, "Tags", None, None),
            (
                2,
                OptionStyle::Card,
                "Monitor input",
                Some("Send the input to the monitor bus."),
                Some("Ctrl+M"),
            ),
        ] {
            if (OptionRow {
                title,
                description,
                shortcut,
                style,
                exclusive: false,
            })
            .show(ui, tokens, self.checks[index], |ui| {
                icon(ui, Glyph::Wave);
            })
            .clicked()
            {
                self.checks[index] = !self.checks[index];
            }
        }
        components::switch(ui, tokens, &mut self.switches[0], "Default switch");
        tokens.scope(
            ui,
            Some(orange_ui_controls::Density::Comfy),
            |ui, tokens| {
                components::switch(ui, tokens, &mut self.switches[1], "Comfy labeled switch");
            },
        );
        components::switch_with_thumb(
            ui,
            tokens,
            &mut self.switches[2],
            "Day / night",
            |ui, rect, ink, on| {
                if on { Glyph::Sun } else { Glyph::Moon }.draw(ui, rect, ink);
            },
        );
        components::card(ui, tokens, |ui| {
            ui.strong("Audio monitoring");
            components::switch(ui, tokens, &mut self.switches[3], "Follow session output");
        });
    }

    pub(super) fn show(&mut self, ui: &mut Ui, tokens: UiTokens) {
        super::card_grid(ui, 6, 295.0, 2, |ui, index| {
            components::card(ui, tokens, |ui| {
                ui.set_min_width(ui.available_width());
                match index {
                    0 => self.menus(ui, tokens),
                    1 => self.dialogs(ui, tokens),
                    2 => self.pickers(ui, tokens),
                    3 => self.feedback(ui, tokens),
                    4 => self.track_table(ui, false),
                    _ => self.track_table(ui, true),
                }
            });
        });
        self.show_modal(ui, tokens);
    }

    fn track_table(&mut self, ui: &mut Ui, sortable: bool) {
        const TRACKS: [(&str, u16); 4] =
            [("Room", 42), ("Brass", 106), ("Bass", 72), ("Voice", 61)];
        ui.heading(if sortable {
            "Sortable track table"
        } else {
            "Session track order"
        });
        egui::Grid::new(("gallery-track-table", sortable))
            .striped(true)
            .min_col_width((ui.available_width() * 0.46).max(40.0))
            .show(ui, |ui| {
                for (column, name) in ["Track", "Duration"].into_iter().enumerate() {
                    if sortable {
                        let active = self.sort.filter(|(index, _)| *index == column);
                        let button = if let Some((_, descending)) = active {
                            let arrow =
                                egui::Atom::paint(egui::Vec2::splat(12.0), move |ui, args| {
                                    let center = args.rect.center();
                                    let dy = if descending { 2.0 } else { -2.0 };
                                    let tip = center + egui::vec2(0.0, dy);
                                    let stroke = egui::Stroke::new(1.6, args.fallback_text_color);
                                    for dx in [-4.0, 4.0] {
                                        ui.painter().line_segment(
                                            [center + egui::vec2(dx, -dy), tip],
                                            stroke,
                                        );
                                    }
                                });
                            Button::new((name, arrow))
                        } else {
                            Button::new(name)
                        };
                        if ui
                            .add(button.frame(false))
                            .on_hover_text("Sort by this column")
                            .clicked()
                        {
                            let descending = self.sort == Some((column, false));
                            self.sort = Some((column, descending));
                            self.order.sort_by(|&a, &b| {
                                let result = if column == 0 {
                                    TRACKS[a].0.cmp(TRACKS[b].0)
                                } else {
                                    TRACKS[a].1.cmp(&TRACKS[b].1)
                                };
                                if descending { result.reverse() } else { result }
                            });
                        }
                    } else {
                        ui.strong(name);
                    }
                }
                ui.end_row();
                for index in if sortable { self.order } else { [0, 1, 2, 3] } {
                    let (name, seconds) = TRACKS[index];
                    ui.label(name);
                    ui.monospace(format!("{}:{:02}", seconds / 60, seconds % 60));
                    ui.end_row();
                }
            });
    }
    fn menus(&mut self, ui: &mut Ui, tokens: UiTokens) {
        ui.heading("Navigation, menus and search");
        navigation::pagination(ui, &mut self.page, 5, "Previous", "Next");
        navigation::stepper(ui, tokens, &["Source", "Edit", "Publish"], &mut self.step);
        let overflow = icon_button(ui, Glyph::More, "More actions");
        let context = ui.button("Right-click for context menu");
        let _ = egui::Popup::menu(&overflow)
            .frame(egui::Frame::popup(ui.style()))
            .show(|ui| self.menu_rows(ui, tokens));
        let _ = egui::Popup::context_menu(&context)
            .frame(egui::Frame::popup(ui.style()))
            .show(|ui| self.menu_rows(ui, tokens));

        let options = ["WAV", "Presets", "Projects", "Sessions"];
        let format = ui.label("File format");
        egui::ComboBox::from_id_salt("format")
            .width(ui.available_width())
            .selected_text(options[self.combo])
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show_select(ui, Surface::Card, |ui| {
                search_field(
                    ui,
                    "format-search",
                    &mut self.combo_query,
                    "Filter formats",
                    f32::INFINITY,
                );
                let query = self.combo_query.to_lowercase();
                for (index, &option) in options
                    .iter()
                    .enumerate()
                    .filter(|(_, option)| option.to_lowercase().contains(&query))
                {
                    if ui.selectable_label(self.combo == index, option).clicked() {
                        self.combo = index;
                        ui.close();
                    }
                }
            })
            .response
            .labelled_by(format.id);
        self.dropdown
            .show(ui, "Select multiple formats", "Filter formats", &options);
        let search = icon_button(ui, Glyph::Search, "Expand search");
        if search.clicked() {
            self.expanded = !self.expanded;
            ui.ctx()
                .data_mut(|data| data.remove::<bool>(search.id.with("opened")));
        }
        let width = ui.ctx().animate_bool(search.id, self.expanded) * ui.available_width();
        if width > 38.0 {
            let field = search_field(ui, "asset-search", &mut self.query, "Search assets", width);
            if self.expanded
                && !ui
                    .ctx()
                    .data(|data| data.get_temp::<bool>(search.id.with("opened")))
                    .unwrap_or(false)
            {
                field.request_focus();
                ui.ctx()
                    .data_mut(|data| data.insert_temp(search.id.with("opened"), true));
            }
            let label = ui.weak("Filter");
            orange_ui_controls::pill_segmented(
                ui,
                tokens,
                &label,
                &mut self.filter,
                &[
                    (Filter::All, "All"),
                    (Filter::Wav, "WAV"),
                    (Filter::Presets, "Presets"),
                ],
            );
            for (name, category) in [
                ("rehearsal.wav", Filter::Wav),
                ("Studio Preset", Filter::Presets),
            ] {
                if (self.filter == Filter::All || self.filter == category)
                    && name.to_lowercase().contains(&self.query.to_lowercase())
                {
                    ui.label(name);
                }
            }
        }
    }

    fn menu_rows(&mut self, ui: &mut Ui, tokens: UiTokens) {
        ui.set_width(205.0);
        if components::menu_row_with_icon(
            ui,
            tokens,
            "Copy project name",
            Some("Ctrl+C"),
            false,
            |ui, rect, ink| Glyph::Copy.draw(ui, rect, ink),
        )
        .clicked()
        {
            ui.ctx().copy_text(self.project.clone());
            ui.close();
        }
        if components::menu_row_with_icon(
            ui,
            tokens,
            "Rename",
            Some("F2"),
            false,
            |ui, rect, ink| Glyph::Edit.draw(ui, rect, ink),
        )
        .clicked()
        {
            self.draft.clone_from(&self.project);
            self.modal = Some(Dialog::Form);
            ui.close();
        }
        components::menu_separator(ui, tokens);
        let submenu = components::menu_row(ui, tokens, "Export", None, None, true);
        egui::Popup::menu(&submenu).show(|ui| {
            ui.set_width(160.0);
            if components::menu_row(ui, tokens, "Copy metadata", None, None, false).clicked() {
                ui.ctx().copy_text(format!("{} · 48 kHz", self.project));
                ui.close();
            }
        });
    }

    fn dialogs(&mut self, ui: &mut Ui, tokens: UiTokens) {
        ui.heading("Cards, dialogs and popovers");
        let deleted = self.deleted;
        if patterns::structured_card(
            ui,
            tokens,
            |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.strong(&self.project);
                    patterns::badge(ui, tokens, Tone::Success, "Ready");
                    ui.menu_button("…", |ui| {
                        if ui.button("Rename").clicked() {
                            self.draft.clone_from(&self.project);
                            self.modal = Some(Dialog::Form);
                            ui.close();
                        }
                    });
                });
                ui.weak("Session workspace");
            },
            |ui| {
                ui.label(if deleted {
                    "The demo take was cleared."
                } else {
                    "The rehearsal take is ready to review."
                });
            },
            |ui| ui.button("Clear take").clicked(),
        )
        .inner
        {
            self.modal = Some(Dialog::Confirm);
        }
        ui.horizontal_wrapped(|ui| {
            if ui.button("Form").clicked() {
                self.draft.clone_from(&self.project);
                self.modal = Some(Dialog::Form);
            }
            if ui.button("Settings tabs").clicked() {
                self.modal = Some(Dialog::Settings);
            }
            ui.menu_button("Popover", |ui| {
                ui.strong("Session defaults");
                ui.label("48 kHz · 256 samples");
            });
            ui.button("Tooltip").on_hover_ui(|ui| {
                ui.strong("Global Search");
                ui.label("Jump to a component section.");
                ui.monospace("Ctrl+K");
            });
        });
        egui::CollapsingHeader::new("Project notebook")
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.toggle_value(&mut self.bold, "Bold");
                    if ui.toggle_value(&mut self.checklist, "Checklist").changed() && self.checklist
                    {
                        self.notes.insert_str(0, "☐ ");
                    }
                    if ui.button("@00:42").clicked() {
                        self.timestamp = Some(42.0);
                    }
                });
                let font = if self.bold {
                    egui::TextStyle::Heading
                } else {
                    egui::TextStyle::Body
                };
                ui.add(
                    egui::TextEdit::multiline(&mut self.notes)
                        .font(font)
                        .margin(egui::Margin::symmetric(8, 6))
                        .desired_rows(2)
                        .desired_width(ui.available_width()),
                );
            });
    }

    fn show_modal(&mut self, ui: &mut Ui, tokens: UiTokens) {
        let Some(kind) = self.modal else { return };
        let title = match kind {
            Dialog::Confirm => "Clear take?",
            Dialog::Form => "Rename project",
            Dialog::Settings => "Session settings",
        };
        let response = patterns::dialog_with_close(
            ui.ctx(),
            ui.scope_id().with("patterns-dialog"),
            tokens,
            title,
            (!matches!(kind, Dialog::Confirm)).then_some("Close dialog"),
            |ui| {
                match kind {
                    Dialog::Confirm => {
                        ui.label("Clear the rehearsal take from this demo session.");
                    }
                    Dialog::Form => {
                        ui.label("Project name");
                        ui.text_edit_singleline(&mut self.draft);
                    }
                    Dialog::Settings => {
                        orange_ui_controls::notebook(
                            ui,
                            Surface::Dialog,
                            orange_ui_controls::NotebookOrientation::Horizontal,
                            &mut self.tab,
                            &[(0, "General"), (1, "Audio"), (2, "Files")],
                        );
                        ui.label(
                            [
                                "Session settings apply to this preview.",
                                "Sample rate 48 kHz · 256 samples",
                                "Project folder: Session 01",
                            ][self.tab],
                        );
                    }
                }
                !self.draft.trim().is_empty()
            },
            |ui, valid| {
                let label = match kind {
                    Dialog::Confirm => "Clear",
                    Dialog::Form => "Save",
                    Dialog::Settings => "Done",
                };
                ui.horizontal_wrapped(|ui| {
                    let action = if matches!(kind, Dialog::Confirm) {
                        Action::Destructive
                    } else {
                        Action::Primary
                    };
                    let accept = button(
                        ui,
                        Surface::Dialog,
                        action,
                        !matches!(kind, Dialog::Form) || *valid,
                        Button::new(label),
                    )
                    .clicked();
                    let cancel = ui.button("Cancel").clicked();
                    (accept, cancel)
                })
                .inner
            },
        );
        let ((accept, cancel), closed) = response.inner;
        if accept {
            match kind {
                Dialog::Confirm => {
                    self.deleted = true;
                    self.push_toast(ui, Tone::Warning, "Take cleared");
                }
                Dialog::Form => self.project = self.draft.trim().to_owned(),
                Dialog::Settings => {}
            }
        }
        if accept || cancel || closed || response.should_close() {
            self.modal = None;
        }
    }

    fn pickers(&mut self, ui: &mut Ui, tokens: UiTokens) {
        ui.heading("Date and timecode");
        let date_label = ui.label("Session date");
        self.date
            .picker_with_months(
                ui,
                ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"],
                Some([
                    "January",
                    "February",
                    "March",
                    "April",
                    "May",
                    "June",
                    "July",
                    "August",
                    "September",
                    "October",
                    "November",
                    "December",
                ]),
                "Previous month",
                "Next month",
            )
            .labelled_by(date_label.id);
        ui.add_space(tokens.density.spacing().md);
        ui.label("Time · HH:MM:SS");
        navigation::clock_wheels(ui, tokens, &mut self.time, ["Hours", "Minutes", "Seconds"]);
        ui.add_space(tokens.density.spacing().md);
        ui.label("Musical position · four beats per bar");
        ui.weak("Choose the bar, then the beat within it.");
        for (i, (label, max)) in [("Bar", 9999), ("Beat", 4)].into_iter().enumerate() {
            ui.horizontal(|ui| {
                let name = ui.label(label);
                if ui
                    .add_enabled(self.bars[i] > 1, Button::new("−"))
                    .accessible_name(format!("Previous {label}"))
                    .clicked()
                {
                    self.bars[i] -= 1;
                }
                ui.add(egui::DragValue::new(&mut self.bars[i]).range(1..=max))
                    .labelled_by(name.id);
                if ui
                    .add_enabled(self.bars[i] < max, Button::new("+"))
                    .accessible_name(format!("Next {label}"))
                    .clicked()
                {
                    self.bars[i] += 1;
                }
            });
        }
    }

    fn push_toast(&mut self, ui: &Ui, tone: Tone, label: &'static str) {
        let now = ui.input(|input| input.time);
        if self.toasts.len() >= 4 {
            if let Some(index) = self
                .toasts
                .iter()
                .position(|t| t.lifetime != patterns::ToastLifetime::PERSISTENT)
            {
                self.toasts.remove(index);
            } else {
                return;
            }
        }
        self.toasts.push(Toast {
            id: self.next_toast,
            label,
            tone,
            started: now,
            lifetime: if tone == Tone::Danger {
                patterns::ToastLifetime::PERSISTENT
            } else {
                patterns::ToastLifetime::timed(Duration::from_secs(7))
            },
        });
        self.next_toast = self.next_toast.wrapping_add(1);
    }

    fn feedback(&mut self, ui: &mut Ui, tokens: UiTokens) {
        ui.heading("Feedback and loading");
        ui.horizontal_wrapped(|ui| {
            for (tone, label, message) in [
                (Tone::Info, "Info", "Session opened"),
                (Tone::Success, "Success", "Project saved"),
                (Tone::Warning, "Warning", "Take cleared"),
                (Tone::Danger, "Error", "Export unavailable"),
            ] {
                patterns::badge(ui, tokens, tone, label);
                if ui.button(format!("Toast {label}")).clicked() {
                    if tone == Tone::Warning {
                        self.deleted = true;
                    }
                    self.push_toast(ui, tone, message);
                }
            }
        });
        ui.label("Uploading · 64%");
        patterns::progress(ui, tokens, Some(0.64), "Uploading 64 percent");
        ui.label("Waiting for sync");
        patterns::progress(ui, tokens, None, "Waiting for sync");
        patterns::skeleton(ui, tokens, &[0.92, 0.62, 0.78]);
        patterns::empty_state(
            ui,
            tokens,
            "No recent exports",
            "Exports appear here after rendering.",
        );
    }

    pub(super) fn toasts(&mut self, ui: &Ui, tokens: UiTokens) {
        let now = ui.input(|input| input.time);
        let remaining = |toast: &Toast| {
            toast
                .lifetime
                .remaining(Duration::from_secs_f64((now - toast.started).max(0.0)))
        };
        self.toasts.retain(|toast| remaining(toast) != Some(0.0));
        if self.toasts.is_empty() {
            return;
        }
        egui::Area::new(egui::Id::unique("gallery-toasts"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-18.0, -54.0))
            .show(ui.ctx(), |ui| {
                tokens.scope(ui, None, |ui, tokens| {
                    ui.set_width((ui.ctx().content_rect().width() - 40.0).clamp(125.0, 275.0));
                    self.toasts.retain(|toast| {
                        let action = ui
                            .push_id(toast.id, |ui| {
                                patterns::toast_with_body(
                                    ui,
                                    tokens,
                                    patterns::Toast {
                                        tone: toast.tone,
                                        message: toast.label,
                                        remaining: remaining(toast).map(|value| (value,"Time remaining")),
                                        dismiss_label: "Dismiss",
                                        undo_label: (toast.tone == Tone::Warning).then_some("Undo"),
                                    },
                                    match toast.tone {Tone::Danger => Some("Choose an available output folder and retry the export."), Tone::Warning => Some("Undo restores the cleared take."), _ => None},
                                )
                            })
                            .inner;
                        if action == patterns::ToastAction::Undo {
                            self.deleted = false;
                        }
                        action == patterns::ToastAction::None
                    });
                });
            });
        if self
            .toasts
            .iter()
            .any(|t| t.lifetime != patterns::ToastLifetime::PERSISTENT)
        {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
    }
}
