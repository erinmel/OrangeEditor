//! Deliberately slow test oracle: demand graph ports, scan edges, memoize by source UUID.
//! No compiler plan/slots, resolver, native kernel or compiler folding helpers are used.
//! Bounded to 64 dependency levels; generated fixtures stay below 25 nodes.
use std::collections::BTreeMap;

use orange_model::catalog::RegistrySnapshot;
use orange_model::document::{NodeInstance, PortDirection};
use orange_model::id::{NodeId, PortKey};
use orange_model::validation::ValidationAnalysis;
use orange_model::value::JsonText;
use orange_model::value::literal_codec;
use orange_model::{GraphDocument, Value};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub struct Fault {
    pub source: NodeId,
    pub code: &'static str,
}
pub type Outcome = Result<Value, Fault>;

pub struct Reference<'a> {
    document: &'a GraphDocument,
    snapshot: &'a RegistrySnapshot,
    analysis: &'a ValidationAnalysis,
    cache: BTreeMap<NodeId, BTreeMap<PortKey, Outcome>>,
    pub executions: BTreeMap<NodeId, usize>,
}
impl<'a> Reference<'a> {
    pub fn new(
        document: &'a GraphDocument,
        snapshot: &'a RegistrySnapshot,
        analysis: &'a ValidationAnalysis,
    ) -> Self {
        assert!(analysis.is_ready_for_lowering(document, snapshot));
        Self {
            document,
            snapshot,
            analysis,
            cache: BTreeMap::new(),
            executions: BTreeMap::new(),
        }
    }
    pub fn exports(&mut self) -> Vec<Outcome> {
        self.document
            .exports()
            .iter()
            .map(|export| self.pull(export.node, &export.port, 0))
            .collect()
    }
    fn pull(&mut self, node_id: NodeId, port_key: &PortKey, depth: usize) -> Outcome {
        assert!(depth <= 64, "test oracle depth exceeded");
        if let Some(outputs) = self.cache.get(&node_id) {
            return outputs[port_key].clone();
        }
        let document = self.document;
        let analysis = self.analysis;
        let node = &document.root().nodes[&node_id];
        let signature = &analysis.report().resolved[&node_id];
        let mut inputs = Vec::new();
        let mut first_fault = None;
        for input in signature
            .ports
            .iter()
            .filter(|p| p.direction == PortDirection::Input)
        {
            assert!(input.variadic.is_none());
            if input.is_lazy() {
                let Some(Value::Bool(deciding)) = inputs.first() else {
                    panic!("oracle selector type")
                };
                let selected = match node.definition.id.as_str() {
                    "core/select" => {
                        if *deciding {
                            "if_true"
                        } else {
                            "if_false"
                        }
                    }
                    "core/and" if *deciding => "b",
                    "core/or" if !*deciding => "b",
                    "core/and" | "core/or" => continue,
                    _ => panic!("lazy form outside oracle vocabulary"),
                };
                if input.port_id.as_str() != selected {
                    continue;
                }
            }
            let key = PortKey::new(input.port_id.clone(), None);
            let outcome = if let Some(edge) = document
                .root()
                .edges
                .values()
                .find(|edge| edge.to.node == node_id && edge.to.port == key)
            {
                self.pull(edge.from.node, &edge.from.port, depth + 1)
            } else {
                let literal = node
                    .inputs
                    .get(&key)
                    .or(input.default.as_ref())
                    .expect("initialized oracle input");
                Ok(literal_codec::decode(literal, self.snapshot.types().as_ref()).unwrap())
            };
            match outcome {
                Ok(value) => inputs.push(value),
                Err(fault) => {
                    first_fault = Some(fault);
                    break;
                }
            }
        }
        let result = if let Some(fault) = first_fault {
            Err(fault)
        } else {
            *self.executions.entry(node_id).or_default() += 1;
            interpret(node, &inputs).map_err(|code| Fault {
                source: node_id,
                code,
            })
        };
        let ports: Vec<_> = signature
            .ports
            .iter()
            .filter(|p| p.direction == PortDirection::Output)
            .map(|p| PortKey::new(p.port_id.clone(), None))
            .collect();
        let outputs = match result {
            Ok(values) => {
                assert_eq!(ports.len(), values.len(), "oracle output contract");
                ports.into_iter().zip(values.into_iter().map(Ok)).collect()
            }
            Err(fault) => ports
                .into_iter()
                .map(|port| (port, Err(fault.clone())))
                .collect(),
        };
        let outputs: BTreeMap<PortKey, Outcome> = outputs;
        let result = outputs[port_key].clone();
        self.cache.insert(node_id, outputs);
        result
    }
}

