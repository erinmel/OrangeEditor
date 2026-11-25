//! Control hints: how a definition asks an editor to present its fields (ADR-33).
//!
//! Hints come from a definition file's `editor.controls` and `editor.layout`. They are
//! presentation only: validation, inference, compilation and execution never read them,
//! and they never change a value. A hint that does not fit its field is dropped with a
//! [`HintProblem`]; it never stops a definition from loading. Words this version does
//! not know are dropped the same way, so a newer package still loads.

use super::TextRef;
use crate::document::{PortDescriptor, PortDirection, PortKind};
use serde_json::Value as Json;
use std::collections::BTreeMap;

/// The control an editor draws for one field.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Widget {
    /// The editor's usual field for the value's type.
    #[default]
    Field,
    Slider,
    Dial,
    Toggle,
    /// A row of choices, for an enumeration of two to five.
    Segmented,
    Dropdown,
    /// A whole number with minus and plus.
    Stepper,
}

/// How a numeric control spreads its range.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Scale {
    #[default]
    Linear,
    /// Each decade takes the same space; the range must be above zero.
    Log,
}

/// How a slider marks its stops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notch {
    Pill,
    Tick,
    Dot,
}

/// How a dial draws its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Knob {
    Arc,
    /// Centered on zero, for a range around it.
    Bipolar,
    Notched,
    /// The upper bound wraps to the lower one.
    Loop,
}

/// One field's hint.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Control {
    pub widget: Widget,
    /// A short unit symbol shown with the value, such as `ms` or `dBFS`.
    pub unit: Option<String>,
    /// Decimals shown.
    pub precision: Option<u8>,
    /// The range the control offers, inside what the value may be.
    pub range: Option<[f64; 2]>,
    pub step: Option<f64>,
    pub scale: Scale,
    /// Values marked on a slider.
    pub marks: Vec<f64>,
    pub notch: Option<Notch>,
    pub knob: Option<Knob>,
    /// A typed number box beside a slider.
    pub number: bool,
    /// A shorter name where the port's or setting's own is too long, as inside a dial.
    pub caption: Option<TextRef>,
}

/// A card region a field can be placed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    /// On the outputs' rows, to their left.
    Header,
    /// Beside the visual.
    Aside,
    /// Across the whole card.
    Body,
    /// Off the card, under Advanced in Properties.
    Advanced,
}

/// How a region lays out its fields.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Arrange {
    #[default]
    Column,
    /// Side by side.
    Row,
    /// This many to a row.
    Grid(u8),
}

/// Fields placed in one region, in order.
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    pub region: Region,
    /// Setting names or input port IDs.
    pub fields: Vec<String>,
    pub arrange: Arrange,
}

/// A visual the editor draws on the card, named by the definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visual {
    LevelMeter,
    Spectrum,
    Waveform,
}

/// Every hint of a definition that fits its fields.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Controls {
    /// By setting name, a property of the config schema.
    pub settings: BTreeMap<String, Control>,
    /// By data input port ID.
    pub inputs: BTreeMap<String, Control>,
    pub visual: Option<Visual>,
    pub regions: Vec<Placement>,
}

impl Controls {
    /// The hint for a setting or, failing that, an input of that name.
    pub fn of(&self, field: &str) -> Option<&Control> {
        self.settings.get(field).or_else(|| self.inputs.get(field))
    }

    /// The region a field is placed in, if a placement names it.
    pub fn region_of(&self, field: &str) -> Option<(Region, Arrange)> {
        self.regions.iter().find_map(|placement| {
            placement
                .fields
                .iter()
                .any(|named| named == field)
                .then_some((placement.region, placement.arrange))
        })
    }

    pub fn is_empty(&self) -> bool {
        self.settings.is_empty()
            && self.inputs.is_empty()
            && self.visual.is_none()
            && self.regions.is_empty()
    }
}

/// Why a hint was dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HintProblem {
    /// The field, or `layout`.
    pub field: String,
    pub reason: HintReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum HintReason {
    /// No setting or data input of that name.
    UnknownField,
    /// A widget, scale, region or visual this version does not know.
    UnknownWord,
    /// The widget does not suit the field's type, such as a dial on text.
    DoesNotFit,
    /// The range is empty, not finite, below zero on a log scale, or wider than the
    /// value may be.
    BadRange,
    /// The hint is not a JSON object of the expected shape.
    Malformed,
}

impl HintReason {
    /// Why, in words for a package author reading a check's output.
    pub fn describe(self) -> &'static str {
        match self {
            Self::UnknownField => "no setting or data input has that name",
            Self::UnknownWord => "it uses a word this version does not know",
            Self::DoesNotFit => "the widget does not suit the field's type",
            Self::BadRange => {
                "its range is empty, not finite, outside the value's bounds, or not above zero on a log scale"
            }
            Self::Malformed => "it is not shaped as a hint",
        }
    }
}

