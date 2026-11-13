//! What a graph's nodes and variables are called, so the terminal can show and accept
//! names instead of IDs.
//!
//! A node is called by its own `name` field when the graph gives it one, otherwise by
//! what its kind is called ("Write text file"). When two nodes would be called the
//! same, each gets the end of its ID added. A variable is called by its declared name.
use crate::*;
use orange_host::{Value, VariableId};
use std::collections::BTreeMap;

/// The names of one graph.
#[derive(Debug, Default)]
pub(crate) struct Names {
    nodes: BTreeMap<String, String>,
    /// Each variable's ID with its name and declared type.
    variables: BTreeMap<String, (String, String)>,
}

/// One open flow: its number in the host, the file it came from and its names.
#[derive(Debug)]
pub(crate) struct Opened {
    pub flow: FlowId,
    pub path: String,
    pub names: Names,
}

fn tail(id: &str) -> &str {
    let end = id.trim_start_matches(|c: char| c == '0' || c == '-' || !c.is_ascii_hexdigit());
    // The part of an ID that tells it from its neighbours: its last digits.
    let short = id.rsplit('-').next().unwrap_or(id).trim_start_matches('0');
    if short.is_empty() { end } else { short }
}

impl Names {
    /// Reads the names of a graph. Text that is not a graph has none.
    pub(crate) fn of(host: &Host, json: &str) -> Self {
        let Ok(graph) = serde_json::from_str::<serde_json::Value>(json) else {
            return Self::default();
        };
        let labels: BTreeMap<String, String> = host
            .nodes()
            .map(|info| {
                (
                    info.definition.definition.id.as_str().to_owned(),
                    info.label().into_owned(),
                )
            })
            .collect();
        let list = |key: &str| graph[key].as_array().cloned().unwrap_or_default();
        let mut nodes: Vec<(String, String)> = list("nodes")
            .iter()
            .filter_map(|node| {
                let id = node["id"].as_str()?.to_owned();
                let kind = node["definition_id"].as_str().unwrap_or_default();
                let name = node["name"]
                    .as_str()
                    .map(str::to_owned)
                    .or_else(|| labels.get(kind).cloned())
                    .unwrap_or_else(|| kind.to_owned());
                Some((id, name))
            })
            .collect();
        // Nodes that would be called the same are told apart by the end of their ID.
        let mut count: BTreeMap<String, usize> = BTreeMap::new();
        for (_, name) in &nodes {
            *count.entry(name.to_lowercase()).or_default() += 1;
        }
        for (id, name) in &mut nodes {
            if count[&name.to_lowercase()] > 1 {
                *name = format!("{name} #{}", tail(id));
            }
        }
        let variables = list("variables")
            .iter()
            .filter_map(|variable| {
                let text = |key: &str| variable[key].as_str().map(str::to_owned);
                Some((text("id")?, (text("name")?, text("type")?)))
            })
            .collect();
        Self {
            nodes: nodes.into_iter().collect(),
            variables,
        }
    }

    /// What a node is called; its ID when the graph does not have it.
    pub(crate) fn node(&self, id: &NodeId) -> String {
        let id = id.to_string();
        self.nodes.get(&id).cloned().unwrap_or(id)
    }

    /// What a variable is called.
    pub(crate) fn variable(&self, id: &VariableId) -> String {
        let id = id.to_string();
        self.variables.get(&id).map_or(id, |(name, _)| name.clone())
    }

    fn find<'a>(
        what: &str,
        word: &str,
        candidates: impl Iterator<Item = (&'a String, &'a str)>,
    ) -> Result<String, String> {
        let mut named = Vec::new();
        for (id, name) in candidates {
            if id == word {
                return Ok(id.clone());
            }
            if name.eq_ignore_ascii_case(word) {
                named.push(id.clone());
            }
        }
        match named.len() {
            1 => Ok(named.remove(0)),
            0 => Err(format!("no {what} is called `{word}`")),
            _ => Err(format!("several {what}s are called `{word}`; use the ID")),
        }
    }

    /// The node a word typed by the user means: an ID, or a name in any letter case.
    pub(crate) fn find_node(&self, word: &str) -> Result<NodeId, String> {
        let id = Self::find(
            "node",
            word,
            self.nodes.iter().map(|(id, name)| (id, name.as_str())),
        )?;
        id.parse().map_err(|_| format!("`{id}` is not a node ID"))
    }

    /// The variable a word means, with its declared type.
    pub(crate) fn find_variable(&self, word: &str) -> Result<(VariableId, String), String> {
        let id = Self::find(
            "variable",
            word,
            self.variables
                .iter()
                .map(|(id, (name, _))| (id, name.as_str())),
        )?;
        let ty = self.variables[&id].1.clone();
        let id = id
            .parse()
            .map_err(|_| format!("`{id}` is not a variable ID"))?;
        Ok((id, ty))
    }
}

/// A value as a person reads it: `7`, `"text"`, `true`, `<12 bytes>`. Long text is cut.
pub(crate) fn plain(value: &Value) -> String {
    match value {
        Value::Bool(v) => v.to_string(),
        Value::I32(v) => v.to_string(),
        Value::U32(v) => v.to_string(),
        Value::I64(v) => v.to_string(),
        Value::U64(v) => v.to_string(),
        Value::F32(v) => v.to_string(),
        Value::F64(v) => v.to_string(),
        Value::Str(text) => {
            let mut shown: String = text.chars().take(120).collect();
            if shown.len() < text.len() {
                shown.push('…');
            }
            format!("{shown:?}")
        }
        Value::Bytes(bytes) => format!("<{} bytes>", bytes.len()),
        other => format!("{other:?}"),
    }
}

