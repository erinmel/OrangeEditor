//! Acceptance of graph-local functions as executable call contracts.
//!
//! A call resolves only against an accepted callee, and a callee is accepted only when its own
//! body, which may contain calls, is complete and conformant. Recursive callees are never
//! accepted, so acceptance has a unique fixed point. [`settle`] reaches it with whole-document
//! rounds, which alone would cost one round per level of a call chain. [`accept_functions`]
//! computes the same fixed point first by checking each body once, callee first, so `settle`
//! normally needs a single round, avoiding one whole-document round per call-chain level.
//! Individual passes and transitive effect summaries have their own costs.

use std::collections::{BTreeMap, BTreeSet};

use crate::catalog::RegistrySnapshot;
use crate::document::{GraphDocument, NodeKind, Suspends};
use crate::id::{FunctionId, NodeId};
use crate::index::DocIndex;

use super::conformance::{BodyStatus, Conformance, check_conformance};
use super::connections::{ConnectionValidation, validate_connections_in};
use super::control_cycles::{ControlCycleValidation, validate_control_cycles_in};
use super::data_cycles::{DataCycleValidation, validate_data_cycles_in};
use super::functions::{
    FunctionAnalysis, FunctionEvidence, LeafEffects, SummaryContext, analyze_functions, call_graph,
    callee_first_order, find_cycles, summarize_function,
};
use super::inputs::validate_inputs_in;
use super::local::PassCaches;
use super::nodes::{CallEvidence, NodeValidation, validate_nodes_with};
use super::regions::{RegionValidation, validate_regions_in};
use super::scope::Scope;
use super::{AnalysisOptions, Diagnostic, DiagnosticCode, Severity};

pub(crate) type Accepted = BTreeMap<FunctionId, (NodeKind, Suspends)>;

/// Results of the body-local passes that decide whether a body is complete.
pub(crate) struct BodyPasses {
    pub(crate) projections: std::rc::Rc<super::ports::CachedPorts>,
    pub(crate) nodes: NodeValidation,
    pub(crate) connections: ConnectionValidation,
    pub(crate) data_cycles: DataCycleValidation,
    pub(crate) regions: RegionValidation,
    pub(crate) control: ControlCycleValidation,
    pub(crate) input_diagnostics: Vec<Diagnostic>,
}

impl BodyPasses {
    pub(crate) fn run(
        document: &GraphDocument,
        registry: &RegistrySnapshot,
        calls: &CallEvidence,
        scope: Scope,
        index: &DocIndex,
        mut caches: Option<&mut PassCaches>,
        options: &AnalysisOptions<'_>,
    ) -> Result<Self, DiagnosticCode> {
        options.checkpoint()?;
        let nodes = validate_nodes_with(
            document,
            registry,
            calls,
            scope,
            index,
            caches.as_deref_mut().map(|caches| &mut caches.components),
        );
        options.checkpoint()?;
        let ports = super::ports::PortResolver::new(document, &nodes.resolved);
        let resolved = &ports;
        let connections = validate_connections_in(
            document,
            resolved,
            registry,
            scope,
            index,
            caches.as_deref_mut().map(|caches| &mut caches.edges),
        );
        options.checkpoint()?;
        let data_cycles =
            validate_data_cycles_in(document, resolved, &connections, registry, scope);
        options.checkpoint()?;
        let regions = validate_regions_in(document, resolved, &connections, registry, scope);
        options.checkpoint()?;
        let control = validate_control_cycles_in(document, resolved, &connections, registry, scope);
        options.checkpoint()?;
        let input_diagnostics = validate_inputs_in(
            document,
            resolved,
            registry,
            scope,
            Some(index),
            caches.map(|caches| &mut caches.inputs),
        );
        options.checkpoint()?;
        let projections = ports.retain();
        drop(ports);
        Ok(Self {
            projections,
            nodes,
            connections,
            data_cycles,
            regions,
            control,
            input_diagnostics,
        })
    }

    /// Nodes that are unresolved, incomplete or carry an error diagnostic.
    fn blocked(&self) -> BTreeSet<NodeId> {
        let mut blocked = self.nodes.incomplete_nodes.clone();
        blocked.extend(self.connections.incomplete_nodes.iter().copied());
        blocked.extend(self.data_cycles.incomplete_nodes.iter().copied());
        blocked.extend(self.regions.incomplete_nodes.iter().copied());
        blocked.extend(self.control.incomplete_nodes.iter().copied());
        for diagnostic in self
            .nodes
            .diagnostics
            .iter()
            .chain(&self.input_diagnostics)
            .chain(&self.connections.diagnostics)
            .chain(&self.data_cycles.diagnostics)
            .chain(&self.regions.diagnostics)
            .chain(&self.control.diagnostics)
            .filter(|item| item.severity == Severity::Error)
        {
            blocked.extend(diagnostic.subject.node);
        }
        blocked
    }
}