/// What a field holds, as far as choosing a control goes.
#[derive(Debug, Clone, PartialEq)]
enum FieldKind {
    Number {
        whole: bool,
        bounds: [Option<f64>; 2],
    },
    Bool,
    Choice(usize),
    Other,
}

fn setting_kind(schema: &Json) -> FieldKind {
    if let Some(options) = schema["enum"].as_array() {
        return FieldKind::Choice(options.len());
    }
    let bounds = [schema["minimum"].as_f64(), schema["maximum"].as_f64()];
    match schema["type"].as_str() {
        Some("integer") => FieldKind::Number {
            whole: true,
            bounds,
        },
        Some("number") => FieldKind::Number {
            whole: false,
            bounds,
        },
        Some("boolean") => FieldKind::Bool,
        _ => FieldKind::Other,
    }
}

fn input_kind(port: &PortDescriptor) -> FieldKind {
    match port.ty.as_ref().map(|ty| ty.as_str()) {
        Some("core/f64@1" | "core/f32@1") => FieldKind::Number {
            whole: false,
            bounds: [None, None],
        },
        Some("core/i64@1" | "core/u64@1" | "core/i32@1" | "core/u32@1") => FieldKind::Number {
            whole: true,
            bounds: [None, None],
        },
        Some("core/bool@1") => FieldKind::Bool,
        _ => FieldKind::Other,
    }
}

fn widget(word: &str) -> Option<Widget> {
    Some(match word {
        "field" => Widget::Field,
        "slider" => Widget::Slider,
        "dial" => Widget::Dial,
        "toggle" => Widget::Toggle,
        "segmented" => Widget::Segmented,
        "dropdown" => Widget::Dropdown,
        "stepper" => Widget::Stepper,
        _ => return None,
    })
}

fn fits(widget: Widget, kind: &FieldKind) -> bool {
    match (widget, kind) {
        (Widget::Field, _) => true,
        (Widget::Slider | Widget::Dial, FieldKind::Number { .. }) => true,
        (Widget::Stepper, FieldKind::Number { whole: true, .. }) => true,
        (Widget::Toggle, FieldKind::Bool) => true,
        (Widget::Segmented, FieldKind::Choice(count)) => (2..=5).contains(count),
        (Widget::Dropdown, FieldKind::Choice(count)) => *count > 0,
        _ => false,
    }
}

/// Reads one field's hint, checked against what the field holds.
fn control(hint: &Json, kind: &FieldKind) -> Result<Control, HintReason> {
    let hint = hint.as_object().ok_or(HintReason::Malformed)?;
    let widget = match hint.get("widget") {
        None => Widget::Field,
        Some(word) => {
            widget(word.as_str().ok_or(HintReason::Malformed)?).ok_or(HintReason::UnknownWord)?
        }
    };
    if !fits(widget, kind) {
        return Err(HintReason::DoesNotFit);
    }
    let scale = match hint.get("scale").map(Json::as_str) {
        None => Scale::Linear,
        Some(Some("linear")) => Scale::Linear,
        Some(Some("log")) => Scale::Log,
        Some(Some(_)) => return Err(HintReason::UnknownWord),
        Some(None) => return Err(HintReason::Malformed),
    };
    let range = match hint.get("range") {
        None => None,
        Some(Json::Array(pair)) if pair.len() == 2 => {
            let low = pair[0].as_f64().ok_or(HintReason::Malformed)?;
            let high = pair[1].as_f64().ok_or(HintReason::Malformed)?;
            Some([low, high])
        }
        Some(_) => return Err(HintReason::Malformed),
    };
    let bounds = match kind {
        FieldKind::Number { bounds, .. } => *bounds,
        _ => [None, None],
    };
    if let Some([low, high]) = range {
        let inside =
            bounds[0].is_none_or(|min| low >= min) && bounds[1].is_none_or(|max| high <= max);
        if !(low.is_finite() && high.is_finite() && low < high && inside) {
            return Err(HintReason::BadRange);
        }
    }
    if scale == Scale::Log {
        let low = range.map(|[low, _]| low).or(bounds[0]);
        if low.is_none_or(|low| low <= 0.0) {
            return Err(HintReason::BadRange);
        }
    }
    let step = hint
        .get("step")
        .map(|step| step.as_f64().filter(|step| *step > 0.0));
    let step = match step {
        None => None,
        Some(Some(step)) => Some(step),
        Some(None) => return Err(HintReason::Malformed),
    };
    let word = |key: &str| hint.get(key).and_then(Json::as_str);
    let notch = match word("notch") {
        None => None,
        Some("pill") => Some(Notch::Pill),
        Some("tick") => Some(Notch::Tick),
        Some("dot") => Some(Notch::Dot),
        Some(_) => return Err(HintReason::UnknownWord),
    };
    let knob = match word("knob") {
        None => None,
        Some("arc") => Some(Knob::Arc),
        Some("bipolar") => Some(Knob::Bipolar),
        Some("notched") => Some(Knob::Notched),
        Some("loop") => Some(Knob::Loop),
        Some(_) => return Err(HintReason::UnknownWord),
    };
    let caption = match hint.get("caption") {
        None => None,
        Some(text) => Some(
            serde_json::from_value::<TextRef>(text.clone()).map_err(|_| HintReason::Malformed)?,
        ),
    };
    Ok(Control {
        widget,
        unit: word("unit").map(str::to_owned),
        precision: hint
            .get("precision")
            .and_then(Json::as_u64)
            .and_then(|digits| u8::try_from(digits.min(6)).ok()),
        range,
        step,
        scale,
        marks: hint
            .get("marks")
            .and_then(Json::as_array)
            .map(|marks| marks.iter().filter_map(Json::as_f64).collect())
            .unwrap_or_default(),
        notch,
        knob,
        number: hint.get("number").and_then(Json::as_bool).unwrap_or(false),
        caption,
    })
}

