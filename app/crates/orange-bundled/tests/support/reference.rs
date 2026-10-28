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
    body: orange_model::document::BodyRef,
    snapshot: &'a RegistrySnapshot,
    analysis: &'a ValidationAnalysis,
    cache: BTreeMap<NodeId, BTreeMap<PortKey, Outcome>>,
    seed: u64,
    sequence: u64,
    invocations: BTreeMap<NodeId, u64>,
    host: BTreeMap<(NodeId, PortKey), Outcome>,
    pub executions: BTreeMap<NodeId, usize>,
}
impl<'a> Reference<'a> {
    pub fn new(
        document: &'a GraphDocument,
        snapshot: &'a RegistrySnapshot,
        analysis: &'a ValidationAnalysis,
    ) -> Self {
        Self::seeded(document, snapshot, analysis, 0, 0)
    }
    pub fn seeded(
        document: &'a GraphDocument,
        snapshot: &'a RegistrySnapshot,
        analysis: &'a ValidationAnalysis,
        seed: u64,
        sequence: u64,
    ) -> Self {
        assert!(
            analysis.is_ready_for_lowering(document, snapshot),
            "{:?}",
            analysis.report().diagnostics
        );
        let mut reference = Self {
            document,
            body: orange_model::document::BodyRef::Root,
            snapshot,
            analysis,
            cache: BTreeMap::new(),
            seed,
            sequence,
            invocations: BTreeMap::new(),
            host: BTreeMap::new(),
            executions: BTreeMap::new(),
        };
        reference.begin_epoch();
        reference
    }
    // The oracle deliberately recomputes constants too. Production constant reuse
    // is checked separately by resolver/callback counters, not this slow evaluator.
    pub fn begin_epoch(&mut self) {
        self.cache.clear();
        self.host.clear();
    }
    #[allow(dead_code)] // Used by the independent control oracle, not pure-only suites.
    pub fn supply(&mut self, node: NodeId, port: PortKey, value: Outcome) {
        self.host.insert((node, port), value);
    }
    #[allow(dead_code)]
    pub fn source(&mut self, node: NodeId, port: &PortKey) -> Outcome {
        self.pull(node, port, 0)
    }
    pub fn exports(&mut self) -> Vec<Outcome> {
        (0..self.document.exports().len())
            .map(|index| self.export(index))
            .collect()
    }
    pub fn export(&mut self, index: usize) -> Outcome {
        let export = &self.document.exports()[index];
        self.pull(export.node, &export.port, 0)
    }
    fn pull(&mut self, node_id: NodeId, port_key: &PortKey, depth: usize) -> Outcome {
        assert!(depth <= 64, "test oracle depth exceeded");
        if let Some(value) = self.host.get(&(node_id, port_key.clone())) {
            return value.clone();
        }
        if let Some(outputs) = self.cache.get(&node_id) {
            return outputs[port_key].clone();
        }
        let document = self.document;
        let analysis = self.analysis;
        let node = document.node(node_id).unwrap();
        let signature = &analysis.report().resolved[&node_id];
        let mut inputs = Vec::new();
        let mut first_fault = None;
        for input in signature
            .ports
            .iter()
            .filter(|p| p.direction == PortDirection::Input)
        {
            assert!(input.variadic.is_none());
            if input.is_lazy() && node.definition.id.as_str() != "core/try" {
                if node.definition.id.as_str() == "core/coalesce" {
                    if !matches!(inputs.first(), Some(Value::None)) {
                        continue;
                    }
                } else {
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
            }
            let key = PortKey::new(input.port_id.clone(), None);
            let outcome = if let Some(edge) = document
                .body(self.body)
                .unwrap()
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
        if node.definition.id.as_str() == "core/call" && first_fault.is_none() {
            let target = node.config["function"].as_str().unwrap().parse().unwrap();
            let function = &document.functions()[&target];
            let mut child = Self::new(document, self.snapshot, analysis);
            child.body = orange_model::document::BodyRef::Function(target);
            if let Some(entry) = function
                .body
                .nodes
                .values()
                .find(|n| n.definition.id.as_str() == "core/function-entry")
            {
                for (parameter, value) in function.inputs.iter().zip(inputs) {
                    child.supply(
                        entry.id,
                        PortKey::new(parameter.port_id.clone(), None),
                        Ok(value),
                    );
                }
            }
            let result = function
                .body
                .nodes
                .values()
                .find(|n| n.definition.id.as_str() == "core/function-outputs")
                .unwrap();
            let mut outputs = BTreeMap::new();
            for output in &function.outputs {
                let key = PortKey::new(output.port_id.clone(), None);
                let value = if let Some(edge) = function
                    .body
                    .edges
                    .values()
                    .find(|e| e.to.node == result.id && e.to.port == key)
                {
                    child.pull(edge.from.node, &edge.from.port, depth + 1)
                } else {
                    Ok(literal_codec::decode(
                        result.inputs.get(&key).unwrap(),
                        self.snapshot.types().as_ref(),
                    )
                    .unwrap())
                };
                outputs.insert(key, value);
            }
            let first_fault = function.outputs.iter().find_map(|p| {
                outputs[&PortKey::new(p.port_id.clone(), None)]
                    .as_ref()
                    .err()
                    .cloned()
            });
            if let Some(fault) = first_fault {
                for value in outputs.values_mut() {
                    *value = Err(fault.clone());
                }
            }
            for (node, count) in child.executions {
                *self.executions.entry(node).or_default() += count;
            }
            let value = outputs[port_key].clone();
            self.cache.insert(node_id, outputs);
            return value;
        }
        let result = if let Some(fault) = first_fault {
            if node.definition.id.as_str() == "core/try" {
                Ok(vec![Value::Err(Arc::new(orange_model::value::NodeError {
                    code: fault.code.into(),
                    node: Some(fault.source),
                }))])
            } else {
                Err(fault)
            }
        } else {
            *self.executions.entry(node_id).or_default() += 1;
            let definition = node.definition.id.as_str();
            let values = if definition == "acme.frame/seeded-value"
                && node.config["fault"].as_bool() == Some(true)
            {
                Err("core/Domain")
            } else if definition == orange_bundled::RANDOM_U64_DEFINITION
                || definition == "acme.frame/seeded-value"
            {
                // Share only the versioned stream primitive (anchored by external
                // vectors), never dense scopes, compiler plans or native callbacks.
                let invocation = self.invocations.entry(node_id).or_default();
                let mut stream = orange_runtime::EntropyStream::new(orange_runtime::EntropyKey {
                    seed: self.seed,
                    frame: orange_runtime::FrameId::ROOT,
                    node: node_id,
                    activation_sequence: self.sequence,
                    invocation: *invocation,
                });
                *invocation = invocation
                    .checked_add(1)
                    .expect("bounded oracle invocations");
                let word = stream.next_u64().unwrap();
                let ty = signature.ports[0].ty.as_ref().unwrap().as_str();
                let value = match ty {
                    "core/u64@1" => Value::U64(word),
                    "core/bool@1" => Value::Bool(word % 2 == 1),
                    "core/option@1<core/u64@1>" if word % 2 == 1 => Value::None,
                    "core/option@1<core/u64@1>" => Value::U64(word),
                    "core/option@1<core/option@1<core/u64@1>>" if word.is_multiple_of(2) => {
                        Value::None
                    }
                    "core/option@1<core/option@1<core/u64@1>>" => {
                        Value::Some(Arc::new(if (word / 2).is_multiple_of(2) {
                            Value::None
                        } else {
                            Value::U64(word)
                        }))
                    }
                    "core/f64@1" => Value::F64(f64::from_bits(word)),
                    _ => panic!("seeded value outside oracle vocabulary"),
                };
                Ok(vec![value])
            } else {
                interpret(node, &inputs, signature)
            };
            values.map_err(|code| Fault {
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

fn interpret(
    node: &NodeInstance,
    inputs: &[Value],
    signature: &orange_model::document::SignatureSnapshot,
) -> Result<Vec<Value>, &'static str> {
    match node.definition.id.as_str() {
        "core/try" => {
            let [value] = inputs else {
                panic!("oracle try contract")
            };
            let ty = term(signature, 0);
            let success = Tagged::Ok(Box::new(Tagged::decode(&ty, value)));
            return Ok(vec![success.encode(&result_of(ty))]);
        }
        "core/coalesce" => {
            let option = term(signature, 0);
            let value = match (Tagged::decode(&option, &inputs[0]), inputs) {
                (Tagged::None, [_, fallback]) => fallback.clone(),
                (Tagged::Some(present), [_]) => present.encode(&option.core("option").unwrap()[0]),
                _ => panic!("oracle coalesce contract"),
            };
            return Ok(vec![value]);
        }
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

fn term(
    signature: &orange_model::document::SignatureSnapshot,
    port: usize,
) -> orange_model::type_term::TypeTerm {
    orange_model::type_term::TypeTerm::parse(signature.ports[port].ty.as_ref().unwrap()).unwrap()
}

fn result_of(ty: orange_model::type_term::TypeTerm) -> orange_model::type_term::TypeTerm {
    orange_model::type_term::TypeTerm::parse(&format!("core/result@1<{ty}>").parse().unwrap())
        .unwrap()
}

/// Fully tagged semantic value. The oracle reasons about Option/Result here and only
/// meets the compact runtime representation through `decode`/`encode`.
#[derive(Debug, Clone, PartialEq)]
enum Tagged {
    None,
    Some(Box<Tagged>),
    Ok(Box<Tagged>),
    Err(Value),
    Leaf(Value),
}

/// Top-level runtime discriminants a compact value of `ty` can show, as bits.
const NONE: u8 = 1;
const ERR: u8 = 2;
const WRAPPED: u8 = 4;
fn bare_tags(ty: &orange_model::type_term::TypeTerm) -> u8 {
    if let Some([inner]) = ty.core("option") {
        // A wrapper is needed only when the payload could already look absent.
        let inner = bare_tags(inner);
        return if inner & NONE != 0 {
            NONE | WRAPPED
        } else {
            inner | NONE
        };
    }
    if let Some([inner]) = ty.core("result") {
        let inner = bare_tags(inner);
        return if inner & ERR != 0 {
            ERR | WRAPPED
        } else {
            inner | ERR
        };
    }
    if ty.core("error").is_some() {
        ERR
    } else {
        WRAPPED
    }
}

impl Tagged {
    fn decode(ty: &orange_model::type_term::TypeTerm, value: &Value) -> Self {
        if let Some([inner]) = ty.core("option") {
            return match value {
                Value::None => Self::None,
                Value::Some(boxed) if bare_tags(inner) & NONE != 0 => {
                    Self::Some(Box::new(Self::decode(inner, boxed)))
                }
                _ if bare_tags(inner) & NONE != 0 => panic!("oracle: unboxed nested option"),
                value => Self::Some(Box::new(Self::decode(inner, value))),
            };
        }
        if let Some([inner]) = ty.core("result") {
            return match value {
                Value::Err(_) => Self::Err(value.clone()),
                Value::Ok(boxed) if bare_tags(inner) & ERR != 0 => {
                    Self::Ok(Box::new(Self::decode(inner, boxed)))
                }
                _ if bare_tags(inner) & ERR != 0 => panic!("oracle: unboxed nested result"),
                value => Self::Ok(Box::new(Self::decode(inner, value))),
            };
        }
        Self::Leaf(value.clone())
    }

    fn encode(&self, ty: &orange_model::type_term::TypeTerm) -> Value {
        let wrap = |inner: &orange_model::type_term::TypeTerm, tag: u8, v: &Self| {
            let value = v.encode(inner);
            if bare_tags(inner) & tag != 0 {
                if tag == NONE {
                    Value::Some(Arc::new(value))
                } else {
                    Value::Ok(Arc::new(value))
                }
            } else {
                value
            }
        };
        match (self, ty.core("option"), ty.core("result")) {
            (Self::None, Some(_), _) => Value::None,
            (Self::Some(v), Some([inner]), _) => wrap(inner, NONE, v),
            (Self::Err(e), _, Some(_)) => e.clone(),
            (Self::Ok(v), _, Some([inner])) => wrap(inner, ERR, v),
            (Self::Leaf(v), None, None) => v.clone(),
            _ => panic!("oracle: value does not match {ty}"),
        }
    }
}
