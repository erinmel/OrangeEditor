use serde_json::{Value, json};

pub fn id(n: u32) -> String {
    format!("10000000-0000-4000-8000-{n:012x}")
}

pub fn core(n: u32, name: &str) -> Value {
    json!({"id":id(n),"definition_id":format!("core/{name}"),"definition_version":1,"config_version":1,"config":{}})
}

pub fn edge(n: u32, from: u32, output: &str, to: u32, input: &str) -> Value {
    json!({"id":format!("30000000-0000-4000-8000-{n:012x}"),"from":{"node":id(from),"port":output},"to":{"node":id(to),"port":input}})
}
