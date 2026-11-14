//! Types a service package declares: registered when the host starts and carried
//! between the package's nodes as records.
use orange_host::{
    Call, Host, HostError, HostOptions, KernelFault, ServiceNode, ServicePackage, Value,
};
use orange_sdk::{Definition, FaultText, RecordRepr};
use orange_testkit::{id, node, until};
use serde_json::json;
use std::sync::Arc;

const PACKAGE: &str = "org.example.geo";
const POINT: &str = "org.example.geo/point@1";
const BAD: KernelFault = KernelFault {
    code: FaultText::Static("org.example.geo/Input"),
    message_key: FaultText::Static("org.example.geo/input-invalid"),
    message: "The inputs are not what the node takes.",
};

#[derive(Debug)]
struct MakePoint;
impl ServiceNode for MakePoint {
    fn run(&self, call: &Call<'_>) -> Result<Vec<Value>, KernelFault> {
        let [x @ Value::F64(_), y @ Value::F64(_)] = call.inputs else {
            return Err(BAD);
        };
        Ok(vec![Value::Record(Arc::new(RecordRepr {
            ty: POINT.parse().map_err(|_| BAD)?,
            fields: Box::new([Some(x.clone()), Some(y.clone())]),
        }))])
    }
}

#[derive(Debug)]
struct Length;
impl ServiceNode for Length {
    fn run(&self, call: &Call<'_>) -> Result<Vec<Value>, KernelFault> {
        let [Value::Record(point)] = call.inputs else {
            return Err(BAD);
        };
        let [Some(Value::F64(x)), Some(Value::F64(y))] = &point.fields[..] else {
            return Err(BAD);
        };
        Ok(vec![Value::F64(x.hypot(*y))])
    }
}

fn package() -> ServicePackage {
    let make = Definition::node(PACKAGE, "make-point", "Make point")
        .input("x", "core/f64@1")
        .input("y", "core/f64@1")
        .output("point", POINT)
        .build();
    let length = Definition::node(PACKAGE, "length", "Length")
        .input("point", POINT)
        .output("length", "core/f64@1")
        .build();
    ServicePackage::new(PACKAGE, "1.0.0")
        .record("point", &[("x", "core/f64@1"), ("y", "core/f64@1")])
        .enumeration("side", &[("left", None), ("at", Some(POINT))])
        .node(&make, MakePoint)
        .and_then(|package| package.node(&length, Length))
        .unwrap()
}

#[test]
fn a_packages_record_type_travels_between_its_nodes() {
    let (mut host, _events) = Host::with_services(HostOptions::default(), vec![package()]).unwrap();
    let number = |value: f64| json!({"type": "core/f64@1", "value": value});
    let edge = |n: u32, from: u32, out: &str, to: u32, port: &str| {
        json!({"id": format!("30000000-0000-4000-8000-{n:012x}"),
            "from": {"node": id(from), "port": out}, "to": {"node": id(to), "port": port}})
    };
    let package_node = |n: u32, name: &str, inputs: serde_json::Value| {
        json!({"id": id(n), "definition_id": format!("{PACKAGE}/{name}"),
            "definition_version": 1, "config_version": 1, "config": {}, "inputs": inputs})
    };
    let written = json!({"format": "orange.graph", "format_version": 1,
        "graph_id": "20000000-0000-4000-8000-0000000000a7", "revision": 1,
        "package_lock": [], "type_lock": [], "events": [], "connections": [],
        "signatures": {}, "functions": [], "exports": [], "editor": {},
        "variables": [{"id": id(100), "name": "length", "type": "core/f64@1", "scope": "run",
            "persistent": false, "conflict": "reject"}],
        "nodes": [
            {"id": id(1), "definition_id": "core/on-manual", "definition_version": 1,
                "config_version": 1, "config": {}},
            package_node(2, "make-point", json!({"x": number(3.0), "y": number(4.0)})),
            package_node(3, "length", json!({})),
            {"id": id(4), "definition_id": "core/set-variable", "definition_version": 1,
                "config_version": 1, "config": {"variable": id(100)}}],
        "edges": [edge(1, 1, "then", 2, "in"), edge(2, 2, "then", 3, "in"),
            edge(3, 2, "point", 3, "point"), edge(4, 3, "then", 4, "in"),
            edge(5, 3, "length", 4, "value")]});
    let bound = host.bind(&written.to_string()).unwrap();
    assert_eq!(host.check(&bound).unwrap(), []);
    let flow = host.open(&bound).unwrap();
    host.trigger(flow, node(1), None).unwrap();
    let length = || {
        host.inspect(flow, |run| run.variable(id(100).parse().unwrap()).cloned())
            .unwrap()
    };
    until("the length", || length().is_some());
    assert_eq!(length(), Some(Value::F64(5.0)));
    // Both declared types are known to the registry.
    for name in ["point", "side"] {
        let term = orange_model::type_term::TypeTerm::parse(
            &format!("{PACKAGE}/{name}@1").parse().unwrap(),
        )
        .unwrap();
        assert!(host.registry().types().validate(&term).is_ok(), "{name}");
    }
}

