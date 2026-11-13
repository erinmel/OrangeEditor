//! Test support shared by the integration tests of the host and the service packages:
//! stable IDs, the JSON of the few core nodes a test graph is made of, and waiting for
//! something a flow does on its own thread.
//!
//! It is a development dependency only; nothing here is product code. Its functions
//! panic on failure, as test code does.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::missing_panics_doc)]

use orange_model::id::{NodeId, VariableId};
use serde_json::{Value as Json, json};
use std::time::{Duration, Instant};

/// How long [`until`] waits before it fails the test.
pub const WAIT: Duration = Duration::from_secs(20);

/// The ID text every test uses for node or variable number `n`.
pub fn id(n: u32) -> String {
    format!("10000000-0000-4000-8000-{n:012x}")
}
/// Node number `n`.
pub fn node(n: u32) -> NodeId {
    id(n).parse().unwrap()
}
/// Variable number `n`.
pub fn variable(n: u32) -> VariableId {
    id(n).parse().unwrap()
}
/// The activation of an entry node in tests: a queue of eight that faults when full.
pub fn activation() -> Json {
    json!({"mode": "queue", "capacity": 8, "overflow": "fault", "on_fault": "continue", "order": 0})
}
/// Core node `core/<name>` as node number `n`; an `on-…` entry gets [`activation`].
pub fn core(n: u32, name: &str) -> Json {
    let mut node = json!({"id": id(n), "definition_id": format!("core/{name}"),
        "definition_version": 1, "config_version": 1, "config": {}});
    if name.starts_with("on-") {
        node["activation"] = activation();
    }
    node
}
/// A `core/set-variable` node number `n` that writes variable number `target`.
pub fn set(n: u32, target: u32) -> Json {
    let mut node = core(n, "set-variable");
    node["config"] = json!({"variable": id(target)});
    node
}
/// Edge number `n` from port `out` of node `from` to port `input` of node `to`.
pub fn edge(n: u32, from: u32, out: &str, to: u32, input: &str) -> Json {
    json!({"id": format!("30000000-0000-4000-8000-{n:012x}"),
        "from": {"node": id(from), "port": out}, "to": {"node": id(to), "port": input}})
}
/// Waits until `done` is true, failing the test after [`WAIT`].
pub fn until(what: &str, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}
