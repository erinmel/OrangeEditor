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