#[test]
fn a_type_declaration_that_is_not_valid_stops_the_host_from_starting() {
    let start = |package: ServicePackage| {
        Host::with_services(HostOptions::default(), vec![package]).map(|_| ())
    };
    let new = || ServicePackage::new(PACKAGE, "1.0.0");
    for bad in [
        new().record("point", &[("x", "not a type")]),
        new().record("point", &[("x", "org.example.other/missing@1")]),
        new().record("Bad Name", &[("x", "core/f64@1")]),
        new()
            .record("point", &[("x", "core/f64@1")])
            .record("point", &[("y", "core/f64@1")]),
    ] {
        assert!(matches!(start(bad), Err(HostError::Start(_))));
    }
    // A disabled package's types are not registered and do not stop the host.
    let options = HostOptions {
        disabled_packages: [PACKAGE.to_owned()].into(),
        ..HostOptions::default()
    };
    assert!(Host::with_services(options, vec![new().record("point", &[("x", "nope")])]).is_ok());
}

const SHAPES: &str = "org.example.shapes";
const SHAPE_POINT: &str = "org.example.shapes/point@1";

/// An installed package with no code: a record type and one node, made as a graph,
/// that hands a point through. `returns` is the type of the graph's own output, so a
/// wrong one makes the package invalid after its type was read.
fn shapes_archive(version: &str, returns: &str) -> Vec<u8> {
    let pass = format!("{SHAPES}/pass");
    let port = |name: &str, direction: &str, ty: &str| json!({"port_id": name, "direction": direction, "kind": "data", "type": ty});
    let manifest = json!({
        "format": "orange.plugin", "format_version": 1,
        "id": SHAPES, "version": version, "publisher": "org.example",
        "host_api": {"major": 1, "min_minor": 0, "max_minor": 0},
        "dependencies": [],
        "contributions": {
            "nodes": [pass],
            "providers": [{"slot": format!("node:{pass}@1"), "id": orange_host::GRAPH_PROVIDER}]
        },
        "permissions": [],
        "license": "MIT"
    });
    let definition = json!({"definition_id": pass, "definition_version": 1,
    "config_version": 1, "kind": "action", "suspends": "never", "determinism": "external",
    "ports": [
        port("p", "input", SHAPE_POINT),
        {"port_id": "result", "direction": "output", "kind": "data", "type": SHAPE_POINT,
            "source": "execution", "available_on": ["then"]}
    ]});
    let core = |n: u32, name: &str, config: serde_json::Value| {
        json!({"id": id(n), "definition_id": format!("core/{name}"), "definition_version": 1,
            "config_version": 1, "config": config})
    };
    let edge = |n: u32, from: u32, out: &str, to: u32, port: &str| {
        json!({"id": format!("30000000-0000-4000-8000-{n:012x}"),
            "from": {"node": id(from), "port": out}, "to": {"node": id(to), "port": port}})
    };
    // The point is kept in a variable of the graph's own and read back.
    let graph = json!({
        "format": orange_host::GRAPH_NODE_FORMAT, "format_version": 1,
        "function": {"id": id(200), "name": "pass", "kind": "action",
            "inputs": [port("p", "input", SHAPE_POINT)],
            "outputs": [port("result", "output", returns)],
            "nodes": [core(201, "function-entry", json!({})),
                core(203, "set-variable", json!({"variable": id(210)})),
                core(204, "get-variable", json!({"variable": id(210)})),
                core(205, "return", json!({}))],
            "edges": [edge(201, 201, "then", 203, "in"), edge(202, 201, "p", 203, "value"),
                edge(203, 203, "then", 205, "in"), edge(204, 204, "value", 205, "result")]},
        "variables": [{"id": id(210), "name": "kept", "type": SHAPE_POINT, "scope": "local",
            "init": {"type": SHAPE_POINT, "value": {"x": "0"}}}]
    });
    let point = json!({"kind": "record", "fields": [
        {"id": "x", "type": "core/i64@1"}, {"id": "y", "type": "core/i64@1", "optional": true}]});
    orange_host::pack(
        manifest.to_string().as_bytes(),
        &[
            (
                "nodes/pass.json".into(),
                definition.to_string().into_bytes(),
            ),
            ("graphs/pass.json".into(), graph.to_string().into_bytes()),
            ("types/point.json".into(), point.to_string().into_bytes()),
        ],
    )
    .unwrap()
}

