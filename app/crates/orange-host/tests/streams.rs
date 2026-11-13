//! Streams a service package's node produces and a graph consumes.
use orange_host::{
    Call, Host, HostOptions, Items, KernelFault, ServicePackage, ServiceStream, Value,
};
use orange_sdk::{Definition, FaultText};
use orange_testkit::{id, node, until};
use serde_json::{Value as Json, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

const PACKAGE: &str = "org.example.feed";
const BAD: KernelFault = KernelFault {
    code: FaultText::Static("org.example.feed/Input"),
    message_key: FaultText::Static("org.example.feed/input-invalid"),
    message: "The count is not a whole number of items.",
};

/// What the test sees of the producer: how many items were asked for and whether it
/// was dropped.
#[derive(Debug, Default)]
struct Seen {
    pulled: AtomicUsize,
    dropped: AtomicBool,
}

/// Counts up from zero: `count` items, or without end when `count` is negative.
struct Counting {
    next: i64,
    count: i64,
    seen: Arc<Seen>,
}
impl Iterator for Counting {
    type Item = Result<Value, KernelFault>;
    fn next(&mut self) -> Option<Self::Item> {
        let _ = self.seen.pulled.fetch_add(1, Ordering::SeqCst);
        if self.count >= 0 && self.next >= self.count {
            return None;
        }
        self.next += 1;
        Some(Ok(Value::I64(self.next - 1)))
    }
}
impl Drop for Counting {
    fn drop(&mut self) {
        self.seen.dropped.store(true, Ordering::SeqCst);
    }
}

#[derive(Debug)]
struct Numbers(Arc<Seen>);
impl ServiceStream for Numbers {
    fn open(&self, call: &Call<'_>) -> Result<Box<dyn Items>, KernelFault> {
        let [Value::I64(count)] = call.inputs else {
            return Err(BAD);
        };
        if *count == 13 {
            return Err(BAD);
        }
        Ok(Box::new(Counting {
            next: 0,
            count: *count,
            seen: Arc::clone(&self.0),
        }))
    }
}

fn host(seen: &Arc<Seen>) -> Host {
    let definition = Definition::node(PACKAGE, "numbers", "Numbers")
        .input("count", "core/i64@1")
        .output("items", "core/stream@1<core/i64@1>")
        .build();
    let package = ServicePackage::new(PACKAGE, "1.0.0")
        .stream(&definition, Numbers(Arc::clone(seen)))
        .unwrap();
    Host::with_services(HostOptions::default(), vec![package])
        .unwrap()
        .0
}

/// A manual trigger opens the stream of `count` numbers, collects at most `max_items`
/// into variable 100 and then sets variable 101; the failed route of either node sets
/// it to -1.
fn document(host: &Host, count: i64, max_items: u32) -> String {
    let core = |n: u32, name: &str, config: Json| {
        json!({"id": id(n), "definition_id": format!("core/{name}"), "definition_version": 1,
            "config_version": 1, "config": config})
    };
    let edge = |n: u32, from: u32, out: &str, to: u32, port: &str| {
        json!({"id": format!("30000000-0000-4000-8000-{n:012x}"),
            "from": {"node": id(from), "port": out}, "to": {"node": id(to), "port": port}})
    };
    let variable = |n: u32, ty: &str| {
        json!({"id": id(n), "name": format!("v{n}"), "type": ty, "scope": "run",
            "persistent": false, "conflict": "reject"})
    };
    let number = |value: i64| json!({"type": "core/i64@1", "value": value.to_string()});
    let mut done = core(5, "set-variable", json!({"variable": id(101)}));
    done["inputs"] = json!({"value": number(1)});
    let mut failed = core(6, "set-variable", json!({"variable": id(101)}));
    failed["inputs"] = json!({"value": number(-1)});
    let written = json!({"format": "orange.graph", "format_version": 1,
        "graph_id": "20000000-0000-4000-8000-0000000000b9", "revision": 1,
        "package_lock": [], "type_lock": [], "events": [], "connections": [],
        "signatures": {}, "functions": [], "exports": [], "editor": {},
        "variables": [variable(100, "core/list@1<core/i64@1>"), variable(101, "core/i64@1")],
        "nodes": [
            core(1, "on-manual", json!({})),
            {"id": id(2), "definition_id": format!("{PACKAGE}/numbers"), "definition_version": 1,
                "config_version": 1, "config": {}, "inputs": {"count": number(count)}},
            core(3, "collect", json!({"max_items": max_items})),
            core(4, "set-variable", json!({"variable": id(100)})),
            done, failed],
        "edges": [edge(1, 1, "then", 2, "in"), edge(2, 2, "then", 3, "in"),
            edge(3, 2, "items", 3, "stream"), edge(4, 3, "then", 4, "in"),
            edge(5, 3, "items", 4, "value"), edge(6, 4, "then", 5, "in"),
            edge(7, 2, "failed", 6, "in"), edge(8, 3, "failed", 6, "in")]});
    let bound = host.bind(&written.to_string()).unwrap();
    assert_eq!(host.check(&bound).unwrap(), []);
    bound
}

fn read(host: &Host, flow: orange_host::FlowId, n: u32) -> Option<Value> {
    host.inspect(flow, move |run| {
        run.variable(id(n).parse().unwrap()).cloned()
    })
    .unwrap()
}

fn numbers(value: Option<Value>) -> Vec<i64> {
    let Some(Value::List(list)) = value else {
        panic!("the variable holds a list");
    };
    list.iter()
        .map(|item| match item {
            Value::I64(number) => number,
            other => panic!("not a number: {other:?}"),
        })
        .collect()
}

#[test]
fn a_graph_collects_the_stream_a_package_node_produces() {
    let seen = Arc::new(Seen::default());
    let mut host = host(&seen);
    let flow = host.open(&document(&host, 5, 100)).unwrap();
    host.trigger(flow, node(1), None).unwrap();
    until("the collected items", || read(&host, flow, 101).is_some());
    assert_eq!(read(&host, flow, 101), Some(Value::I64(1)));
    assert_eq!(numbers(read(&host, flow, 100)), [0, 1, 2, 3, 4]);
    // Five items and the end were asked for, one at a time, and the producer is gone.
    assert_eq!(seen.pulled.load(Ordering::SeqCst), 6);
    until("the producer to be dropped", || {
        seen.dropped.load(Ordering::SeqCst)
    });
}

#[test]
fn an_endless_stream_is_read_only_as_far_as_the_graph_asks() {
    let seen = Arc::new(Seen::default());
    let mut host = host(&seen);
    // A stream longer than the collection allows fails the collection (11 §9); the
    // producer is not run ahead of the graph, and it is dropped when the stream closes.
    let flow = host.open(&document(&host, -1, 3)).unwrap();
    host.trigger(flow, node(1), None).unwrap();
    until("the failed route", || read(&host, flow, 101).is_some());
    assert_eq!(read(&host, flow, 101), Some(Value::I64(-1)));
    assert_eq!(read(&host, flow, 100), None);
    let pulled = seen.pulled.load(Ordering::SeqCst);
    assert!((4..=5).contains(&pulled), "{pulled}");
    until("the producer to be dropped", || {
        seen.dropped.load(Ordering::SeqCst)
    });
    assert!(seen.pulled.load(Ordering::SeqCst) <= 5);
}

#[test]
fn a_stream_that_does_not_open_takes_the_failed_route() {
    let seen = Arc::new(Seen::default());
    let mut host = host(&seen);
    let flow = host.open(&document(&host, 13, 3)).unwrap();
    host.trigger(flow, node(1), None).unwrap();
    until("the failed route", || read(&host, flow, 101).is_some());
    assert_eq!(read(&host, flow, 101), Some(Value::I64(-1)));
    assert_eq!(seen.pulled.load(Ordering::SeqCst), 0);
}
