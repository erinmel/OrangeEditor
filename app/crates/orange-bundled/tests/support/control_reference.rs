//! Bounded source interpreter: scans graph edges, uses lexical recursion for
//! regions, and evaluates source data independently. Never reads compiled IR,
//! runtime cursors, native kernels or the production scheduling implementation.
use super::reference::{Fault, Outcome, Reference};
use orange_model::catalog::RegistrySnapshot;
use orange_model::document::VarScope;
use orange_model::id::{NodeId, PortKey, VariableId};
use orange_model::validation::ValidationAnalysis;
use orange_model::value::literal_codec;
use orange_model::{GraphDocument, Value};
use std::collections::BTreeMap;
use std::sync::Arc;

enum Flow {
    End,
    Break,
    Continue,
    Fault,
}
pub struct ControlReference<'a> {
    document: &'a GraphDocument,
    snapshot: &'a RegistrySnapshot,
    analysis: &'a ValidationAnalysis,
    data: Reference<'a>,
    pub variables: BTreeMap<VariableId, Option<Value>>,
    produced: BTreeMap<(NodeId, PortKey), Value>,
    pub observations: Vec<(NodeId, &'static str)>,
    pub faults: Vec<(NodeId, NodeId, &'static str)>,
    pub now: u64,
    steps: usize,
}
impl<'a> ControlReference<'a> {
    pub fn new(
        document: &'a GraphDocument,
        snapshot: &'a RegistrySnapshot,
        analysis: &'a ValidationAnalysis,
        seed: u64,
    ) -> Self {
        let variables = document
            .variables()
            .values()
            .map(|v| {
                (
                    v.id,
                    v.init
                        .as_ref()
                        .map(|i| literal_codec::decode(i, snapshot.types().as_ref()).unwrap()),
                )
            })
            .collect();
        Self {
            document,
            snapshot,
            analysis,
            data: Reference::seeded(document, snapshot, analysis, seed, 0),
            variables,
            produced: BTreeMap::new(),
            observations: vec![],
            faults: vec![],
            now: 0,
            steps: 0,
        }
    }
    pub fn run(&mut self, seed: u64) {
        let document = self.document;
        let mut entries: Vec<_> = document
            .root()
            .nodes
            .values()
            .filter(|n| n.definition.id.as_str() == "core/on-start")
            .collect();
        entries.sort_by_key(|n| (n.activation.as_ref().unwrap().order, n.id));
        for (sequence, entry) in entries.iter().enumerate() {
            self.data = Reference::seeded(
                document,
                self.snapshot,
                self.analysis,
                seed,
                sequence as u64,
            );
            self.produced.clear();
            for v in document
                .variables()
                .values()
                .filter(|v| v.scope == VarScope::Local)
            {
                self.variables.insert(
                    v.id,
                    v.init
                        .as_ref()
                        .map(|i| literal_codec::decode(i, self.snapshot.types().as_ref()).unwrap()),
                );
            }
            let _ = self.region(Some(entry.id), 0);
        }
    }
    fn target(&self, node: NodeId, port: &str) -> Option<NodeId> {
        let key: PortKey = port.parse().unwrap();
        self.document
            .root()
            .edges
            .values()
            .find(|e| e.from.node == node && e.from.port == key)
            .map(|e| e.to.node)
    }
    fn input(&mut self, node: NodeId, port: &str) -> Outcome {
        self.data.begin_epoch();
        for n in self
            .document
            .root()
            .nodes
            .values()
            .filter(|n| n.definition.id.as_str() == "core/get-variable")
        {
            let id: VariableId = n.config["variable"].as_str().unwrap().parse().unwrap();
            let value = self.variables[&id].clone().ok_or(Fault {
                source: n.id,
                code: "core/UninitializedVariable",
            });
            self.data.supply(n.id, "value".parse().unwrap(), value);
        }
        for ((source, port), value) in &self.produced {
            self.data.supply(*source, port.clone(), Ok(value.clone()));
        }
        let key: PortKey = port.parse().unwrap();
        if let Some(edge) = self
            .document
            .root()
            .edges
            .values()
            .find(|e| e.to.node == node && e.to.port == key)
        {
            return self.data.source(edge.from.node, &edge.from.port);
        }
        let literal = &self.document.root().nodes[&node].inputs[&key];
        Ok(literal_codec::decode(literal, self.snapshot.types().as_ref()).unwrap())
    }
    fn failed(&mut self, node: NodeId, fault: Fault) -> Option<NodeId> {
        self.observations.push((node, "failed"));
        self.faults.push((node, fault.source, fault.code));
        self.produced.insert(
            (node, "error".parse().unwrap()),
            Value::Err(Arc::new(orange_model::value::NodeError {
                code: fault.code.into(),
                node: Some(fault.source),
            })),
        );
        self.target(node, "failed")
    }
    fn region(&mut self, mut pc: Option<NodeId>, depth: usize) -> Flow {
        assert!(depth <= 32, "bounded source control depth");
        let document = self.document;
        while let Some(id) = pc {
            self.steps += 1;
            assert!(self.steps < 20_000, "bounded source control fuel");
            let node = &document.root().nodes[&id];
            pc = match node.definition.id.as_str() {
                "core/on-start" => self.target(id, "then"),
                "core/branch" => match self.input(id, "condition") {
                    Ok(Value::Bool(flag)) => {
                        self.observations
                            .push((id, if flag { "true" } else { "false" }));
                        self.target(id, if flag { "true" } else { "false" })
                    }
                    Err(fault) => {
                        let next = self.failed(id, fault);
                        if next.is_none() {
                            return Flow::Fault;
                        }
                        next
                    }
                    _ => panic!("oracle bool contract"),
                },
                "core/set-variable" => match self.input(id, "value") {
                    Ok(value) => {
                        let variable = node.config["variable"].as_str().unwrap().parse().unwrap();
                        self.variables.insert(variable, Some(value));
                        self.observations.push((id, "then"));
                        self.target(id, "then")
                    }
                    Err(fault) => {
                        let next = self.failed(id, fault);
                        if next.is_none() {
                            return Flow::Fault;
                        }
                        next
                    }
                },
                "core/sequence" => {
                    for k in &node.variadic[&"then".parse().unwrap()].order {
                        match self.region(self.target(id, &format!("then#{k}")), depth + 1) {
                            Flow::End => {}
                            flow => return flow,
                        }
                    }
                    None
                }
                "core/count" => {
                    let count = match self.input(id, "count") {
                        Ok(Value::I64(count)) if count >= 0 => count,
                        Ok(Value::I64(_)) => {
                            let next = self.failed(
                                id,
                                Fault {
                                    source: id,
                                    code: "core/OutOfRange",
                                },
                            );
                            if next.is_none() {
                                return Flow::Fault;
                            }
                            pc = next;
                            continue;
                        }
                        Err(fault) => {
                            let next = self.failed(id, fault);
                            if next.is_none() {
                                return Flow::Fault;
                            }
                            pc = next;
                            continue;
                        }
                        _ => panic!("oracle count contract"),
                    };
                    for index in 0..count {
                        self.produced
                            .insert((id, "index".parse().unwrap()), Value::I64(index));
                        match self.region(self.target(id, "body"), depth + 1) {
                            Flow::End | Flow::Continue => {}
                            Flow::Break => break,
                            Flow::Fault => return Flow::Fault,
                        }
                    }
                    self.produced.remove(&(id, "index".parse().unwrap()));
                    self.target(id, "completed")
                }
                "core/while" => {
                    let maximum = node.config["max_iterations"].as_u64().unwrap();
                    let mut iterations = 0;
                    let failure = loop {
                        match self.input(id, "condition") {
                            Ok(Value::Bool(false)) => break None,
                            Ok(Value::Bool(true)) if iterations >= maximum => {
                                break Some(Fault {
                                    source: id,
                                    code: "core/LoopBudgetExceeded",
                                });
                            }
                            Ok(Value::Bool(true)) => {}
                            Err(fault) => break Some(fault),
                            _ => panic!("oracle condition contract"),
                        }
                        iterations += 1;
                        match self.region(self.target(id, "body"), depth + 1) {
                            Flow::End | Flow::Continue => {}
                            Flow::Break => break None,
                            Flow::Fault => return Flow::Fault,
                        }
                    };
                    if let Some(fault) = failure {
                        let next = self.failed(id, fault);
                        if next.is_none() {
                            return Flow::Fault;
                        }
                        next
                    } else {
                        self.target(id, "completed")
                    }
                }
                "core/wait" => match self.input(id, "duration") {
                    Ok(Value::Duration(n)) if n >= 0 => {
                        self.observations.push((id, "suspended"));
                        self.now = self.now.checked_add(n as u64).unwrap();
                        self.observations.push((id, "then"));
                        self.target(id, "then")
                    }
                    Ok(Value::Duration(_)) => {
                        let next = self.failed(
                            id,
                            Fault {
                                source: id,
                                code: "core/OutOfRange",
                            },
                        );
                        if next.is_none() {
                            return Flow::Fault;
                        }
                        next
                    }
                    Err(fault) => {
                        let next = self.failed(id, fault);
                        if next.is_none() {
                            return Flow::Fault;
                        }
                        next
                    }
                    _ => panic!("oracle duration contract"),
                },
                "core/yield" => {
                    self.observations.push((id, "yielded"));
                    self.target(id, "then")
                }
                "core/break" => {
                    self.observations.push((id, "break"));
                    return Flow::Break;
                }
                "core/continue" => {
                    self.observations.push((id, "continue"));
                    return Flow::Continue;
                }
                _ => panic!("unsupported source control {}", node.definition.id),
            };
        }
        Flow::End
    }
}