/// Whole-document pass results at the acceptance fixed point.
pub(crate) struct Settled {
    pub(crate) passes: BodyPasses,
    /// The callee acceptance `passes` resolved call nodes with.
    pub(crate) calls: Accepted,
    pub(crate) functions: FunctionAnalysis,
    pub(crate) conformance: Conformance,
    /// Whole-document rounds run before acceptance stopped changing.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "observed by the convergence tests")
    )]
    pub(crate) rounds: usize,
}

/// Runs whole-document rounds from the `seed` acceptance until it stops changing. From the
/// fixed point this is one round; from any other seed it converges within one round per
/// call-chain level, so a wrong seed costs time, never correctness.
pub(crate) fn settle(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    leaf_effects: &BTreeMap<NodeId, LeafEffects>,
    index: &DocIndex,
    seed: Accepted,
    mut caches: Option<&mut PassCaches>,
    options: &AnalysisOptions<'_>,
) -> Result<Settled, DiagnosticCode> {
    let mut calls = CallEvidence {
        accepted: seed,
        pipeline: true,
    };
    let mut rounds = 0;
    loop {
        let passes = BodyPasses::run(
            document,
            registry,
            &calls,
            Scope::All,
            index,
            caches.as_deref_mut(),
            options,
        )?;
        let evidence = FunctionEvidence::from_nodes(document, &passes.nodes, leaf_effects.clone());
        let functions = analyze_functions(document, &evidence, registry);
        options.checkpoint()?;
        let blocked = passes.blocked();
        let status = BodyStatus {
            resolved: &passes.nodes.resolved,
            regions: &passes.regions.index,
            blocked: &blocked,
        };
        let conformance = check_conformance(document, &functions.summaries, &status, registry);
        options.checkpoint()?;
        let errors = function_errors(&functions.diagnostics);
        let accepted: Accepted = conformance
            .conformant
            .iter()
            .filter(|id| !errors.contains(id))
            .filter_map(|id| {
                functions
                    .summaries
                    .get(id)
                    .map(|summary| (*id, (summary.kind, summary.suspends)))
            })
            .collect();
        rounds += 1;
        if accepted == calls.accepted || rounds > document.functions().len() + 1 {
            return Ok(Settled {
                passes,
                calls: calls.accepted,
                functions,
                conformance,
                rounds,
            });
        }
        calls.accepted = accepted;
    }
}

/// The acceptance fixed point, computed by checking each function body once in callee-first
/// order, when every callee's acceptance is already final.
pub(crate) fn accept_functions(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    leaf_effects: &BTreeMap<NodeId, LeafEffects>,
    index: &DocIndex,
    mut caches: Option<&mut PassCaches>,
    options: &AnalysisOptions<'_>,
) -> Result<Accepted, DiagnosticCode> {
    options.checkpoint()?;
    let mut functions = FunctionAnalysis::default();
    let graph = call_graph(document, registry, &mut functions);
    let (_, cyclic) = find_cycles(&graph);
    let mut errors = function_errors(&functions.diagnostics);
    let mut calls = CallEvidence {
        accepted: BTreeMap::new(),
        pipeline: true,
    };
    for id in callee_first_order(&graph, &cyclic) {
        let passes = BodyPasses::run(
            document,
            registry,
            &calls,
            Scope::Function(id),
            index,
            caches.as_deref_mut(),
            options,
        )?;
        let context = SummaryContext {
            document,
            resolved: &passes.nodes.resolved,
            leaf_effects,
            registry,
            calls: &graph,
            cyclic: &cyclic,
        };
        let known = functions.diagnostics.len();
        summarize_function(&context, id, &mut functions);
        options.checkpoint()?;
        errors.extend(function_errors(&functions.diagnostics[known..]));
        let Some(summary) = functions.summaries.get(&id) else {
            continue;
        };
        if errors.contains(&id) {
            continue;
        }
        let blocked = passes.blocked();
        let status = BodyStatus {
            resolved: &passes.nodes.resolved,
            regions: &passes.regions.index,
            blocked: &blocked,
        };
        let summaries = BTreeMap::from([(id, summary.clone())]);
        if check_conformance(document, &summaries, &status, registry)
            .conformant
            .contains(&id)
        {
            calls.accepted.insert(id, (summary.kind, summary.suspends));
        }
    }
    options.checkpoint()?;
    Ok(calls.accepted)
}

