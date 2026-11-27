//! A package version that renames a node: documents saved with the old node open with the
//! new one, values and wires kept (12 §6.4).
use orange_host::{
    Call, Host, HostOptions, KernelFault, ServiceNode, ServicePackage, Severity, Value,
    new_document,
};
use orange_sdk::Definition;

const PACKAGE: &str = "org.example.math";

#[derive(Debug)]
struct Twice;
impl ServiceNode for Twice {
    fn run(&self, call: &Call<'_>) -> Result<Vec<Value>, KernelFault> {
        match call.inputs {
            [Value::F64(x)] => Ok(vec![Value::F64(x * 2.0)]),
            _ => Ok(vec![Value::F64(0.0)]),
        }
    }
}

/// Version 1 ships `scale` (`amount` → `scaled`).
fn version_1() -> ServicePackage {
    let scale = Definition::node(PACKAGE, "scale", "Scale")
        .input("amount", "core/f64@1")
        .output("scaled", "core/f64@1")
        .build();
    ServicePackage::new(PACKAGE, "1.0.0")
        .node(&scale, Twice)
        .unwrap()
}

/// Version 2 ships `multiply` (`factor` → `result`) in its place and drops `scale`.
fn version_2() -> ServicePackage {
    let multiply = Definition::node(PACKAGE, "multiply", "Multiply")
        .replaces("scale", 1, &[("amount", "factor"), ("scaled", "result")])
        .input("factor", "core/f64@1")
        .output("result", "core/f64@1")
        .build();
    ServicePackage::new(PACKAGE, "2.0.0")
        .node(&multiply, Twice)
        .unwrap()
}

fn errors(problems: &[orange_host::Diagnostic]) -> Vec<String> {
    problems
        .iter()
        .filter(|problem| problem.severity == Severity::Error)
        .map(|problem| format!("{:?}", problem.code))
        .collect()
}

#[test]
fn a_document_saved_with_a_replaced_node_opens_with_its_replacement() {
    let (old, _events) = Host::with_services(HostOptions::default(), vec![version_1()]).unwrap();
    let mut draft = old.draft(&new_document().unwrap()).unwrap();
    let scale = old
        .add_node(&mut draft, &format!("{PACKAGE}/scale"), [0.0, 0.0])
        .unwrap();
    let _ = old
        .set_input(&mut draft, (scale, "amount"), Some("1.5"))
        .unwrap();
    let saved = draft.text().unwrap();
    assert!(!draft.opened().changed(), "a new document changes nothing");

    let (new, _events) = Host::with_services(HostOptions::default(), vec![version_2()]).unwrap();
    let mut opened = new.draft(&saved).unwrap();
    let report = opened.opened().clone();
    assert!(report.changed());
    assert_eq!(report.upgraded_from, None);
    assert_eq!(report.replaced.len(), 1);
    assert_eq!(report.replaced[0].node, scale);
    assert_eq!(
        report.replaced[0].to.id.as_str(),
        "org.example.math/multiply"
    );

    let node = opened.document().node(scale).unwrap();
    assert_eq!(node.definition.id.as_str(), "org.example.math/multiply");
    assert_eq!(
        node.inputs
            .keys()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["factor"]
    );
    assert_eq!(
        errors(&new.problems(&mut opened).unwrap()),
        Vec::<String>::new()
    );
    // Running and checking go through the same rewrite.
    assert_eq!(errors(&new.check(&saved).unwrap()), Vec::<String>::new());

    // The old host still has `scale`, so it never moves the node.
    let again = old.draft(&saved).unwrap();
    assert!(!again.opened().changed());
}
