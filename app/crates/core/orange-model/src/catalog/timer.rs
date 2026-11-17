//! Checked schedule of the `core/on-timer` and `core/on-timeout` entries (11 §10.2).

use std::num::{NonZeroU32, NonZeroU64};

use crate::catalog::intrinsic_id as ids;
use crate::document::NodeInstance;

/// When an interval timer's first tick is due, relative to the moment the run starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerFirst {
    /// At the start time itself.
    Immediate,
    /// One interval after the start time (the default).
    AfterInterval,
}

/// What an interval timer does with ticks that became due before a pump handled them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissedTicks {
    /// Deliver a tick only when it is handled before the next one comes due. When the
    /// run falls behind by more than one tick, all due ticks are dropped and counted in
    /// the next delivered tick's `missed`; nothing is delivered late in a burst.
    Skip,
    /// Deliver one tick for the latest due time; the earlier ones are counted in `missed`
    /// (the default).
    Coalesce,
    /// Deliver up to this many of the latest due ticks, oldest first; older ones are
    /// counted in the first delivered tick's `missed`.
    CatchUp(NonZeroU32),
}

/// A timer entry's checked config. Times are virtual nanoseconds; every scheduled time
/// is `start + offset + k * interval`, so the schedule never drifts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerSchedule {
    /// `core/on-timer`: `interval` (required), `first`, `missed` and `catch_up_max`.
    Interval {
        interval_ns: NonZeroU64,
        first: TimerFirst,
        missed: MissedTicks,
    },
    /// `core/on-timeout`: one tick `delay` (required) after the run starts.
    Timeout { delay_ns: u64 },
}

impl TimerSchedule {
    /// The checked schedule of a timer entry node, or `None` when the node is not a timer
    /// entry or its config is malformed. Durations are canonical decimal nanosecond strings
    /// within the signed `core/duration@1` range; no other field is allowed.
    pub fn from_node(node: &NodeInstance) -> Option<Self> {
        let config = &node.config;
        match node.definition.id.as_str() {
            ids::ON_TIMEOUT => {
                if config.keys().any(|key| key != "delay") {
                    return None;
                }
                let delay_ns = duration(config.get("delay")?)?;
                Some(Self::Timeout { delay_ns })
            }
            ids::ON_TIMER => {
                const FIELDS: [&str; 4] = ["interval", "first", "missed", "catch_up_max"];
                if config.keys().any(|key| !FIELDS.contains(&key.as_str())) {
                    return None;
                }
                let interval_ns = NonZeroU64::new(duration(config.get("interval")?)?)?;
                let first = match config.get("first").map(serde_json::Value::as_str) {
                    None => TimerFirst::AfterInterval,
                    Some(Some("after-interval")) => TimerFirst::AfterInterval,
                    Some(Some("immediate")) => TimerFirst::Immediate,
                    Some(_) => return None,
                };
                let max = config.get("catch_up_max");
                let missed = match (config.get("missed").map(serde_json::Value::as_str), max) {
                    (None | Some(Some("coalesce")), None) => MissedTicks::Coalesce,
                    (Some(Some("skip")), None) => MissedTicks::Skip,
                    (Some(Some("catch-up")), Some(max)) => MissedTicks::CatchUp(
                        max.as_u64()
                            .and_then(|n| u32::try_from(n).ok())
                            .and_then(NonZeroU32::new)?,
                    ),
                    _ => return None,
                };
                Some(Self::Interval {
                    interval_ns,
                    first,
                    missed,
                })
            }
            _ => None,
        }
    }
}

/// A canonical decimal string of non-negative nanoseconds that fits `core/duration@1`.
fn duration(value: &serde_json::Value) -> Option<u64> {
    let text = value.as_str()?;
    let canonical = !text.is_empty()
        && text.bytes().all(|b| b.is_ascii_digit())
        && (text == "0" || !text.starts_with('0'));
    let n: u64 = text.parse().ok().filter(|_| canonical)?;
    (i64::try_from(n).is_ok()).then_some(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn node(id: &str, config: serde_json::Value) -> NodeInstance {
        serde_json::from_value(json!({
            "id": "00000000-0000-4000-8000-000000000001",
            "definition_id": id,
            "definition_version": 1,
            "config_version": 1,
            "config": config,
        }))
        .unwrap()
    }

    #[test]
    fn interval_defaults_and_options() {
        let parse = |config| TimerSchedule::from_node(&node(ids::ON_TIMER, config));
        let interval = NonZeroU64::new(5).unwrap();
        assert_eq!(
            parse(json!({"interval": "5"})),
            Some(TimerSchedule::Interval {
                interval_ns: interval,
                first: TimerFirst::AfterInterval,
                missed: MissedTicks::Coalesce,
            })
        );
        assert_eq!(
            parse(
                json!({"interval": "5", "first": "immediate", "missed": "catch-up", "catch_up_max": 3})
            ),
            Some(TimerSchedule::Interval {
                interval_ns: interval,
                first: TimerFirst::Immediate,
                missed: MissedTicks::CatchUp(NonZeroU32::new(3).unwrap()),
            })
        );
        assert_eq!(
            parse(json!({"interval": "9223372036854775807", "missed": "skip"})).map(|s| match s {
                TimerSchedule::Interval { missed, .. } => missed,
                TimerSchedule::Timeout { .. } => unreachable!(),
            }),
            Some(MissedTicks::Skip)
        );
        for bad in [
            json!({}),
            json!({"interval": "0"}),
            json!({"interval": 5}),
            json!({"interval": "05"}),
            json!({"interval": "-5"}),
            json!({"interval": "9223372036854775808"}),
            json!({"interval": "5", "first": "later"}),
            json!({"interval": "5", "missed": "catch-up"}),
            json!({"interval": "5", "missed": "catch-up", "catch_up_max": 0}),
            json!({"interval": "5", "missed": "catch-up", "catch_up_max": 4294967296u64}),
            json!({"interval": "5", "catch_up_max": 2}),
            json!({"interval": "5", "delay": "1"}),
        ] {
            assert_eq!(parse(bad.clone()), None, "{bad}");
        }
    }

    #[test]
    fn timeout_delay() {
        let parse = |config| TimerSchedule::from_node(&node(ids::ON_TIMEOUT, config));
        assert_eq!(
            parse(json!({"delay": "0"})),
            Some(TimerSchedule::Timeout { delay_ns: 0 })
        );
        for bad in [
            json!({}),
            json!({"delay": "1", "interval": "1"}),
            json!({"delay": ""}),
        ] {
            assert_eq!(parse(bad.clone()), None, "{bad}");
        }
    }
}
