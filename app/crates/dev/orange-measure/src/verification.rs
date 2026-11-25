//! Paired measurements for observation, composite stream collection and control lowering.
use crate::graph::{self, Source, U64, core, edge, math, run_variable, var_node};
use crate::harness::{Config, Session, measure, print_table};
use orange_compiler::{CompileOptions, KernelFault, compile_control};
use orange_model::Value;
use orange_model::value::ListRepr;
use orange_runtime::{
    CloseReason, PullStream, Run, RunOptions, StreamCollectPoll, StreamDemand, StreamStep,
};
use serde_json::json;
use std::collections::BTreeMap;
use std::hint::black_box;
use std::sync::Arc;

struct Sample<F> {
    work: Option<F>,
    result: usize,
}

impl<F: FnOnce() -> usize> Session for Sample<F> {
    type Outcome = usize;

    fn step(&mut self) -> bool {
        self.result = self.work.take().expect("one sample")();
        false
    }

    fn finish(self) -> usize {
        self.result
    }
}

fn sample<F: FnOnce() -> usize>(work: F) -> Sample<F> {
    Sample {
        work: Some(work),
        result: 0,
    }
}

pub fn run(config: Config, filter: &str) -> bool {
    match filter {
        "observe-variables" => observe_variables(config),
        "collect-composites" => collect_composites(config),
        "compile-watches" => compile_watches(config),
        _ => return false,
    }
    true
}

fn observe_variables(config: Config) {
    const ROUNDS: usize = 16;
    let rows = [128, 1024, 4096].map(|count| {
        let mut source = Source::new(
            (1..=count)
                .map(|n| var_node(n, "get-variable", n))
                .collect(),
            Vec::new(),
        );
        source.variables = (1..=count)
            .map(|n| run_variable(n, U64, Some(json!("0"))))
            .collect();
        let (doc, plan) = source.control();
        let mut run = Run::start(
            plan.clone(),
            &doc,
            plan.data().snapshot(),
            42,
            RunOptions::default(),
        )
        .unwrap();
        let reported: BTreeMap<_, _> = plan
            .variables()
            .iter()
            .map(|cell| (cell.id(), Some(Value::U64(0))))
            .collect();
        assert_eq!(reported.len(), count as usize);
        run.set_variable(graph::id(count).parse().unwrap(), Value::U64(1))
            .unwrap();
        measure(
            format!("observe {count} variables, one changed per read"),
            ROUNDS as u64,
            "passes",
            config,
            || {
                sample(|| {
                    let mut samples = 0;
                    for _ in 0..ROUNDS {
                        let changed: Vec<_> = black_box(&run)
                            .variable_values()
                            .filter_map(|(id, value)| {
                                if black_box(&reported).get(&id).map(Option::as_ref) == Some(value)
                                {
                                    return None;
                                }
                                Some((id, value.cloned()))
                            })
                            .collect();
                        samples += black_box(changed).len();
                    }
                    samples
                })
            },
            |samples| assert_eq!(samples, ROUNDS),
        )
    });
    print_table(&rows);
}

struct CompositeProducer {
    value: Value,
    remaining: usize,
}

impl PullStream for CompositeProducer {
    fn next(&mut self, _: StreamDemand) -> Result<StreamStep, KernelFault> {
        if self.remaining == 0 {
            return Ok(StreamStep::End);
        }
        self.remaining -= 1;
        Ok(StreamStep::Item(self.value.clone()))
    }

    fn close(self: Box<Self>, _: CloseReason) {}
}

fn collect_composites(config: Config) {
    const ITEMS: usize = 96;
    let (doc, plan) = Source::new(Vec::new(), Vec::new()).control();
    let ty = "core/list@1<core/option@1<core/u64@1>>".parse().unwrap();
    let value = Value::List(Arc::new(ListRepr::Values(
        (0..1024)
            .map(|n| {
                if n % 2 == 0 {
                    Value::U64(n)
                } else {
                    Value::None
                }
            })
            .collect(),
    )));
    let row = measure(
        "collect 96 composite items with 1024 optional scalars each".into(),
        ITEMS as u64,
        "items",
        config,
        || {
            sample(|| {
                let mut run = Run::start(
                    plan.clone(),
                    &doc,
                    plan.data().snapshot(),
                    42,
                    RunOptions::default(),
                )
                .unwrap();
                let stream = run
                    .open_stream(
                        &ty,
                        Box::new(CompositeProducer {
                            value: value.clone(),
                            remaining: ITEMS,
                        }),
                    )
                    .unwrap();
                let mut consumer = run.claim_stream(stream).unwrap();
                let StreamCollectPoll::Complete(Value::List(values)) = run
                    .collect_stream(&mut consumer, ITEMS, &mut (ITEMS + 1))
                    .unwrap()
                else {
                    panic!("collect did not finish");
                };
                assert_eq!(values.get(0), Some(value.clone()));
                values.len()
            })
        },
        |items| assert_eq!(items, ITEMS),
    );
    print_table(&[row]);
}

fn compile_watches(config: Config) {
    const CHAIN: u32 = 256;
    const WATCHES: u32 = 512;
    let mut nodes = vec![var_node(1, "get-variable", 1)];
    let mut edges = Vec::new();
    for n in 2..=CHAIN + 1 {
        nodes.push(math(n, "add", 0, 1));
        edges.push(edge(n, n - 1, if n == 2 { "value" } else { "sum" }, n, "a"));
    }
    for n in CHAIN + 2..CHAIN + 2 + WATCHES {
        let mut watch = core(n, "on-watch");
        watch["activation"] = json!({"mode":"queue","capacity":4,"overflow":"drop-newest","on_fault":"continue","order":0});
        watch["config"] = json!({"mode":"changed"});
        nodes.push(watch);
        edges.push(edge(n, CHAIN + 1, "sum", n, "watch"));
    }
    let mut source = Source::new(nodes, edges);
    source.variables = vec![run_variable(1, U64, Some(json!("0")))];
    let (package, snapshot, doc, analysis) = source.checked();
    let row = measure(
        "compile 512 watches over one 256-node data cone".into(),
        WATCHES as u64,
        "watches",
        config,
        || {
            sample(|| {
                let plan = compile_control(
                    &doc,
                    snapshot.clone(),
                    &analysis,
                    &package,
                    CompileOptions::default(),
                )
                .unwrap();
                black_box(&plan).watches().len()
            })
        },
        |watches| assert_eq!(watches, WATCHES as usize),
    );
    print_table(&[row]);
}
