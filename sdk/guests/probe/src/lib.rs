//! Test guest for `orange:expansion/plugin@1.0.0` (K02b). Its nodes exercise values,
//! guest-reported errors, the granted `log` import and traps.

wit_bindgen::generate!({
    world: "plugin",
    path: "../../wit",
});

use exports::orange::expansion::nodes::{Guest, NodeError, Value};
use orange::expansion::{host, log};
use serde_json::{Value as Json, json};

struct Probe;

fn error(code: &str) -> NodeError {
    NodeError {
        code: code.into(),
        message_key: "org.example.probe/failed".into(),
    }
}

/// The integer payload of an `orange.value.v1` i64 literal (`"value"` is decimal text).
fn int(literal: &Value) -> Result<i64, NodeError> {
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
    json!({"type": "core/i64@1", "value": value.to_string()}).to_string()
}

impl Guest for Probe {
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
            "org.example.probe/wrong-type" => Ok(vec![
                json!({"type": "core/string@1", "value": "x"}).to_string(),
            ]),
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

export!(Probe);
