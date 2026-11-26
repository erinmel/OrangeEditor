//! Authoring IDs for what the palette offers beyond single node kinds: ready-made
//! arrangements of nodes, and older IDs that now name a dedicated definition.
//! [`Host::add_node`](crate::Host::add_node) expands each one; none of them is ever
//! saved in a document.

/// What every preset ID starts with.
pub const PREFIX: &str = "preset:";

/// A Dispatch and a Listen on a new channel.
pub const SIGNAL: &str = "preset:signal";
/// A pulse Dispatch and Listen on a new channel.
pub const WIRELESS_PULSE: &str = "preset:wireless-pulse";
/// A Set Variable and Get Variable on a new variable, sharing a value without a wire.
pub const WIRELESS_VALUE: &str = "preset:wireless-value";
/// A heartbeat pulse with a Wait that times out when the beats stop.
pub const WATCHDOG: &str = "preset:watchdog";
/// A Branch that lets the flow through only when its condition holds.
pub const GUARD: &str = "preset:guard";

/// Older ID of Increment Counter.
pub const INCREMENT: &str = "preset:increment";
/// Older ID of Decrement Counter.
pub const DECREMENT: &str = "preset:decrement";
/// Older ID of Rising Edge.
pub const RISING_EDGE: &str = "preset:rising-edge";
/// Older ID of Falling Edge.
pub const FALLING_EDGE: &str = "preset:falling-edge";
/// Older ID of Change Beyond Tolerance.
pub const DEADBAND: &str = "preset:deadband";

/// Whether `id` is a preset rather than a node kind.
pub fn is_preset(id: &str) -> bool {
    id.starts_with(PREFIX)
}
