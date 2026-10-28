//! Saved-graph builders and checked compilation for the workloads.
//!
//! The math catalog and node/edge helpers are the bundled integration-test builders, shared
//! by path so the harness and the tests describe graphs the same way.

#[path = "../../orange-bundled/tests/support/data_graph.rs"]
#[allow(dead_code)]
mod data_graph;

use orange_bundled::MathPackage;
use orange_compiler::{CompileOptions, ControlPlan, compile_control};
use orange_model::catalog::RegistrySnapshot;
use orange_model::validation::ValidationAnalysis;
use orange_model::{GraphDocument, analyze, from_json, to_json};
use serde_json::{Value as Json, json};
use std::sync::Arc;

pub use data_graph::{edge, id, math};

pub const U64: &str = "core/u64@1";

pub fn core(n: u32, name: &str) -> Json {
    let mut node = json!({"id":id(n),"definition_id":format!("core/{name}"),"definition_version":1,"config_version":1,"config":{}});
    if name == "on-start" {
        node["activation"] = json!({"mode":"queue","capacity":4,"overflow":"drop-newest","on_fault":"continue","order":0});
    }
    node
}

pub fn var_node(n: u32, name: &str, variable: u32) -> Json {
    let mut node = core(n, name);
    node["config"] = json!({"variable":id(variable)});
    node
}

/// `update-variable` with a typed literal operand; connected inputs override it.
pub fn update(n: u32, variable: u32, operation: &str, ty: &str, value: Json) -> Json {
    let mut node = var_node(n, "update-variable", variable);
    node["config"]["operation"] = json!(operation);
    node["inputs"] = json!({"value":{"type":ty,"value":value}});
    node
}

pub fn count(n: u32, times: u64) -> Json {
    let mut node = core(n, "count");
    node["inputs"] = json!({"count":{"type":"core/i64@1","value":times.to_string()}});
    node
}

pub fn run_variable(n: u32, ty: &str, init: Option<Json>) -> Json {
    let mut variable = json!({"id":id(n),"name":format!("v{n}"),"type":ty,"scope":"run"});
    if let Some(value) = init {
        variable["init"] = json!({"type":ty,"value":value});
    }
    variable
}

pub fn port(name: &str, direction: &str, ty: &str) -> Json {
    json!({"port_id":name,"direction":direction,"kind":"data","type":ty})
}

/// One saved graph with declarations, parsed back so validation sees a loaded document.
pub struct Source {
    pub nodes: Vec<Json>,
    pub edges: Vec<Json>,
    pub exports: Vec<Json>,
    pub variables: Vec<Json>,
    pub functions: Vec<Json>,
}

impl Source {
    pub fn new(nodes: Vec<Json>, edges: Vec<Json>) -> Self {
        Self {
            nodes,
            edges,
            exports: vec![],
            variables: vec![],
            functions: vec![],
        }
    }

    /// Loads the saved graph and requires it to be ready for lowering.
    fn checked(
        self,
    ) -> (
        MathPackage,
        Arc<RegistrySnapshot>,
        GraphDocument,
        ValidationAnalysis,
    ) {
        let (package, snapshot) = data_graph::catalog();
        let base = data_graph::graph(&package, self.nodes, self.edges, self.exports);
        let mut raw: Json =
            serde_json::from_str(&to_json(&base).expect("saved graph")).expect("saved graph JSON");
        raw["variables"] = json!(self.variables);
        raw["functions"] = json!(self.functions);
        let document = from_json(&raw.to_string()).expect("loadable graph");
        let analysis = analyze(&document, &snapshot);
        assert!(
            analysis.is_ready_for_lowering(&document, &snapshot),
            "{:?}",
            analysis.report().diagnostics
        );
        (package, snapshot, document, analysis)
    }

    pub fn control(self) -> (GraphDocument, Arc<ControlPlan>) {
        let (package, snapshot, document, analysis) = self.checked();
        let options = CompileOptions::default();
        let plan = compile_control(&document, snapshot, &analysis, &package, options);
        (document, Arc::new(plan.expect("control plan")))
    }
}