/// One function's acceptance, summary and function-level diagnostics.
pub(crate) struct FunctionCheck {
    pub accepted: Option<(NodeKind, Suspends)>,
    pub summary: Option<super::functions::FunctionSummary>,
    pub incomplete: bool,
    /// Function-pass and conformance diagnostics whose subject is the function, sorted.
    pub diagnostics: Vec<Diagnostic>,
}

/// `id`'s acceptance, summary and function-level diagnostics, computed as
/// [`accept_functions`] and the final round compute them once every callee's acceptance
/// (`calls`) and summary (`summaries`) is final.
#[expect(
    clippy::too_many_arguments,
    reason = "a function check reads its callees' final acceptance and summaries beside the pass inputs"
)]
pub(crate) fn check_function(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    leaf_effects: &BTreeMap<NodeId, LeafEffects>,
    index: &DocIndex,
    calls: &Accepted,
    summaries: &BTreeMap<FunctionId, super::functions::FunctionSummary>,
    id: FunctionId,
    options: &AnalysisOptions<'_>,
) -> Result<FunctionCheck, DiagnosticCode> {
    let evidence = CallEvidence {
        accepted: calls.clone(),
        pipeline: true,
    };
    let passes = BodyPasses::run(
        document,
        registry,
        &evidence,
        Scope::Function(id),
        index,
        None,
        options,
    )?;
    let mut functions = FunctionAnalysis::default();
    let graph = call_graph(document, registry, &mut functions);
    let (cycles, cyclic) = find_cycles(&graph);
    for path in &cycles {
        let mut diagnostic = Diagnostic::new(
            DiagnosticCode::Recursion,
            super::Subject::new(None, None, None).with_function(path[0]),
            registry.origin(),
        );
        diagnostic.severity = Severity::Error;
        functions.diagnostics.push(diagnostic);
    }
    functions
        .incomplete_functions
        .extend(cyclic.iter().copied());
    functions.summaries = summaries.clone();
    functions.summaries.remove(&id);
    let context = SummaryContext {
        document,
        resolved: &passes.nodes.resolved,
        leaf_effects,
        registry,
        calls: &graph,
        cyclic: &cyclic,
    };
    summarize_function(&context, id, &mut functions);
    let errors = function_errors(&functions.diagnostics);
    let summary = functions.summaries.get(&id).cloned();
    let blocked = passes.blocked();
    let status = BodyStatus {
        resolved: &passes.nodes.resolved,
        regions: &passes.regions.index,
        blocked: &blocked,
    };
    let conformance = match &summary {
        Some(summary) => check_conformance(
            document,
            &BTreeMap::from([(id, summary.clone())]),
            &status,
            registry,
        ),
        None => Conformance::default(),
    };
    let accepted = summary
        .as_ref()
        .filter(|_| !errors.contains(&id) && conformance.conformant.contains(&id))
        .map(|summary| (summary.kind, summary.suspends));
    let mut diagnostics: Vec<Diagnostic> = functions
        .diagnostics
        .into_iter()
        .chain(conformance.diagnostics)
        .filter(|diagnostic| diagnostic.subject.function == Some(id))
        .collect();
    super::sort_diagnostics(&mut diagnostics);
    diagnostics.dedup();
    Ok(FunctionCheck {
        accepted,
        summary,
        incomplete: functions.incomplete_functions.contains(&id),
        diagnostics,
    })
}

