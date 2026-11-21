//! Input and shape helpers for headless egui tests.

use egui::{Color32, Event, Modifiers, PointerButton, Pos2, Rect, Shape, epaint::ClippedShape};

/// Sends each half of a click on a separate frame.
pub fn click(pos: Pos2, button: PointerButton, mut frame: impl FnMut(Vec<Event>)) {
    frame(vec![Event::PointerMoved(pos)]);
    for pressed in [true, false] {
        frame(vec![Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: Modifiers::NONE,
        }]);
    }
}

/// Finds the visual center of a rendered label.
pub fn text_center(shapes: &[ClippedShape], label: &str) -> Option<Pos2> {
    shapes.iter().find_map(|item| match &item.shape {
        Shape::Text(text) if text.galley.text() == label => {
            Some(text.pos + text.galley.mesh_bounds.center().to_vec2())
        }
        _ => None,
    })
}

/// Finds the fill of a rectangle emitted at the given bounds.
pub fn rect_fill(shapes: &[ClippedShape], rect: Rect) -> Option<Color32> {
    shapes.iter().find_map(|item| match &item.shape {
        Shape::Rect(shape) if shape.rect == rect => Some(shape.fill),
        _ => None,
    })
}
