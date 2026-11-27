//! Checked root Watch configuration (11 §10.5).
use super::intrinsic_id as ids;
use crate::document::NodeInstance;
use crate::value::{Value, shape::ValueShape};
use serde_json::{Number, Value as Json};
use std::cmp::Ordering;

/// A Watch condition resolved to the exact type of the watched input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchCondition {
    Changed,
    Deadband(Value),
    Rising,
    Falling,
    Numeric {
        test: NumericWatch,
        hold_ns: u64,
        cooldown_ns: u64,
    },
}

/// Typed, ordered bounds of a numeric Watch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NumericWatch {
    Above { threshold: Value, release: Value },
    Below { threshold: Value, release: Value },
    Inside { low: Value, high: Value },
    Outside { low: Value, high: Value },
}

impl NumericWatch {
    /// The sampled value meets the firing condition. NaN satisfies no condition.
    pub fn matches(&self, value: &Value) -> bool {
        match self {
            Self::Above { threshold, .. } => compare(value, threshold) == Some(Ordering::Greater),
            Self::Below { threshold, .. } => compare(value, threshold) == Some(Ordering::Less),
            Self::Inside { low, high } => {
                compare(value, low).is_some_and(Ordering::is_ge)
                    && compare(value, high).is_some_and(Ordering::is_le)
            }
            Self::Outside { low, high } => {
                compare(value, low) == Some(Ordering::Less)
                    || compare(value, high) == Some(Ordering::Greater)
            }
        }
    }