/// The literal text for a value typed plainly (`7`, `hello there`, `true`, `0.5`) for a
/// variable of type `ty`. Text that is already a literal object is passed through.
pub(crate) fn literal(ty: &str, typed: &str) -> Result<String, String> {
    let typed = typed.trim();
    if typed.starts_with('{') {
        return Ok(typed.to_owned());
    }
    let number = |what: &str| format!("`{typed}` is not {what}");
    let value = match ty {
        "core/string@1" => serde_json::json!({"type": ty, "value": typed}),
        "core/bool@1" => {
            let value: bool = typed.parse().map_err(|_| number("true or false"))?;
            serde_json::json!({"type": ty, "value": value})
        }
        "core/i64@1" | "core/u64@1" => {
            let _: i128 = typed.parse().map_err(|_| number("a whole number"))?;
            serde_json::json!({"type": ty, "value": typed})
        }
        "core/f64@1" => {
            let value: f64 = typed.parse().map_err(|_| number("a number"))?;
            serde_json::json!({"type": ty, "bits": format!("{:016x}", value.to_bits())})
        }
        _ => {
            return Err(format!(
                "a variable of type {ty} needs a literal such as {{\"type\":\"{ty}\",\"value\":…}}"
            ));
        }
    };
    Ok(value.to_string())
}

/// One diagnostic as a line: what is wrong and where, by name.
pub(crate) fn describe(diagnostic: &orange_host::Diagnostic, names: &Names) -> String {
    let subject = &diagnostic.subject;
    let mut place = Vec::new();
    if let Some(node) = &subject.node {
        place.push(format!("node {}", names.node(node)));
    }
    if let Some(port) = &subject.port {
        place.push(format!("port {port}"));
    }
    if let Some(variable) = &subject.variable {
        place.push(format!("variable {}", names.variable(variable)));
    }
    let place = if place.is_empty() {
        String::new()
    } else {
        format!(" ({})", place.join(", "))
    };
    format!("{:?}: {}{place}", diagnostic.code, diagnostic.message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use orange_host::HostOptions;

    #[test]
    fn nodes_and_variables_are_found_by_name_and_same_names_are_told_apart() {
        let (host, _events) = Host::new(HostOptions::default()).unwrap();
        let graph = serde_json::json!({
            "nodes": [
                {"id": "10000000-0000-4000-8000-000000000001", "name": "Save Greeting",
                    "definition_id": "core/on-manual"},
                {"id": "10000000-0000-4000-8000-000000000002", "definition_id": "core/set-variable"},
                {"id": "10000000-0000-4000-8000-00000000000a", "definition_id": "core/set-variable"}
            ],
            "variables": [{"id": "10000000-0000-4000-8000-000000000064", "name": "total",
                "type": "core/i64@1"}]
        })
        .to_string();
        let names = Names::of(&host, &graph);
        let first: NodeId = "10000000-0000-4000-8000-000000000001".parse().unwrap();
        assert_eq!(names.node(&first), "Save Greeting");
        assert_eq!(names.find_node("save greeting"), Ok(first));
        assert_eq!(
            names.find_node("10000000-0000-4000-8000-000000000001"),
            Ok(first)
        );
        // Two nodes of the same kind are called by the kind and the end of their ID.
        let second: NodeId = "10000000-0000-4000-8000-000000000002".parse().unwrap();
        assert_eq!(names.node(&second), "Set variable #2");
        assert_eq!(
            names.find_node("set variable #a").map(|id| id.to_string()),
            Ok("10000000-0000-4000-8000-00000000000a".to_owned())
        );
        assert!(names.find_node("set variable").is_err());
        assert!(names.find_node("nobody").is_err());
        let (variable, ty) = names.find_variable("Total").unwrap();
        assert_eq!(
            (names.variable(&variable).as_str(), ty.as_str()),
            ("total", "core/i64@1")
        );
        // A node the graph does not have is shown by its ID.
        let unknown: NodeId = "10000000-0000-4000-8000-0000000000ff".parse().unwrap();
        assert_eq!(names.node(&unknown), unknown.to_string());
    }

    #[test]
    fn plainly_typed_values_become_literals_of_the_variables_type() {
        let value = |ty: &str, typed: &str| {
            literal(ty, typed).map(|text| serde_json::from_str::<serde_json::Value>(&text).unwrap())
        };
        assert_eq!(
            value("core/string@1", "hello there").unwrap()["value"],
            "hello there"
        );
        assert_eq!(value("core/i64@1", "-7").unwrap()["value"], "-7");
        assert_eq!(value("core/bool@1", "true").unwrap()["value"], true);
        assert_eq!(
            value("core/f64@1", "0.15").unwrap()["bits"],
            "3fc3333333333333"
        );
        assert!(value("core/i64@1", "seven").is_err());
        assert!(value("core/f64@1", "loud").is_err());
        assert!(value("core/bytes@1", "abc").is_err());
        // A literal written in full is passed through for any type.
        assert_eq!(
            literal(
                "core/bytes@1",
                r#" {"type":"core/bytes@1","value":"AA=="} "#
            )
            .unwrap(),
            r#"{"type":"core/bytes@1","value":"AA=="}"#
        );
        assert_eq!(plain(&Value::I64(7)), "7");
        assert_eq!(
            plain(&Value::Str(std::sync::Arc::new("hi".into()))),
            "\"hi\""
        );
        assert_eq!(
            plain(&Value::Bytes(std::sync::Arc::new(vec![1, 2]))),
            "<2 bytes>"
        );
    }
}