fn interpret(node: &NodeInstance, inputs: &[Value]) -> Result<Vec<Value>, &'static str> {
    match node.definition.id.as_str() {
        "core/select" => {
            let [Value::Bool(_), value] = inputs else {
                panic!("oracle select contract")
            };
            return Ok(vec![value.clone()]);
        }
        "core/and" | "core/or" => {
            let value = match (node.definition.id.as_str(), inputs) {
                (_, [Value::Bool(value)]) => *value,
                ("core/and", [Value::Bool(a), Value::Bool(b)]) => *a && *b,
                ("core/or", [Value::Bool(a), Value::Bool(b)]) => *a || *b,
                _ => panic!("oracle short circuit contract"),
            };
            return Ok(vec![Value::Bool(value)]);
        }
        _ => {}
    }
    if node.definition.id.as_str() == "core/convert" {
        return conversion(node, inputs);
    }
    arithmetic(node, inputs).map(|value| vec![value])
}

fn arithmetic(node: &NodeInstance, inputs: &[Value]) -> Result<Value, &'static str> {
    if node.definition.id.as_str() == "org.orange.math/not" {
        let [Value::Bool(value)] = inputs else {
            panic!("oracle bool contract")
        };
        return Ok(Value::Bool(!value));
    }
    let operation = node
        .definition
        .id
        .as_str()
        .strip_prefix("org.orange.math/")
        .unwrap();
    let mode = node
        .config
        .get("int_overflow")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("checked");
    let float_policy = node
        .config
        .get("float_policy")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("finite");
    match inputs {
        [Value::I32(a), Value::I32(b)] => {
            signed(i128::from(*a), i128::from(*b), 32, operation, mode)
                .map(|v| Value::I32(v as i32))
        }
        [Value::I64(a), Value::I64(b)] => {
            signed(i128::from(*a), i128::from(*b), 64, operation, mode)
                .map(|v| Value::I64(v as i64))
        }
        [Value::U32(a), Value::U32(b)] => {
            unsigned(u128::from(*a), u128::from(*b), 32, operation, mode)
                .map(|v| Value::U32(v as u32))
        }
        [Value::U64(a), Value::U64(b)] => {
            unsigned(u128::from(*a), u128::from(*b), 64, operation, mode)
                .map(|v| Value::U64(v as u64))
        }
        [Value::F32(a), Value::F32(b)] => float32(*a, *b, operation, float_policy).map(Value::F32),
        [Value::F64(a), Value::F64(b)] => float64(*a, *b, operation, float_policy).map(Value::F64),
        _ => panic!("arithmetic outside oracle vocabulary"),
    }
}

// Same platform floating primitives, but independent graph traversal and explicit policy
// handling. These do not constitute an independent IEEE implementation or cross-OS proof.
macro_rules! reference_float {
    ($name:ident,$ty:ty) => {
        fn $name(a: $ty, b: $ty, op: &str, policy: &str) -> Result<$ty, &'static str> {
            assert!(matches!(policy, "finite" | "ieee"));
            if policy == "finite" && !(a.is_finite() && b.is_finite()) {
                return Err("core/NonFinite");
            }
            let value = match op {
                "add" => a + b,
                "subtract" => a - b,
                "multiply" => a * b,
                "divide" if policy == "finite" && b == 0.0 => return Err("core/DivideByZero"),
                "divide" => a / b,
                _ => panic!("unknown float operation"),
            };
            if policy == "finite" && !value.is_finite() {
                Err("core/NonFinite")
            } else {
                Ok(value)
            }
        }
    };
}
reference_float!(float32, f32);
reference_float!(float64, f64);