    /// An already-fired Watch can start a new crossing after reaching its release.
    pub fn released(&self, value: &Value) -> bool {
        match self {
            Self::Above { release, .. } => compare(value, release).is_some_and(Ordering::is_le),
            Self::Below { release, .. } => compare(value, release).is_some_and(Ordering::is_ge),
            Self::Inside { low, high } => {
                compare(value, low) == Some(Ordering::Less)
                    || compare(value, high) == Some(Ordering::Greater)
            }
            Self::Outside { low, high } => {
                compare(value, low).is_some_and(Ordering::is_ge)
                    && compare(value, high).is_some_and(Ordering::is_le)
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Mode {
    Above,
    Below,
    Inside,
    Outside,
}

#[derive(Debug, Clone, Copy)]
enum Test<'a> {
    Changed,
    Edge(bool),
    Numeric(Mode, &'a Number, &'a Number),
    Deadband(&'a Number),
}

/// The syntactically checked configuration of a Watch before its input type is known.
#[derive(Debug, Clone, Copy)]
pub struct WatchConfig<'a> {
    test: Test<'a>,
    hold_ns: u64,
    cooldown_ns: u64,
}

impl<'a> WatchConfig<'a> {
    /// Reads the settings of a general Watch or one of its dedicated forms.
    pub fn from_node(node: &'a NodeInstance) -> Option<Self> {
        let config = &node.config;
        let specialized = match (node.definition.id.as_str(), node.config_version) {
            (id @ (ids::RISING_EDGE | ids::FALLING_EDGE), 1) if config.is_empty() => {
                Some(Test::Edge(id == ids::RISING_EDGE))
            }
            (ids::CHANGE_BEYOND_TOLERANCE, 1) if config.len() == 1 => {
                let threshold = config.get("threshold")?.as_number()?;
                threshold.as_f64().filter(|value| *value >= 0.0)?;
                Some(Test::Deadband(threshold))
            }
            (ids::ON_WATCH, _) => None,
            _ => return None,
        };
        if let Some(test) = specialized {
            return Some(Self {
                test,
                hold_ns: 0,
                cooldown_ns: 0,
            });
        }
        let mode = config.get("mode")?.as_str()?;
        let change = match (node.config_version, mode) {
            (1 | 2, "changed") => Some(Test::Changed),
            (1 | 2, "rising") => Some(Test::Edge(true)),
            (2, "falling") => Some(Test::Edge(false)),
            _ => None,
        };
        if let Some(test) = change {
            return (config.len() == 1).then_some(Self {
                test,
                hold_ns: 0,
                cooldown_ns: 0,
            });
        }
        if node.config_version != 2 {
            return None;
        }
        let number = |key| config.get(key)?.as_number();
        if mode == "deadband" {
            return (config.len() == 2).then_some(Self {
                test: Test::Deadband(number("threshold")?),
                hold_ns: 0,
                cooldown_ns: 0,
            });
        }
        let (mode, first, second, fields) = match mode {
            "above" | "below" => {
                let threshold = number("threshold")?;
                let release = if config.contains_key("release") {
                    number("release")?
                } else {
                    threshold
                };
                let mode = if mode == "above" {
                    Mode::Above
                } else {
                    Mode::Below
                };
                (mode, threshold, release, ["threshold", "release"])
            }
            "inside" | "outside" => {
                let mode = if mode == "inside" {
                    Mode::Inside
                } else {
                    Mode::Outside
                };
                (mode, number("low")?, number("high")?, ["low", "high"])
            }
            _ => return None,
        };
        if config.keys().any(|key| {
            !matches!(key.as_str(), "mode" | "hold" | "cooldown") && !fields.contains(&key.as_str())
        }) {
            return None;
        }
        let time = |key| match config.get(key) {
            None => Some(0),
            Some(value) => value.as_u64()?.checked_mul(1_000_000),
        };
        Some(Self {
            test: Test::Numeric(mode, first, second),
            hold_ns: time("hold")?,
            cooldown_ns: time("cooldown")?,
        })
    }

    /// Whether this mode can run over the resolved type.
    pub fn accepts_type(self, shape: &ValueShape) -> bool {
        match self.test {
            Test::Changed => shape.host_eq(),
            Test::Edge(_) => shape == &ValueShape::Bool,
            Test::Numeric(..) | Test::Deadband(_) => matches!(
                shape,
                ValueShape::I32
                    | ValueShape::I64
                    | ValueShape::U32
                    | ValueShape::U64
                    | ValueShape::F32
                    | ValueShape::F64
            ),
        }
    }

    /// Converts numeric bounds to the watched type and checks their ordering.
    pub fn resolve(self, shape: &ValueShape) -> Option<WatchCondition> {
        if !self.accepts_type(shape) {
            return None;
        }
        match self.test {
            Test::Changed => Some(WatchCondition::Changed),
            Test::Edge(true) => Some(WatchCondition::Rising),
            Test::Edge(false) => Some(WatchCondition::Falling),
            Test::Deadband(threshold) if threshold.as_f64()? >= 0.0 => {
                Some(WatchCondition::Deadband(bound(threshold, shape)?))
            }
            Test::Deadband(_) => None,
            Test::Numeric(mode, first, second) => {
                let first = bound(first, shape)?;
                let second = bound(second, shape)?;
                let order = compare(&first, &second)?;
                let test = match mode {
                    Mode::Above if order.is_ge() => NumericWatch::Above {
                        threshold: first,
                        release: second,
                    },
                    Mode::Below if order.is_le() => NumericWatch::Below {
                        threshold: first,
                        release: second,
                    },
                    Mode::Inside if order.is_le() => NumericWatch::Inside {
                        low: first,
                        high: second,
                    },
                    Mode::Outside if order.is_le() => NumericWatch::Outside {
                        low: first,
                        high: second,
                    },
                    _ => return None,
                };
                Some(WatchCondition::Numeric {
                    test,
                    hold_ns: self.hold_ns,
                    cooldown_ns: self.cooldown_ns,
                })
            }
        }
    }
}

/// Settings displayed by the editor for both versions. The mode determines the
/// applicable fields, with detailed checking in [`WatchConfig`].
pub(super) fn schema() -> Json {
    serde_json::json!({
        "type": "object",
        "required": ["mode"],
        "properties": {
            "mode": {"type": "string", "enum": ["changed", "rising", "falling", "above", "below", "inside", "outside", "deadband"]},
            "threshold": {"type": "number"},
            "release": {"type": "number"},
            "low": {"type": "number"},
            "high": {"type": "number"},
            "hold": {"type": "integer", "minimum": 0},
            "cooldown": {"type": "integer", "minimum": 0}
        },
        "additionalProperties": false
    })
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "decimal f32 bounds use native f32 rounding"
)]
fn bound(number: &Number, shape: &ValueShape) -> Option<Value> {
    match shape {
        ValueShape::I32 => i32::try_from(number.as_i64()?).ok().map(Value::I32),
        ValueShape::I64 => number.as_i64().map(Value::I64),
        ValueShape::U32 => u32::try_from(number.as_u64()?).ok().map(Value::U32),
        ValueShape::U64 => number.as_u64().map(Value::U64),
        ValueShape::F32 => {
            let value = number.as_f64()? as f32;
            value.is_finite().then_some(Value::F32(value))
        }
        ValueShape::F64 => number.as_f64().filter(|f| f.is_finite()).map(Value::F64),
        _ => None,
    }
}

fn compare(left: &Value, right: &Value) -> Option<Ordering> {
    match (left, right) {
        (Value::I32(a), Value::I32(b)) => Some(a.cmp(b)),
        (Value::I64(a), Value::I64(b)) => Some(a.cmp(b)),
        (Value::U32(a), Value::U32(b)) => Some(a.cmp(b)),
        (Value::U64(a), Value::U64(b)) => Some(a.cmp(b)),
        (Value::F32(a), Value::F32(b)) => a.partial_cmp(b),
        (Value::F64(a), Value::F64(b)) => a.partial_cmp(b),
        _ => None,
    }
}
