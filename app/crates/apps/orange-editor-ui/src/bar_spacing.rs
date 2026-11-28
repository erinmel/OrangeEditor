//! Spacing items shared by the command toolbar and the dock icon rails.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Spacing {
    Fixed,
    Flexible,
    Divider,
}

impl Spacing {
    pub(super) const ALL: [Self; 3] = [Self::Fixed, Self::Flexible, Self::Divider];

    pub(super) fn key(self) -> &'static str {
        match self {
            Self::Fixed => "toolbar-fixed-spacer",
            Self::Flexible => "toolbar-flex-spacer",
            Self::Divider => "toolbar-divider",
        }
    }

    pub(super) fn add_key(self) -> &'static str {
        match self {
            Self::Fixed => "bar-add-fixed-spacer",
            Self::Flexible => "bar-add-flexible-spacer",
            Self::Divider => "bar-add-separator",
        }
    }

    pub(super) fn read(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.key() == key)
    }

    pub(super) fn extent(self, flexible: f32) -> f32 {
        match self {
            Self::Fixed => 16.0,
            Self::Flexible => flexible,
            Self::Divider => 12.0,
        }
    }

    pub(super) fn paint(self, ui: &egui::Ui, rect: egui::Rect, horizontal_bar: bool) {
        let center = rect.center();
        let color = ui
            .visuals()
            .weak_text_color()
            .gamma_multiply(if self == Self::Flexible { 0.42 } else { 0.58 });
        if self == Self::Divider {
            let half = if horizontal_bar {
                egui::vec2(0.0, 8.0)
            } else {
                egui::vec2(8.0, 0.0)
            };
            ui.painter().line_segment(
                [center - half, center + half],
                egui::Stroke::new(1.0, color),
            );
        } else if self == Self::Fixed {
            let half = if horizontal_bar {
                egui::vec2(0.0, 4.0)
            } else {
                egui::vec2(4.0, 0.0)
            };
            for offset in [-3.0, 3.0] {
                let center = center
                    + if horizontal_bar {
                        egui::vec2(offset, 0.0)
                    } else {
                        egui::vec2(0.0, offset)
                    };
                ui.painter().line_segment(
                    [center - half, center + half],
                    egui::Stroke::new(1.0, color),
                );
            }
        } else {
            let along = if horizontal_bar {
                egui::Vec2::X
            } else {
                egui::Vec2::Y
            };
            for offset in [-4.0, 0.0, 4.0] {
                ui.painter()
                    .circle_filled(center + along * offset, 1.25, color);
            }
        }
    }
}