fn point_known(host: &Host) -> bool {
    let term = orange_model::type_term::TypeTerm::parse(&SHAPE_POINT.parse().unwrap()).unwrap();
    host.registry().types().validate(&term).is_ok()
}

#[test]
fn an_installed_package_brings_its_types_and_a_refused_one_leaves_none() {
    let store = std::env::temp_dir().join(format!("orange-host-types-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&store);
    let options = || HostOptions {
        package_dir: Some(store.clone()),
        ..HostOptions::default()
    };
    let (mut host, _events) = Host::new(options()).unwrap();
    // A package whose graph does not keep its contract is refused; its type goes too.
    assert!(
        host.install(&shapes_archive("1.0.0", "core/i64@1"), Default::default())
            .is_err()
    );
    assert!(!point_known(&host));

    let _ = host
        .install(&shapes_archive("1.0.0", SHAPE_POINT), Default::default())
        .unwrap();
    assert!(point_known(&host));

    let variable = |n: u32, init: Option<serde_json::Value>| {
        let mut declared = json!({"id": id(n), "name": format!("v{n}"), "type": SHAPE_POINT,
            "scope": "run", "persistent": false, "conflict": "reject"});
        if let Some(value) = init {
            declared["init"] = json!({"type": SHAPE_POINT, "value": value});
        }
        declared
    };
    let core = |n: u32, name: &str, config: serde_json::Value| {
        json!({"id": id(n), "definition_id": format!("core/{name}"), "definition_version": 1,
            "config_version": 1, "config": config})
    };
    let edge = |n: u32, from: u32, out: &str, to: u32, port: &str| {
        json!({"id": format!("30000000-0000-4000-8000-{n:012x}"),
            "from": {"node": id(from), "port": out}, "to": {"node": id(to), "port": port}})
    };
    let written = json!({"format": "orange.graph", "format_version": 1,
        "graph_id": "20000000-0000-4000-8000-0000000000a8", "revision": 1,
        "package_lock": [], "type_lock": [], "events": [], "connections": [],
        "signatures": {}, "functions": [], "exports": [], "editor": {},
        "variables": [variable(100, Some(json!({"x": "3", "y": "4"}))), variable(101, None)],
        "nodes": [
            core(1, "on-manual", json!({})),
            core(2, "get-variable", json!({"variable": id(100)})),
            {"id": id(3), "definition_id": format!("{SHAPES}/pass"), "definition_version": 1,
                "config_version": 1, "config": {}},
            core(4, "set-variable", json!({"variable": id(101)}))],
        "edges": [edge(1, 1, "then", 3, "in"), edge(2, 2, "value", 3, "p"),
            edge(3, 3, "then", 4, "in"), edge(4, 3, "result", 4, "value")]});
    let bound = host.bind(&written.to_string()).unwrap();
    assert_eq!(host.check(&bound).unwrap(), []);
    let run = |host: &mut Host| {
        let flow = host.open(&bound).unwrap();
        host.trigger(flow, node(1), None).unwrap();
        let copied = || {
            host.inspect(flow, |run| run.variable(id(101).parse().unwrap()).cloned())
                .unwrap()
        };
        until("the copied point", || copied().is_some());
        let Some(Value::Record(point)) = copied() else {
            panic!("the variable holds a record");
        };
        assert_eq!(point.ty.to_string(), SHAPE_POINT);
        assert_eq!(
            &point.fields[..],
            [Some(Value::I64(3)), Some(Value::I64(4))]
        );
    };
    run(&mut host);
    // A newer version with the same type is installed beside the first.
    let _ = host
        .install(&shapes_archive("1.1.0", SHAPE_POINT), Default::default())
        .unwrap();
    assert_eq!(host.check(&bound).unwrap(), []);
    drop(host);

    // A host started from the same directory has the type again and runs the document.
    let (mut host, _events) = Host::new(options()).unwrap();
    assert_eq!(host.not_restored().len(), 0);
    assert!(point_known(&host));
    run(&mut host);
    drop(host);
    let _ = std::fs::remove_dir_all(&store);
}