fn function_errors(diagnostics: &[Diagnostic]) -> BTreeSet<FunctionId> {
    diagnostics
        .iter()
        .filter(|item| item.severity == Severity::Error)
        .filter_map(|item| item.subject.function)
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::validation::HostOrigin;

    fn id(prefix: u32, n: usize) -> String {
        format!("{prefix:08x}-0000-4000-8000-{n:012}")
    }

    fn node(id: &str, definition: &str, config: Value) -> Value {
        json!({"id":id,"definition_id":definition,"definition_version":1,"config_version":1,
            "config":config})
    }

    fn edge(n: usize, from: (&str, &str), to: (&str, &str)) -> Value {
        json!({"id":id(0x2000_0000, n),"from":{"node":from.0,"port":from.1},
            "to":{"node":to.0,"port":to.1}})
    }

    fn port(port: &str, direction: &str) -> Value {
        json!({"port_id":port,"direction":direction,"kind":"data","type":"core/i64@1"})
    }

    /// A pure `x -> y` function: identity, or through one call per callee, chained.
    fn function(n: usize, kind: &str, callees: &[usize], wired: bool) -> Value {
        let entry = id(0x4000_0000, n * 100);
        let out = id(0x4000_0000, n * 100 + 1);
        let mut nodes = vec![
            node(&entry, "core/function-entry", json!({})),
            node(&out, "core/function-outputs", json!({})),
        ];
        let mut edges = Vec::new();
        let mut source = entry.clone();
        let mut source_port = "x";
        for (k, callee) in callees.iter().enumerate() {
            let call = id(0x4000_0000, n * 100 + 2 + k);
            let function = id(0x3000_0000, *callee);
            nodes.push(node(&call, "core/call", json!({"function":function})));
            edges.push(edge(n * 100 + k, (&source, source_port), (&call, "x")));
            source = call;
            source_port = "y";
        }
        if wired {
            edges.push(edge(n * 100 + 99, (&source, source_port), (&out, "y")));
        }
        json!({"id":id(0x3000_0000, n),"name":format!("f{n}"),"kind":kind,"nodes":nodes,
            "edges":edges,"inputs":[port("x","input")],"outputs":[port("y","output")]})
    }

    fn document(functions: Vec<Value>) -> GraphDocument {
        serde_json::from_value(json!({"format_version":1,
            "graph_id":"50000000-0000-4000-8000-000000000001","revision":0,"nodes":[],
            "functions":functions}))
        .unwrap()
    }

    /// Settles from the callee-first seed and from nothing, which must agree; returns the
    /// rounds each took.
    fn converge(document: &GraphDocument) -> (Accepted, usize, usize) {
        let registry = RegistrySnapshot::core(
            HostOrigin::new("1.0.0", format!("sha256:{}", "a".repeat(64))).unwrap(),
        );
        let effects = BTreeMap::new();
        let index = DocIndex::build(document);
        let options = AnalysisOptions::default();
        let seed = accept_functions(document, &registry, &effects, &index, None, &options).unwrap();
        let seeded = settle(
            document,
            &registry,
            &effects,
            &index,
            seed.clone(),
            None,
            &options,
        )
        .unwrap();
        let unseeded = settle(
            document,
            &registry,
            &effects,
            &index,
            BTreeMap::new(),
            None,
            &options,
        )
        .unwrap();
        assert_eq!(seeded.passes.nodes, unseeded.passes.nodes);
        assert_eq!(seeded.functions, unseeded.functions);
        assert_eq!(
            seeded.conformance.conformant,
            unseeded.conformance.conformant
        );
        (seed, seeded.rounds, unseeded.rounds)
    }

    #[test]
    fn a_call_chain_settles_in_one_round_from_the_callee_first_seed() {
        let depth = 40;
        let chain = (0..depth)
            .map(|n| {
                let callees: &[usize] = if n + 1 < depth { &[n + 1] } else { &[] };
                function(n, "pure", callees, true)
            })
            .collect();
        let (accepted, seeded, unseeded) = converge(&document(chain));
        assert_eq!(accepted.len(), depth);
        assert_eq!(seeded, 1);
        assert_eq!(unseeded, depth + 1);
    }

    #[test]
    fn rejected_callees_keep_their_callers_unaccepted_in_one_round() {
        let cases = [
            // An unwired leaf is incomplete.
            vec![
                function(0, "pure", &[1], true),
                function(1, "pure", &[2], true),
                function(2, "pure", &[], false),
            ],
            // A leaf declared with the wrong kind carries a function error.
            vec![
                function(0, "pure", &[1], true),
                function(1, "action", &[], true),
            ],
            // Recursion is never accepted, nor is a caller of it.
            vec![
                function(0, "pure", &[1], true),
                function(1, "pure", &[2], true),
                function(2, "pure", &[1], true),
            ],
        ];
        for functions in cases {
            let (accepted, seeded, _) = converge(&document(functions));
            assert!(accepted.is_empty());
            assert_eq!(seeded, 1);
        }
    }

    #[test]
    fn shared_callees_are_accepted_once_for_every_caller() {
        let diamond = vec![
            function(0, "pure", &[1, 2], true),
            function(1, "pure", &[3], true),
            function(2, "pure", &[3], true),
            function(3, "pure", &[], true),
        ];
        let (accepted, seeded, _) = converge(&document(diamond));
        assert_eq!(accepted.len(), 4);
        assert_eq!(seeded, 1);
    }
}
