//! Live preview: the values of pure outputs, computed without running the graph.
use super::*;
use orange_compiler::{EvaluationOptions, PreviewLimits, PurePreview};

/// The engine that folds pure outputs, with the registry generation it was made for.
pub(crate) type Engine = (u64, PurePreview<WithGuests<Native>>);

/// One output of a preview.
#[derive(Debug, Clone, PartialEq)]
pub struct Previewed {
    pub node: NodeId,
    pub port: PortKey,
    /// The value, or the fault computing it gives with the node that fails.
    pub value: Result<Value, (NodeId, KernelFault)>,
}

impl Host {
    /// Computes outputs of a document without running it: what an editor shows beside
    /// a node while the graph is being built.
    ///
    /// `ports` are the outputs wanted; when empty, the document's own `exports` are
    /// computed. Only what follows from constants and pure nodes can be previewed: an
    /// output that depends on a variable, a trigger's payload or a node with an effect
    /// makes the document invalid for preview, reported as [`HostError::Invalid`].
    /// Nothing is executed, no connection is opened and no running flow is touched.
    /// Results that an edit did not change are reused from the previous call.
    pub fn preview(
        &mut self,
        json: &str,
        ports: &[(NodeId, PortKey)],
    ) -> Result<Vec<Previewed>, HostError> {
        let refused = |error: &dyn std::fmt::Debug| HostError::Document(format!("{error:?}"));
        let document = if ports.is_empty() {
            self.linked(json)?
        } else {
            let mut saved: serde_json::Value = serde_json::from_str(json)
                .map_err(|error| HostError::Document(error.to_string()))?;
            saved["exports"] = ports
                .iter()
                .enumerate()
                .map(|(index, (node, port))| {
                    serde_json::json!({"id": format!("preview-{index}"),
                        "node": node.to_string(), "port": port.to_string()})
                })
                .collect();
            self.linked(&saved.to_string())?
        };
        let wanted: Vec<(NodeId, PortKey)> = document
            .exports()
            .iter()
            .map(|export| (export.node, export.port.clone()))
            .collect();
        let registry = Arc::clone(self.store.current());
        let analysis = analyzed(&document, &registry);
        if !analysis.is_ready_for_lowering(&document, &registry) {
            return Err(HostError::Invalid(analysis.report().diagnostics.clone()));
        }
        // Providers are fixed per registry generation, so the engine and what it has
        // already computed are kept until packages change.
        let generation = registry.generation();
        if self
            .preview
            .as_ref()
            .is_none_or(|(made, _)| *made != generation)
        {
            let mut resolver = WithGuests::new(self.native.clone());
            for guest in &self.guests {
                resolver.add(guest.clone());
            }
            self.preview = Some((
                generation,
                PurePreview::new(resolver, PreviewLimits::default()),
            ));
        }
        let Some((_, engine)) = self.preview.as_mut() else {
            unreachable!("the engine was made above")
        };
        let snapshot = engine.update(
            &document,
            Arc::clone(&registry),
            &analysis,
            CompileOptions::default(),
        )?;
        // A constant that faults is one output's result, not a refusal of the others.
        let evaluation = snapshot
            .evaluate(&document, &registry, EvaluationOptions::default())
            .map_err(|error| refused(&error))?;
        wanted
            .into_iter()
            .zip(evaluation.exports())
            .map(|((node, port), export)| {
                let export = export.map_err(|error| refused(&error))?;
                Ok(Previewed {
                    node,
                    port,
                    value: export
                        .outcome
                        .cloned()
                        .map_err(|fault| (fault.source, fault.fault.clone())),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value as Json, json};

    fn id(n: u32) -> String {
        format!("10000000-0000-4000-8000-{n:012x}")
    }

    #[test]
    fn pure_outputs_are_computed_without_running_and_a_fault_is_one_outputs_result() {
        let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
        let math = |n: u32, name: &str, inputs: Json| {
            json!({"id": id(n), "definition_id": format!("org.orange.math/{name}"),
                "definition_version": 1, "config_version": 1, "config": {},
                "type_pins": {"T": "core/i64@1"}, "inputs": inputs})
        };
        let number = |value: &str| json!({"type": "core/i64@1", "value": value});
        let edge = |n: u32, port: &str, to: u32| {
            json!({"id": format!("30000000-0000-4000-8000-{n:012x}"),
                "from": {"node": id(1), "port": port}, "to": {"node": id(to), "port": "a"}})
        };
        let mut written = json!({"format": "orange.graph", "format_version": 1,
            "graph_id": "20000000-0000-4000-8000-0000000000f3", "revision": 1,
            "package_lock": [], "type_lock": [], "variables": [], "events": [],
            "connections": [], "signatures": {}, "functions": [], "exports": [], "editor": {},
            "nodes": [math(1, "add", json!({"a": number("40"), "b": number("2")})),
                math(2, "multiply", json!({"b": number("10")})),
                math(3, "divide", json!({"b": number("0")}))],
            "edges": [edge(1, "sum", 2), edge(2, "sum", 3)]});
        let bound = host.bind(&written.to_string()).unwrap();
        let node = |n: u32| id(n).parse::<NodeId>().unwrap();
        let port = |name: &str| name.parse::<PortKey>().unwrap();
        let wanted = [
            (node(2), port("product")),
            (node(3), port("quotient")),
            (node(1), port("sum")),
        ];
        let seen = host.preview(&bound, &wanted).unwrap();
        assert_eq!(seen[0].value, Ok(Value::I64(420)));
        assert_eq!(seen[2].value, Ok(Value::I64(42)));
        let (failed_at, fault) = seen[1].value.clone().unwrap_err();
        assert_eq!(
            (failed_at, fault.code.as_ref()),
            (node(3), "core/DivideByZero")
        );
        // No outputs named and none exported: nothing to compute.
        assert_eq!(host.preview(&bound, &[]).unwrap(), []);
        // An edit is previewed with the engine kept from the first call.
        written["nodes"][0]["inputs"]["b"] = number("3");
        let edited = host.bind(&written.to_string()).unwrap();
        let seen = host.preview(&edited, &wanted[..1]).unwrap();
        assert_eq!(seen[0].value, Ok(Value::I64(430)));
        // A port that does not exist is a diagnostic, not a value.
        assert!(matches!(
            host.preview(&bound, &[(node(1), port("nope"))]),
            Err(HostError::Invalid(_))
        ));
        // Nothing was opened.
        assert_eq!(host.flows().count(), 0);
    }
}