fn placement(entry: &Json) -> Result<Placement, HintReason> {
    let region = match entry["region"].as_str().ok_or(HintReason::Malformed)? {
        "header" => Region::Header,
        "aside" => Region::Aside,
        "body" => Region::Body,
        "advanced" => Region::Advanced,
        _ => return Err(HintReason::UnknownWord),
    };
    let fields = entry["fields"]
        .as_array()
        .ok_or(HintReason::Malformed)?
        .iter()
        .map(|field| {
            field
                .as_str()
                .map(str::to_owned)
                .ok_or(HintReason::Malformed)
        })
        .collect::<Result<_, _>>()?;
    let arrange = match entry.get("arrange").map(Json::as_str) {
        None | Some(Some("column")) => Arrange::Column,
        Some(Some("row")) => Arrange::Row,
        Some(Some(word)) => match word.strip_prefix("grid:").and_then(|n| n.parse().ok()) {
            Some(columns) if columns > 0 => Arrange::Grid(columns),
            _ => return Err(HintReason::UnknownWord),
        },
        Some(None) => return Err(HintReason::Malformed),
    };
    Ok(Placement {
        region,
        fields,
        arrange,
    })
}

/// Reads `editor.controls` and `editor.layout` against a definition's config schema and
/// ports. Every hint that does not fit is left out and reported.
pub fn read(
    controls: Option<&Json>,
    layout: Option<&Json>,
    schema: Option<&Json>,
    ports: &[PortDescriptor],
) -> (Controls, Vec<HintProblem>) {
    let mut read = Controls::default();
    let mut problems = Vec::new();
    let mut report = |field: &str, reason| {
        problems.push(HintProblem {
            field: field.to_owned(),
            reason,
        });
    };
    let properties = schema.and_then(|schema| schema["properties"].as_object());
    if let Some(controls) = controls {
        for (name, hint) in controls["settings"].as_object().into_iter().flatten() {
            match properties.and_then(|properties| properties.get(name)) {
                None => report(name, HintReason::UnknownField),
                Some(field) => match control(hint, &setting_kind(field)) {
                    Ok(control) => {
                        let _ = read.settings.insert(name.clone(), control);
                    }
                    Err(reason) => report(name, reason),
                },
            }
        }
        for (name, hint) in controls["inputs"].as_object().into_iter().flatten() {
            let port = ports.iter().find(|port| {
                port.port_id.as_str() == name
                    && port.direction == PortDirection::Input
                    && port.kind == PortKind::Data
            });
            match port {
                None => report(name, HintReason::UnknownField),
                Some(port) => match control(hint, &input_kind(port)) {
                    Ok(control) => {
                        let _ = read.inputs.insert(name.clone(), control);
                    }
                    Err(reason) => report(name, reason),
                },
            }
        }
    }
    if let Some(layout) = layout {
        read.visual = match layout.get("visual").map(Json::as_str) {
            None | Some(Some("none")) => None,
            Some(Some("level-meter")) => Some(Visual::LevelMeter),
            Some(Some("spectrum")) => Some(Visual::Spectrum),
            Some(Some("waveform")) => Some(Visual::Waveform),
            Some(Some(_)) => {
                report("layout", HintReason::UnknownWord);
                None
            }
            Some(None) => {
                report("layout", HintReason::Malformed);
                None
            }
        };
        for entry in layout["regions"].as_array().into_iter().flatten() {
            match placement(entry) {
                Ok(mut placement) => {
                    placement.fields.retain(|field| {
                        let known = properties.is_some_and(|p| p.contains_key(field))
                            || ports.iter().any(|port| {
                                port.port_id.as_str() == field
                                    && port.direction == PortDirection::Input
                            });
                        if !known {
                            report(field, HintReason::UnknownField);
                        }
                        known
                    });
                    read.regions.push(placement);
                }
                Err(reason) => report("layout", reason),
            }
        }
    }
    (read, problems)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema() -> Json {
        json!({"type": "object", "properties": {
            "hold_ms": {"type": "integer", "minimum": 0, "maximum": 10000},
            "reset_db": {"type": "number", "minimum": 0, "maximum": 24},
            "direction": {"type": "string", "enum": ["above", "below"]},
            "label": {"type": "string"},
            "cutoff": {"type": "number", "minimum": 0, "maximum": 20000}
        }})
    }

    fn reasons(controls: Json) -> Vec<(String, HintReason)> {
        read(
            Some(&json!({"settings": controls})),
            None,
            Some(&schema()),
            &[],
        )
        .1
        .into_iter()
        .map(|problem| (problem.field, problem.reason))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
    }

    #[test]
    fn fitting_hints_are_read_with_their_parameters() {
        let (controls, problems) = read(
            Some(&json!({"settings": {
                "hold_ms": {"widget": "dial", "unit": "ms", "step": 1},
                "direction": {"widget": "segmented"},
                "reset_db": {"widget": "slider", "range": [0, 12], "precision": 1,
                             "marks": [0, 6, 12], "notch": "tick", "number": true}
            }})),
            Some(&json!({"visual": "level-meter", "regions": [
                {"region": "aside", "fields": ["hold_ms", "reset_db"], "arrange": "row"},
                {"region": "body", "fields": ["direction"]}
            ]})),
            Some(&schema()),
            &[],
        );
        assert_eq!(problems, []);
        let hold = &controls.settings["hold_ms"];
        assert_eq!(
            (hold.widget, hold.unit.as_deref(), hold.step),
            (Widget::Dial, Some("ms"), Some(1.0))
        );
        let reset = &controls.settings["reset_db"];
        assert_eq!(reset.range, Some([0.0, 12.0]));
        assert_eq!(
            (reset.notch, reset.number, reset.precision),
            (Some(Notch::Tick), true, Some(1))
        );
        assert_eq!(controls.visual, Some(Visual::LevelMeter));
        assert_eq!(
            controls.region_of("reset_db"),
            Some((Region::Aside, Arrange::Row))
        );
        assert_eq!(controls.region_of("label"), None);
    }

    #[test]
    fn hints_that_do_not_fit_are_dropped_with_a_reason() {
        assert_eq!(
            reasons(json!({
                "label": {"widget": "dial"},
                "missing": {"widget": "slider"},
                "hold_ms": {"widget": "knob-o-matic"},
                "direction": {"widget": "toggle"},
                "reset_db": {"widget": "slider", "range": [0, 48]},
                "cutoff": {"widget": "slider", "scale": "log"}
            })),
            [
                ("cutoff".to_owned(), HintReason::BadRange),
                ("direction".to_owned(), HintReason::DoesNotFit),
                ("hold_ms".to_owned(), HintReason::UnknownWord),
                ("label".to_owned(), HintReason::DoesNotFit),
                ("missing".to_owned(), HintReason::UnknownField),
                ("reset_db".to_owned(), HintReason::BadRange),
            ]
        );
        // A log scale needs a range above zero; given one, it fits.
        assert_eq!(
            reasons(json!({"cutoff": {"widget": "slider", "scale": "log", "range": [20, 20000]}})),
            []
        );
    }

    #[test]
    fn a_layout_keeps_known_fields_and_reports_the_rest() {
        let (controls, problems) = read(
            None,
            Some(&json!({"visual": "hologram", "regions": [
                {"region": "body", "fields": ["direction", "nowhere"]},
                {"region": "sidecar", "fields": ["label"]}
            ]})),
            Some(&schema()),
            &[],
        );
        assert_eq!(controls.visual, None);
        assert_eq!(controls.regions.len(), 1);
        assert_eq!(controls.regions[0].fields, ["direction"]);
        assert_eq!(
            problems.iter().map(|p| p.reason).collect::<Vec<_>>(),
            [
                HintReason::UnknownWord,
                HintReason::UnknownField,
                HintReason::UnknownWord
            ]
        );
    }
}
