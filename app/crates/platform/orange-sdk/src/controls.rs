//! Builders for control hints (ADR-33): how a definition asks an editor to present its
//! settings and inputs. They write the definition's `editor.controls` and
//! `editor.layout`; an editor that does not know a hint shows its usual field.

use serde_json::{Map, Value as Json, json};

/// The control an editor draws for one setting or input.
#[derive(Debug, Clone, PartialEq)]
#[must_use]
pub struct Control(Map<String, Json>);

impl Control {
    fn widget(widget: &str) -> Self {
        let mut hint = Map::new();
        let _ = hint.insert("widget".into(), widget.into());
        Self(hint)
    }
    fn with(mut self, key: &str, value: Json) -> Self {
        let _ = self.0.insert(key.into(), value);
        self
    }
    /// A slider along the value's range.
    pub fn slider() -> Self {
        Self::widget("slider")
    }
    /// A self-contained dial with its name, value and unit inside.
    pub fn dial() -> Self {
        Self::widget("dial")
    }
    /// An on and off switch, for a true or false value.
    pub fn toggle() -> Self {
        Self::widget("toggle")
    }
    /// A row of choices, for an enumeration of two to five with short names; longer
    /// names read better in a dropdown.
    pub fn segmented() -> Self {
        Self::widget("segmented")
    }
    pub fn dropdown() -> Self {
        Self::widget("dropdown")
    }
    /// A whole number with minus and plus.
    pub fn stepper() -> Self {
        Self::widget("stepper")
    }
    /// A short unit symbol shown with the value, such as `ms`.
    pub fn unit(self, unit: &str) -> Self {
        self.with("unit", unit.into())
    }
    /// Decimals shown.
    pub fn precision(self, digits: u8) -> Self {
        self.with("precision", digits.into())
    }
    /// The range the control offers, inside the range the value may take.
    pub fn range(self, low: f64, high: f64) -> Self {
        self.with("range", json!([low, high]))
    }
    pub fn step(self, step: f64) -> Self {
        self.with("step", step.into())
    }
    /// Each decade takes the same space; the range must be above zero.
    pub fn log(self) -> Self {
        self.with("scale", "log".into())
    }
    /// Values marked on a slider.
    pub fn marks(self, marks: &[f64]) -> Self {
        self.with("marks", json!(marks))
    }
    /// How a slider marks its stops: `pill`, `tick` or `dot`.
    pub fn notch(self, notch: &str) -> Self {
        self.with("notch", notch.into())
    }
    /// How a dial draws its value: `arc`, `bipolar`, `notched` or `loop`.
    pub fn knob(self, knob: &str) -> Self {
        self.with("knob", knob.into())
    }
    /// A typed number box beside a slider.
    pub fn number(self) -> Self {
        self.with("number", true.into())
    }
    /// A shorter name, under message key `key`, where the field's own is too long.
    pub fn caption(self, key: &str, fallback: &str) -> Self {
        self.with("caption", json!({"key": key, "fallback": fallback}))
    }
    pub(crate) fn into_json(self) -> Json {
        Json::Object(self.0)
    }
}

/// Where a node's fields go on its card, and the visual it shows.
#[derive(Debug, Clone, Default, PartialEq)]
#[must_use]
pub struct Layout {
    visual: Option<String>,
    regions: Vec<Json>,
}

impl Layout {
    pub fn new() -> Self {
        Self::default()
    }
    /// A visual the editor draws: `level-meter`, `spectrum` or `waveform`.
    pub fn visual(mut self, visual: &str) -> Self {
        self.visual = Some(visual.to_owned());
        self
    }
    fn region(mut self, region: &str, fields: &[&str], arrange: &str) -> Self {
        self.regions
            .push(json!({"region": region, "fields": fields, "arrange": arrange}));
        self
    }
    /// Fields on the outputs' rows, to their left.
    pub fn header(self, fields: &[&str]) -> Self {
        self.region("header", fields, "column")
    }
    /// Fields beside the visual, one under another.
    pub fn aside(self, fields: &[&str]) -> Self {
        self.region("aside", fields, "column")
    }
    /// Fields beside the visual, side by side.
    pub fn aside_row(self, fields: &[&str]) -> Self {
        self.region("aside", fields, "row")
    }
    /// Fields across the whole card.
    pub fn body(self, fields: &[&str]) -> Self {
        self.region("body", fields, "column")
    }
    /// Fields across the whole card, side by side.
    pub fn body_row(self, fields: &[&str]) -> Self {
        self.region("body", fields, "row")
    }
    /// Fields kept off the card, under Advanced in Properties.
    pub fn advanced(self, fields: &[&str]) -> Self {
        self.region("advanced", fields, "column")
    }
    pub(crate) fn into_json(self) -> Json {
        let mut layout = Map::new();
        if let Some(visual) = self.visual {
            let _ = layout.insert("visual".into(), visual.into());
        }
        let _ = layout.insert("regions".into(), Json::Array(self.regions));
        Json::Object(layout)
    }
}