fn conversion(node: &NodeInstance, inputs: &[Value]) -> Result<Vec<Value>, &'static str> {
    match (node.config["converter"].as_str().unwrap(), inputs) {
        ("core/i32-to-i64", [Value::I32(value)]) => Ok(vec![Value::I64(i64::from(*value))]),
        ("core/i64-to-i32", [Value::I64(value)]) => i32::try_from(*value)
            .map(|v| vec![Value::I32(v)])
            .map_err(|_| "core/OutOfRange"),
        ("core/f32-to-f64", [Value::F32(value)]) => Ok(vec![Value::F64(widen(*value))]),
        ("core/f64-to-f32", [Value::F64(value)]) => {
            if value.is_finite() && value.abs() > f64::from(f32::MAX) {
                return Err("core/OutOfRange");
            }
            let narrowed = if value.is_nan() {
                let fraction = value.to_bits() % (1_u64 << 52);
                let retained = u32::try_from(fraction / (1_u64 << 29)).unwrap();
                let sign = if value.is_sign_negative() {
                    1_u32 << 31
                } else {
                    0
                };
                f32::from_bits(sign + (255_u32 << 23) + retained.max(1))
            } else {
                *value as f32
            };
            let lost = widen(narrowed).to_bits() != value.to_bits();
            let text = serde_json::json!({"lost":lost}).to_string();
            Ok(vec![
                Value::F32(narrowed),
                Value::Json(Arc::new(JsonText::new(text).unwrap())),
            ])
        }
        _ => panic!("conversion outside oracle vocabulary"),
    }
}
fn widen(value: f32) -> f64 {
    if value.is_nan() {
        let fraction = u64::from(value.to_bits() % (1_u32 << 23));
        let sign = if value.is_sign_negative() {
            1_u64 << 63
        } else {
            0
        };
        f64::from_bits(sign + (2047_u64 << 52) + fraction * (1_u64 << 29))
    } else {
        f64::from(value)
    }
}

// Wider mathematical intermediates deliberately differ from the native kernels'
// primitive-width checked/wrapping/saturating intrinsics. All 64-bit products fit.
fn signed(a: i128, b: i128, bits: u32, operation: &str, mode: &str) -> Result<i128, &'static str> {
    let raw = match operation {
        "add" => a + b,
        "subtract" => a - b,
        "multiply" => a * b,
        "divide" if b == 0 => return Err("core/DivideByZero"),
        "divide" => a / b,
        _ => panic!("unknown oracle operation"),
    };
    let min = -(1_i128 << (bits - 1));
    let max = (1_i128 << (bits - 1)) - 1;
    match mode {
        "checked" if !(min..=max).contains(&raw) => Err("core/NumericOverflow"),
        "checked" => Ok(raw),
        "saturating" => Ok(raw.clamp(min, max)),
        "wrapping" => {
            let modulus = 1_i128 << bits;
            Ok((raw - min).rem_euclid(modulus) + min)
        }
        _ => panic!("unknown oracle overflow policy"),
    }
}
fn unsigned(
    a: u128,
    b: u128,
    bits: u32,
    operation: &str,
    mode: &str,
) -> Result<u128, &'static str> {
    let max = (1_u128 << bits) - 1;
    if operation == "subtract" && a < b {
        return match mode {
            "checked" => Err("core/NumericOverflow"),
            "saturating" => Ok(0),
            "wrapping" => Ok((1_u128 << bits) - (b - a)),
            _ => panic!("unknown oracle policy"),
        };
    }
    let raw = match operation {
        "add" => a + b,
        "subtract" => a - b,
        "multiply" => a * b,
        "divide" if b == 0 => return Err("core/DivideByZero"),
        "divide" => a / b,
        _ => panic!("unknown oracle operation"),
    };
    match mode {
        "checked" if raw > max => Err("core/NumericOverflow"),
        "checked" => Ok(raw),
        "saturating" => Ok(raw.min(max)),
        "wrapping" => Ok(raw & max),
        _ => panic!("unknown oracle policy"),
    }
}
