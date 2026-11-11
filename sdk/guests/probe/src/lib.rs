//! Test guest for `orange:expansion/plugin@1.0.0` (K02b). Its nodes exercise literal and
//! binary values, guest-reported errors, the granted `log` import, traps and stateful
//! effects.

wit_bindgen::generate!({
    world: "plugin",
    path: "../../wit",
});

use exports::orange::expansion::{effects, nodes};
use orange::expansion::types::{NodeError, Value};
use orange::expansion::{host, log};
use serde_json::{Value as Json, json};
use std::sync::atomic::{AtomicI64, Ordering};

struct Probe;

/// State kept between `execute` calls on one instance.
static TOTAL: AtomicI64 = AtomicI64::new(0);

fn error(code: &str) -> NodeError {
    NodeError {
        code: code.into(),
        message_key: "org.example.probe/failed".into(),
    }
}

/// The integer payload of an `orange.value.v1` i64 literal (`"value"` is decimal text).
fn int(value: &Value) -> Result<i64, NodeError> {
    let Value::Literal(literal) = value else {
        return Err(error("org.example.probe/BadInput"));
    };
    let parsed: Json =
        serde_json::from_str(literal).map_err(|_| error("org.example.probe/BadInput"))?;
    if parsed["type"] != "core/i64@1" {
        return Err(error("org.example.probe/BadInput"));
    }
    parsed["value"]
        .as_str()
        .and_then(|text| text.parse().ok())
        .ok_or_else(|| error("org.example.probe/BadInput"))
}

fn i64_literal(value: i64) -> Value {
    Value::Literal(json!({"type": "core/i64@1", "value": value.to_string()}).to_string())
}

impl nodes::Guest for Probe {
    fn evaluate(definition: String, inputs: Vec<Value>) -> Result<Vec<Value>, NodeError> {
        match definition.as_str() {
            "org.example.probe/add" => {
                let [a, b] = inputs.as_slice() else {
                    return Err(error("org.example.probe/Arity"));
                };
                let sum = int(a)?
                    .checked_add(int(b)?)
                    .ok_or_else(|| error("core/NumericOverflow"))?;
                Ok(vec![i64_literal(sum)])
            }
            "org.example.probe/echo-logged" => {
                log::write(&format!("echo {} inputs", inputs.len()));
                Ok(inputs)
            }
            "org.example.probe/api-major" => {
                let (major, _) = host::api_version();
                Ok(vec![i64_literal(i64::from(major))])
            }
            // Bulk data arrives and leaves in its binary case, never as text.
            "org.example.probe/reverse-bytes" => match inputs.as_slice() {
                [Value::Bytes(bytes)] => {
                    Ok(vec![Value::Bytes(bytes.iter().rev().copied().collect())])
                }
                _ => Err(error("org.example.probe/BadInput")),
            },
            "org.example.probe/scale-samples" => match inputs.as_slice() {
                [Value::F32List(samples)] => Ok(vec![Value::F32List(
                    samples.iter().map(|sample| sample * 2.0).collect(),
                )]),
                _ => Err(error("org.example.probe/BadInput")),
            },
            "org.example.probe/byte-count" => match inputs.as_slice() {
                [Value::Bytes(bytes)] => Ok(vec![i64_literal(bytes.len() as i64)]),
                _ => Err(error("org.example.probe/BadInput")),
            },
            // A binary case for a type it does not belong to.
            "org.example.probe/wrong-binary" => Ok(vec![Value::F64List(vec![1.0])]),
            "org.example.probe/wrong-type" => Ok(vec![Value::Literal(
                json!({"type": "core/string@1", "value": "x"}).to_string(),
            )]),
            "org.example.probe/bad-error" => Err(NodeError {
                code: "no namespace".into(),
                message_key: "also bad".into(),
            }),
            "org.example.probe/trap" => panic!("probe trap"),
            "org.example.probe/spin" => loop {
                std::hint::black_box(());
            },
            _ => Err(error("org.example.probe/UnknownDefinition")),
        }
    }
}

impl effects::Guest for Probe {
    fn execute(definition: String, inputs: Vec<Value>) -> Result<Vec<Value>, NodeError> {
        match definition.as_str() {
            // Adds its input to a total that lives as long as the instance.
            "org.example.probe/accumulate" => {
                let [amount] = inputs.as_slice() else {
                    return Err(error("org.example.probe/Arity"));
                };
                let total = TOTAL.fetch_add(int(amount)?, Ordering::Relaxed) + int(amount)?;
                Ok(vec![i64_literal(total)])
            }
            "org.example.probe/refuse" => Err(error("org.example.probe/Refused")),
            _ => Err(error("org.example.probe/UnknownDefinition")),
        }
    }
}

export!(Probe);
