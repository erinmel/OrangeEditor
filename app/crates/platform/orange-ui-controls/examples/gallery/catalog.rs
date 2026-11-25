use std::num::NonZeroU16;

use egui::{Button, Id, Label, Response, RichText, Ui};
use orange_ui_controls::{
    Action, ButtonGeometry, ButtonSpec, ButtonStyle, ButtonTone, CheckStyle, KnobStyle,
    NotebookOrientation, RadioStyle, SliderBehavior, SliderGuide, SliderMarks, StepLabels, Surface,
    UiTokens, button, button_with, checkbox,
    components::{self, MenuItem, MenuTrigger, PasswordLabels, PathPicker},
    guided_slider, knob, logarithmic_slider,
    note::{Note, NoteState, NoteTone},
    notebook, pill_segmented, radio, rotary, stepped_slider,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum State {
    Normal,
    Disabled,
    Error,
}

#[derive(Clone, Copy)]
pub enum Widget {
    Buttons,
    Sliders,
    Choices,
    Text,
    Color,
    Card,
    Note(NoteTone),
    Pill,
    Switch,
    Password,
    File,
    Files,
    Folder,
    Menu(MenuTrigger),
    Modal,
}

#[derive(Clone, Copy)]
pub struct Entry {
    pub id: &'static str,
    pub title: &'static str,
    pub widget: Widget,
}

impl Entry {
    pub fn alternate(self) -> Option<(&'static str, State)> {
        match self.widget {
            Widget::Password | Widget::File | Widget::Files | Widget::Folder => {
                Some(("Validation error", State::Error))
            }
            Widget::Pill => Some(("Disabled", State::Disabled)),
            _ => None,
        }
    }
}

pub const ENTRIES: &[Entry] = &[
    Entry {
        id: "2.1.1",
        title: "Buttons",
        widget: Widget::Buttons,
    },
    Entry {
        id: "2.3.1",
        title: "Sliders and knobs",
        widget: Widget::Sliders,
    },
    Entry {
        id: "2.2.3",
        title: "Checkboxes, radio buttons and tabs",
        widget: Widget::Choices,
    },
    Entry {
        id: "2.2.1",
        title: "Segmented control",
        widget: Widget::Pill,
    },
    Entry {
        id: "2.2.2",
        title: "Switches",
        widget: Widget::Switch,
    },
    Entry {
        id: "3.1.2",
        title: "Text fields",
        widget: Widget::Text,
    },
    Entry {
        id: "3.3.1",
        title: "Color picker",
        widget: Widget::Color,
    },
    Entry {
        id: "1.1.1",
        title: "Card",
        widget: Widget::Card,
    },
    Entry {
        id: "1.2.1",
        title: "Info Note",
        widget: Widget::Note(NoteTone::Info),
    },
    Entry {
        id: "1.2.2",
        title: "Tip Note",
        widget: Widget::Note(NoteTone::Tip),
    },
    Entry {
        id: "1.2.3",
        title: "Warning Note",
        widget: Widget::Note(NoteTone::Warning),
    },
    Entry {
        id: "1.2.4",
        title: "Danger Note",
        widget: Widget::Note(NoteTone::Danger),
    },
    Entry {
        id: "3.1.1",
        title: "Password field",
        widget: Widget::Password,
    },
    Entry {
        id: "3.2.1",
        title: "File",
        widget: Widget::File,
    },
    Entry {
        id: "3.2.2",
        title: "Multiple files",
        widget: Widget::Files,
    },
    Entry {
        id: "3.2.3",
        title: "Folder",
        widget: Widget::Folder,
    },
    Entry {
        id: "4.1.1",
        title: "Trigger menu",
        widget: Widget::Menu(MenuTrigger::Button),
    },
    Entry {
        id: "4.1.2",
        title: "Context menu",
        widget: Widget::Menu(MenuTrigger::Context),
    },
    Entry {
        id: "4.2.1",
        title: "Modal",
        widget: Widget::Modal,
    },
];

#[derive(Default)]
pub struct Fixture {
    note: NoteState,
    segment: u8,
    checked: bool,
    password: String,
    revealed: bool,
    activated: bool,
    modal: bool,
    extra_item: bool,
    continuous: f32,
    guided: f32,
    stepped: u16,
    large_range: f32,
    dense_steps: u16,
    cutoff_hz: f32,
    knob: f32,
    trim: f32,
    pan: f32,
    phase: f32,
    tab: u8,
    text: String,
    email: String,
    areas: [String; 3],
    segment_examples: [u8; 4],
    color: [u8; 4],
    picked: Option<(String, String)>,
}

impl Fixture {
    pub fn sample() -> Self {
        Self {
            checked: true,
            continuous: 65.0,
            guided: 42.0,
            stepped: 75,
            large_range: 55_000.0,
            dense_steps: 48_000,
            cutoff_hz: 1000.0,
            knob: 35.0,
            trim: -4.0,
            phase: 350.0,
            color: [225, 134, 82, 255],
            ..Self::default()
        }
    }
}

pub fn preview(
    ui: &mut Ui,
    tokens: UiTokens,
    entry: Entry,
    state: State,
    width: f32,
    fixture: &mut Fixture,
) {
    ui.push_id((entry.id, state as u8), |ui| {
        let bounds = egui::Rect::from_min_size(
            ui.next_widget_position(),
            egui::vec2(width, ui.available_height()),
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(bounds), |ui| {
            ui.set_width(width);
            ui.add_enabled_ui(state != State::Disabled, |ui| {
                render(ui, tokens, entry.widget, state, fixture);
            });
        });
    });
}

fn render(
    ui: &mut Ui,
    tokens: UiTokens,
    widget: Widget,
    state: State,
    fixture: &mut Fixture,
) -> Option<Response> {
    let error = state == State::Error;
    let response = match widget {
        Widget::Buttons => {
            buttons(ui, fixture);
            return None;
        }
        Widget::Sliders => {
            sliders(ui, fixture);
            return None;
        }
        Widget::Choices => {
            checkbox(
                ui,
                Surface::Card,
                CheckStyle::Simple,
                &mut fixture.checked,
                "Keep history",
            );
            ui.horizontal_wrapped(|ui| {
                radio(
                    ui,
                    Surface::Card,
                    RadioStyle::Simple,
                    &mut fixture.segment,
                    0,
                    "Local",
                );
                radio(
                    ui,
                    Surface::Card,
                    RadioStyle::Simple,
                    &mut fixture.segment,
                    1,
                    "Remote",
                );
            });
            notebook(
                ui,
                Surface::Card,
                if ui.available_width() < 160.0 {
                    NotebookOrientation::Vertical
                } else {
                    NotebookOrientation::Horizontal
                },
                &mut fixture.tab,
                &[(0, "General"), (1, "Audio")],
            )
        }
        Widget::Text => {
            let label = ui.label("Name");
            let id = ui.scope_id().with("name");
            orange_ui_controls::text_field(
                ui,
                Surface::Card,
                id,
                &mut fixture.text,
                "Project name",
            )
            .labelled_by(label.id);
            let label = ui
                .horizontal(|ui| {
                    #[cfg(feature = "icons")]
                    {
                        let (slot, _) =
                            ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
                        orange_ui_controls::icons::draw_centered_icon(
                            ui,
                            slot,
                            orange_ui_controls::icons::UiIcon::Material('\u{e0be}'),
                            tokens.theme.text_muted,
                        );
                    }
                    ui.label("Contact email")
                })
                .inner;
            orange_ui_controls::text_field(
                ui,
                Surface::Card,
                id.with("email"),
                &mut fixture.email,
                "name@example.com",
            )
            .labelled_by(label.id);
            if !fixture.email.is_empty()
                && !fixture.email.split_once('@').is_some_and(|(name, domain)| {
                    !name.is_empty()
                        && domain.contains('.')
                        && !domain.ends_with('.')
                        && !fixture.email.contains(char::is_whitespace)
                })
            {
                ui.colored_label(
                    tokens.theme.danger,
                    "Enter an address like name@example.com.",
                );
            }
            for (index, (name, options)) in [
                ("Vertical resize", egui::Vec2b::new(false, true)),
                ("Horizontal resize", egui::Vec2b::new(true, false)),
                ("Two-axis resize", egui::Vec2b::new(true, true)),
            ]
            .into_iter()
            .enumerate()
            {
                ui.add_space(tokens.density.spacing().md);
                let label = ui
                    .horizontal(|ui| {
                        #[cfg(feature = "icons")]
                        {
                            let (slot, _) = ui
                                .allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                            orange_ui_controls::icons::draw_centered_icon(
                                ui,
                                slot,
                                orange_ui_controls::icons::UiIcon::Material('\u{e262}'),
                                tokens.theme.text_muted,
                            );
                        }
                        ui.label(name)
                    })
                    .inner;
                let id = ui.scope_id().with(("area", index));
                orange_ui_controls::resizable_text_area(
                    ui,
                    Surface::Card,
                    id,
                    &mut fixture.areas[index],
                    "Write a description",
                    options,
                )
                .labelled_by(label.id);
            }
            return None;
        }
        Widget::Color => {
            use orange_ui_controls::color_picker::{Labels, popup, swatch};
            let response = ui.button("Edit color");
            let (rect, _) = ui.allocate_exact_size(egui::vec2(48.0, 22.0), egui::Sense::hover());
            swatch(ui, rect, fixture.color);
            if let Some(color) = popup(
                ui,
                &response,
                fixture.color,
                true,
                Labels {
                    channels: ["Red", "Green", "Blue", "Alpha"],
                    hue: "Hue",
                    saturation_value: "Saturation and value",
                    recent: "Recent colors",
                    value: "Hex color",
                    invalid_value: "Enter a valid hex color",
                    apply: "Apply",
                    cancel: "Cancel",
                },
            ) {
                fixture.color = color;
            }
            response
        }
        Widget::Card => {
            components::card(ui, tokens, |ui| {
                ui.add(Label::new(RichText::new("Sync settings").strong()).wrap());
                ui.add(Label::new("Changes sync in the background.").wrap())
            })
            .inner
        }
        Widget::Note(tone) => {
            if fixture.note.dismissed {
                let response = ui.button("Restore");
                if response.clicked() {
                    fixture.note = NoteState::default();
                }
                response
            } else {
                let (title, body) = match tone {
                    NoteTone::Info => (
                        "Offline editing",
                        "Local edits are available while the session is offline.",
                    ),
                    NoteTone::Tip => (
                        "Fine adjustment",
                        "Hold Shift while dragging a knob for smaller changes.",
                    ),
                    NoteTone::Warning => (
                        "Input is clipping",
                        "Lower the input gain until the clip light stays off.",
                    ),
                    NoteTone::Danger => (
                        "Recording interrupted",
                        "The device disconnected. Reconnect it before starting another take.",
                    ),
                };
                Note {
                    tone,
                    title,
                    body,
                    collapse_labels: Some(["Collapse", "Expand"]),
                    dismiss_label: Some("Dismiss"),
                }
                .show(ui, tokens, &mut fixture.note)?
            }
        }
        Widget::Pill => {
            let examples: [(&str, &[(u8, &str)]); 4] = [
                ("2 options · Input", &[(0, "Mono"), (1, "Stereo")]),
                ("3 options · View", &[(0, "List"), (1, "Grid"), (2, "Wave")]),
                (
                    "4 options · Bands",
                    &[(0, "Low"), (1, "Low mid"), (2, "High mid"), (3, "High")],
                ),
                (
                    "5 options · Rating",
                    &[(0, "1"), (1, "2"), (2, "3"), (3, "4"), (4, "5")],
                ),
            ];
            for (index, (name, options)) in examples.into_iter().enumerate() {
                if state == State::Disabled && index != 1 {
                    continue;
                }
                let label = ui.label(name);
                pill_segmented(
                    ui,
                    tokens,
                    &label,
                    &mut fixture.segment_examples[index],
                    options,
                );
            }
            return None;
        }
        Widget::Switch => {
            components::switch(ui, tokens, &mut fixture.checked, "Enable background sync")
        }
        Widget::Password => {
            let label = ui.label("Password");
            components::password(
                ui,
                tokens,
                &mut fixture.password,
                &mut fixture.revealed,
                PasswordLabels {
                    field: "Password",
                    reveal: "Show password",
                    hide: "Hide password",
                },
                error.then_some("Enter a password"),
            )
            .labelled_by(label.id)
        }
        Widget::File | Widget::Files | Widget::Folder => {
            let (name, display, full) = match widget {
                Widget::File => (
                    "Browse for a file",
                    "rehearsal.wav",
                    "C:/Projects/Audio/rehearsal.wav",
                ),
                Widget::Files => (
                    "Browse for files",
                    "rehearsal.wav\nvoice.wav\nbackground.wav",
                    "rehearsal.wav\nvoice.wav\nbackground.wav",
                ),
                _ => (
                    "Browse for a folder",
                    "C:/Projects/Audio/Archive",
                    "C:/Projects/Audio/Archive",
                ),
            };
            let group = ui.scope(|ui| {
                if matches!(widget, Widget::Files) && !error {
                    ui.weak("Drop files here or browse");
                }
                let (display, full) = fixture
                    .picked
                    .as_ref()
                    .map_or((display, full), |(display, full)| {
                        (display.as_str(), full.as_str())
                    });
                if !error && matches!(widget, Widget::File | Widget::Folder) {
                    let result = ui.horizontal_wrapped(|ui| {
                        let response = ui.button((
                            super::advanced::Glyph::Browse.atom(),
                            if matches!(widget, Widget::Folder) {
                                "Choose folder"
                            } else {
                                "Choose file"
                            },
                        ));
                        ui.add(Label::new(display).wrap()).on_hover_text(full);
                        response
                    });
                    return result.inner;
                }
                PathPicker {
                    name,
                    display,
                    full_selection: full,
                    error: error.then_some("Selection unavailable"),
                }
                .show(ui, tokens)
            });
            let response = group.inner;
            let paths = if response.clicked() {
                let dialog = rfd::FileDialog::new().set_title(name);
                match widget {
                    Widget::File => dialog.pick_file().map(|path| vec![path]),
                    Widget::Files => dialog.pick_files(),
                    _ => dialog.pick_folder().map(|path| vec![path]),
                }
            } else if matches!(widget, Widget::Files)
                && !error
                && ui.rect_contains_pointer(group.response.rect)
            {
                ui.input(|input| {
                    let paths: Vec<_> = input
                        .raw
                        .dropped_files
                        .iter()
                        .filter_map(|file| {
                            let path = file.path();
                            (!path.as_os_str().is_empty()).then(|| path.to_path_buf())
                        })
                        .collect();
                    (!paths.is_empty()).then_some(paths)
                })
            } else {
                None
            };
            if let Some(paths) = paths {
                let full = paths
                    .iter()
                    .map(|path| path.to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("\n");
                let display = if matches!(widget, Widget::Folder) {
                    full.clone()
                } else {
                    paths
                        .iter()
                        .map(|path| {
                            path.file_name()
                                .unwrap_or(path.as_os_str())
                                .to_string_lossy()
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                fixture.picked = Some((display, full));
            }
            response
        }
        Widget::Menu(kind) => {
            ui.checkbox(&mut fixture.extra_item, "Show Rename");
            let caption = match kind {
                MenuTrigger::Button => "Actions",
                MenuTrigger::Context => "Right-click or press Enter",
            };
            let trigger = ui
                .add(
                    Button::new(caption)
                        .truncate()
                        .min_size(egui::vec2(0.0, tokens.density.row_height())),
                )
                .on_hover_text(caption);
            let mut items = vec![
                MenuItem {
                    id: "copy",
                    label: "Copy",
                    icon: None,
                    shortcut: Some("Ctrl+C"),
                    enabled: true,
                },
                MenuItem {
                    id: "delete",
                    label: "Delete",
                    icon: None,
                    shortcut: None,
                    enabled: false,
                },
            ];
            if fixture.extra_item {
                items.push(MenuItem {
                    id: "rename",
                    label: "Rename",
                    icon: None,
                    shortcut: Some("F2"),
                    enabled: true,
                });
            }
            if components::menu_with_icons(
                ui,
                tokens,
                &trigger,
                kind,
                &items,
                |item, ui, rect, ink| {
                    use super::advanced::Glyph;
                    let icon = match item {
                        "copy" => Glyph::Copy,
                        "delete" => Glyph::Trash,
                        _ => Glyph::Edit,
                    };
                    icon.draw(ui, rect, ink);
                },
            )
            .is_some()
            {
                fixture.activated = !fixture.activated;
            }
            if fixture.activated {
                ui.add(Label::new("Action selected").wrap());
            }
            trigger
        }
        Widget::Modal => {
            let response = ui.button("Open modal");
            fixture.modal |= response.clicked();
            if fixture.modal {
                let modal = orange_ui_controls::patterns::dialog_with_close(
                    ui.ctx(),
                    Id::unique("gallery-modal").with(response.id),
                    tokens,
                    "Sync settings",
                    Some("Close dialog"),
                    |ui| {
                        ui.label("Changes sync when connected.");
                    },
                    |ui, _| ui.button("Done").clicked(),
                );
                if modal.inner.0 || modal.inner.1 || modal.should_close() {
                    fixture.modal = false;
                    response.request_focus();
                }
            }
            response
        }
    };
    Some(response)
}

fn buttons(ui: &mut Ui, fixture: &mut Fixture) {
    ui.horizontal_wrapped(|ui| {
        for (label, action) in [
            ("Primary", Action::Primary),
            ("Secondary", Action::Secondary),
            ("Outline", Action::Outline),
            ("Tertiary", Action::Tertiary),
            ("Destructive", Action::Destructive),
            ("Menu", Action::Menu),
        ] {
            button(ui, Surface::Card, action, true, Button::new(label));
        }
        if button(
            ui,
            Surface::Card,
            Action::Toggle(fixture.checked),
            true,
            Button::new("Toggle"),
        )
        .clicked()
        {
            fixture.checked = !fixture.checked;
        }
        button(
            ui,
            Surface::Card,
            Action::Primary,
            false,
            Button::new("Disabled"),
        );
    });
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        for (label, style, geometry) in [
            ("Solid", ButtonStyle::Solid, ButtonGeometry::ROUNDED),
            ("Pill", ButtonStyle::Outline, ButtonGeometry::PILL),
            ("Ghost", ButtonStyle::Ghost, ButtonGeometry::NO_BORDER),
            ("Square", ButtonStyle::Subtle, ButtonGeometry::SQUARE),
            ("Compact", ButtonStyle::Outline, ButtonGeometry::COMPACT),
        ] {
            button_with(
                ui,
                Surface::Card,
                ButtonSpec::new(style, ButtonTone::Neutral).geometry(geometry),
                Button::new(label),
            );
        }
    });
}

fn sliders(ui: &mut Ui, fixture: &mut Fixture) {
    let label = ui.label(format!("Continuous · {:.0}%", fixture.continuous));
    guided_slider(
        ui,
        Surface::Card,
        &mut fixture.continuous,
        0.0..=100.0,
        SliderBehavior::Free,
        SliderMarks::Values,
        &[
            SliderGuide {
                fraction: 0.0,
                label: "0 min",
            },
            SliderGuide {
                fraction: 1.0,
                label: "100 max",
            },
        ],
    )
    .labelled_by(label.id);
    ui.add_space(ui.spacing().item_spacing.y);
    let label = ui.label(format!("With guides · {:.0}%", fixture.guided));
    guided_slider(
        ui,
        Surface::Card,
        &mut fixture.guided,
        0.0..=100.0,
        SliderBehavior::Free,
        SliderMarks::NotchesAndValues,
        &[
            SliderGuide {
                fraction: 0.0,
                label: "0",
            },
            SliderGuide {
                fraction: 0.5,
                label: "50",
            },
            SliderGuide {
                fraction: 1.0,
                label: "100",
            },
        ],
    )
    .labelled_by(label.id);
    let label = ui.label(format!("Stepped · {}%", fixture.stepped));
    stepped_slider(
        ui,
        Surface::Card,
        &mut fixture.stepped,
        0..=100,
        NonZeroU16::new(25).unwrap(),
        StepLabels::Values,
    )
    .labelled_by(label.id);
    ui.add_space(ui.spacing().item_spacing.y);
    let label = ui.label(format!(
        "Wide range · {:.1} kHz",
        fixture.large_range / 1000.0
    ));
    guided_slider(
        ui,
        Surface::Card,
        &mut fixture.large_range,
        0.0..=100_000.0,
        SliderBehavior::Free,
        SliderMarks::NotchesAndValues,
        &[
            SliderGuide {
                fraction: 0.0,
                label: "0",
            },
            SliderGuide {
                fraction: 0.25,
                label: "25k",
            },
            SliderGuide {
                fraction: 0.5,
                label: "50k",
            },
            SliderGuide {
                fraction: 0.75,
                label: "75k",
            },
            SliderGuide {
                fraction: 1.0,
                label: "100k",
            },
        ],
    )
    .labelled_by(label.id);
    let label = ui.label(format!("Log cutoff · {:.0} Hz", fixture.cutoff_hz));
    logarithmic_slider(ui, Surface::Card, &mut fixture.cutoff_hz, 20.0..=20_000.0)
        .labelled_by(label.id);
    let label = ui.label(format!("Dense steps · {} frames", fixture.dense_steps));
    stepped_slider(
        ui,
        Surface::Card,
        &mut fixture.dense_steps,
        0..=60_000,
        NonZeroU16::new(100).unwrap(),
        StepLabels::Values,
    )
    .labelled_by(label.id);
    ui.add_space(ui.spacing().item_spacing.y);
    super::card_grid(ui, 4, 85.0, 4, |ui, index| {
        ui.with_layout(
            egui::Layout::top_down(egui::Align::Center),
            |ui| match index {
                0 => {
                    let label = ui.label("Mix (%)");
                    knob(ui, Surface::Card, &mut fixture.knob, 0.0..=100.0, None, &[])
                        .labelled_by(label.id)
                        .on_hover_text("Drag up or right · Shift for fine control");
                }
                1 => {
                    let label = ui.label("Trim (dB)");
                    rotary(
                        ui,
                        Surface::Card,
                        &mut fixture.trim,
                        -12.0..=12.0,
                        Some(1.0),
                        &[0.0, 0.125, 0.25, 0.375, 0.5, 0.625, 0.75, 0.875, 1.0],
                        KnobStyle::Notched,
                    )
                    .labelled_by(label.id)
                    .on_hover_text("Drag up or right · 1 dB steps");
                }
                2 => {
                    let label = ui.label("Pan");
                    rotary(
                        ui,
                        Surface::Card,
                        &mut fixture.pan,
                        -50.0..=50.0,
                        Some(1.0),
                        &[0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0],
                        KnobStyle::Bipolar,
                    )
                    .labelled_by(label.id);
                }
                _ => {
                    let label = ui.label("Phase (°)");
                    rotary(
                        ui,
                        Surface::Card,
                        &mut fixture.phase,
                        0.0..=360.0,
                        Some(5.0),
                        &[0.0, 0.125, 0.25, 0.375, 0.5, 0.625, 0.75, 0.875],
                        KnobStyle::Loop,
                    )
                    .labelled_by(label.id)
                    .on_hover_text("Wraps in both directions · 5° per step");
                }
            },
        );
    });
}
