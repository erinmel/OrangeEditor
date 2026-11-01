//! cargo run -p orange-bundled --example headless_authoring --locked
//! A small host-side composition of retained authoring, history, compilation and execution.
//! The channel calls block to keep this demonstrator deterministic; an editor should submit
//! without blocking and poll bounded acknowledgements on its UI thread.

use orange_bundled::{MathPackage, math_definitions};
use orange_compiler::{CompileOptions, ControlPlan, compile_control};
use orange_model::catalog::{Ports, RegistryBuilder, RegistrySnapshot};
use orange_model::validation::{AnalysisOptions, AnalysisSession, HostOrigin};
use orange_model::{DocCommand, GraphDocument, Inverse, LayoutCommand, Value, from_json};
use orange_runtime::{Run, RunOptions, StopMode};
use serde_json::{Value as Json, json};
use std::collections::BTreeMap;
use std::error::Error;
use std::io;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::thread;

type BoxError = Box<dyn Error + Send + Sync>;

enum Command {
    SetB(u64),
    Undo,
    Redo,
    SetLayout,
    StartCurrent,
    PumpOld,
    StopOld,
    StartFresh,
    Shutdown,
}

struct Request {
    sequence: u64,
    command: Command,
}

#[derive(Debug)]
struct Ack {
    sequence: u64,
    revision: u64,
    analysis_current: bool,
    plan_current: Option<bool>,
    layout_was_no_work: bool,
    total: Option<u64>,
}

struct HostState {
    document: GraphDocument,
    analysis: AnalysisSession,
    undo: Vec<Inverse>,
    redo: Vec<Inverse>,
    plan: Option<Arc<ControlPlan>>,
    run: Option<Run>,
    last_total: Option<u64>,
    registry: Arc<RegistrySnapshot>,
    package: MathPackage,
}

fn id(n: u32) -> String {
    format!("10000000-0000-4000-8000-{n:012x}")
}

fn core(n: u32, name: &str) -> Json {
    json!({"id":id(n),"definition_id":format!("core/{name}"),"definition_version":1,
        "config_version":1,"config":{}})
}

fn edge(n: u32, from: u32, output: &str, to: u32, input: &str) -> Json {
    json!({"id":format!("30000000-0000-4000-8000-{n:012x}"),
        "from":{"node":id(from),"port":output},"to":{"node":id(to),"port":input}})
}

fn setup(package: MathPackage, registry: Arc<RegistrySnapshot>) -> Result<HostState, BoxError> {
    let add = math_definitions()?
        .into_iter()
        .find(|definition| definition.definition.id.as_str() == "org.orange.math/add")
        .expect("bundled Add definition");
    let Ports::Declared(ports) = add.ports else {
        unreachable!("bundled Add has declared ports")
    };
    let args = BTreeMap::from([("T".into(), "core/u64@1".parse()?)]);
    let signature = orange_model::signature::project(add.kind, ports, &args);

    let mut start = core(1, "on-start");
    start["activation"] = json!({"mode":"queue","capacity":1,
        "overflow":"drop-newest","on_fault":"continue","order":0});
    let key = package.provider_key();
    let add = json!({"id":id(2),"definition_id":"org.orange.math/add",
        "definition_version":1,"config_version":1,"config":{"int_overflow":"checked"},
        "signature":"add-u64","provider":{"package":key.package,"version":key.version,"id":key.id},
        "inputs":{"a":{"type":"core/u64@1","value":"1"},
            "b":{"type":"core/u64@1","value":"2"}}});
    let mut set = core(3, "set-variable");
    set["config"] = json!({"variable":id(100)});
    let document = from_json(
        &json!({"format":"orange.graph","format_version":1,
            "graph_id":"20000000-0000-4000-8000-000000000001","revision":0,
            "package_lock":[{"id":key.package,"version":key.version,"content_digest":key.content_digest}],
            "signatures":{"add-u64":signature},
            "variables":[{"id":id(100),"name":"total","type":"core/u64@1",
                "scope":"run","init":{"type":"core/u64@1","value":"0"}}],
            "nodes":[start,add,set],
            "edges":[edge(1,1,"then",3,"in"),edge(2,2,"sum",3,"value")]})
            .to_string(),
    )?;
    let analysis = AnalysisSession::new(
        &document,
        &registry,
        BTreeMap::new(),
        AnalysisOptions::default(),
    );
    if !analysis.analysis().report().is_valid() {
        return Err(error("example graph should validate").into());
    }
    Ok(HostState {
        document,
        analysis,
        undo: Vec::new(),
        redo: Vec::new(),
        plan: None,
        run: None,
        last_total: None,
        registry,
        package,
    })
}

fn host_registry() -> Result<(MathPackage, Arc<RegistrySnapshot>), BoxError> {
    // Illustrative build identity. A production host authenticates actual provider bytes.
    let digest = format!("sha256:{}", "e".repeat(64));
    let package = MathPackage::new(digest.clone())?;
    let registry = package
        .register(RegistryBuilder::core(HostOrigin::new("0.0.0", digest)?))?
        .build()?;
    Ok((package, registry))
}

fn error(message: &str) -> io::Error {
    io::Error::other(message.to_owned())
}

fn pump_to_idle(run: &mut Run) -> Result<(), BoxError> {
    loop {
        let progress = run.pump(64)?;
        if !progress.ready {
            if let Some(at) = progress.next_wakeup {
                run.advance_to(at)?;
            } else {
                return Ok(());
            }
        }
    }
}

fn compile(state: &HostState) -> Result<Arc<ControlPlan>, BoxError> {
    Ok(Arc::new(compile_control(
        &state.document,
        state.registry.clone(),
        state.analysis.analysis(),
        &state.package,
        CompileOptions::default(),
    )?))
}

fn set_b(state: &mut HostState, value: u64) -> Result<(), BoxError> {
    let committed = state.document.apply([DocCommand::SetLiteral {
        port: orange_model::document::PortRef {
            node: id(2).parse()?,
            port: "b".parse()?,
        },
        literal: Some(serde_json::from_value(json!({
            "type":"core/u64@1","value":value.to_string()
        }))?),
    }])?;
    state.analysis.update(
        &state.document,
        &state.registry,
        &committed.change_set,
        AnalysisOptions::default(),
    );
    state.undo.push(committed.inverse);
    state.redo.clear();
    Ok(())
}

fn history(state: &mut HostState, undoing: bool) -> Result<(), BoxError> {
    let inverse = if undoing {
        state.undo.pop()
    } else {
        state.redo.pop()
    };
    let inverse = inverse.ok_or_else(|| error("history is empty"))?;
    let committed = state.document.revert(inverse)?;
    state.analysis.update(
        &state.document,
        &state.registry,
        &committed.change_set,
        AnalysisOptions::default(),
    );
    if undoing {
        state.redo.push(committed.inverse);
    } else {
        state.undo.push(committed.inverse);
    }
    Ok(())
}

fn execute(
    state: &mut HostState,
    sequence: u64,
    command: Command,
) -> Result<(Ack, bool), BoxError> {
    let mut layout_was_no_work = false;
    match command {
        Command::SetB(value) => set_b(state, value)?,
        Command::Undo => history(state, true)?,
        Command::Redo => history(state, false)?,
        Command::SetLayout => {
            let committed = state
                .document
                .apply([DocCommand::Layout(LayoutCommand::Set {
                    path: vec!["nodes".into(), id(2), "position".into()],
                    value: Some(json!({"x": 20, "y": 30})),
                })])?;
            let work = state.analysis.update(
                &state.document,
                &state.registry,
                &committed.change_set,
                AnalysisOptions::default(),
            );
            layout_was_no_work = work.work == orange_model::validation::SessionWork::None;
            state.undo.push(committed.inverse);
            state.redo.clear();
        }
        Command::StartCurrent => {
            let plan = compile(state)?;
            state.run = Some(Run::start(
                Arc::clone(&plan),
                &state.document,
                &state.registry,
                42,
                RunOptions::default(),
            )?);
            state.plan = Some(plan);
        }
        Command::PumpOld => {
            let run = state.run.as_mut().ok_or_else(|| error("run is missing"))?;
            pump_to_idle(run)?;
            state.last_total = match state
                .run
                .as_ref()
                .and_then(|run| run.variable(id(100).parse().ok()?))
            {
                Some(Value::U64(value)) => Some(*value),
                _ => None,
            };
        }
        Command::StopOld => {
            let mut run = state.run.take().ok_or_else(|| error("run is missing"))?;
            run.request_stop(StopMode::Graceful)?;
            if run.state() != orange_runtime::RunState::Stopped {
                pump_to_idle(&mut run)?;
            }
            run.dispose()?;
            state.run = None;
        }
        Command::StartFresh => {
            let plan = compile(state)?;
            let mut run = Run::start(
                Arc::clone(&plan),
                &state.document,
                &state.registry,
                43,
                RunOptions::default(),
            )?;
            pump_to_idle(&mut run)?;
            let total = match run.variable(id(100).parse()?) {
                Some(Value::U64(value)) => *value,
                _ => return Err(error("fresh run did not publish a u64 total").into()),
            };
            if total != 8 {
                return Err(error("fresh run should publish total=8").into());
            }
            run.request_stop(StopMode::Graceful)?;
            if run.state() != orange_runtime::RunState::Stopped {
                pump_to_idle(&mut run)?;
            }
            run.dispose()?;
            state.plan = Some(plan);
            state.last_total = Some(total);
        }
        Command::Shutdown => {
            if let Some(mut run) = state.run.take() {
                run.request_stop(StopMode::Graceful)?;
                if run.state() != orange_runtime::RunState::Stopped {
                    pump_to_idle(&mut run)?;
                }
                run.dispose()?;
            }
            return Ok((ack(state, sequence, layout_was_no_work), true));
        }
    }
    Ok((ack(state, sequence, layout_was_no_work), false))
}

fn ack(state: &HostState, sequence: u64, layout_was_no_work: bool) -> Ack {
    Ack {
        sequence,
        revision: state.document.revision(),
        analysis_current: state
            .analysis
            .analysis()
            .is_current(&state.document, &state.registry),
        plan_current: state
            .plan
            .as_ref()
            .map(|plan| plan.is_current(&state.document, &state.registry)),
        layout_was_no_work,
        total: state.last_total.or_else(|| {
            state
                .run
                .as_ref()
                .and_then(|run| match run.variable(id(100).parse().ok()?) {
                    Some(Value::U64(value)) => Some(*value),
                    _ => None,
                })
        }),
    }
}

fn worker(
    requests: Receiver<Request>,
    replies: SyncSender<Result<Ack, String>>,
    registry: Arc<RegistrySnapshot>,
    package: MathPackage,
) -> Result<(), BoxError> {
    let mut state = setup(package, registry)?;
    let mut expected = 1;
    while let Ok(request) = requests.recv() {
        if request.sequence != expected {
            let cause = error("request sequence is out of order");
            let _ = replies.send(Err(cause.to_string()));
            return Err(cause.into());
        }
        expected += 1;
        let (reply, shutdown) = match execute(&mut state, request.sequence, request.command) {
            Ok(result) => result,
            Err(cause) => {
                let _ = replies.send(Err(cause.to_string()));
                return Err(cause);
            }
        };
        replies
            .send(Ok(reply))
            .map_err(|_| error("host dropped reply channel"))?;
        if shutdown {
            return Ok(());
        }
    }
    Err(error("host dropped request channel before shutdown").into())
}

fn send(
    requests: &SyncSender<Request>,
    replies: &Receiver<Result<Ack, String>>,
    sequence: u64,
    command: Command,
) -> Result<Ack, BoxError> {
    requests
        .send(Request { sequence, command })
        .map_err(|_| error("authoring worker stopped"))?;
    let ack = replies
        .recv()
        .map_err(|_| error("authoring worker dropped reply"))?
        .map_err(|message| error(&message))?;
    if ack.sequence != sequence || !ack.analysis_current {
        return Err(error("worker acknowledgement is stale or out of order").into());
    }
    Ok(ack)
}

fn workflow() -> Result<(), BoxError> {
    let (package, registry) = host_registry()?;
    thread::scope(|scope| -> Result<(), BoxError> {
        let (request_tx, request_rx) = sync_channel(1);
        let (reply_tx, reply_rx) = sync_channel(1);
        let handle = scope.spawn(move || worker(request_rx, reply_tx, registry, package));

        let conversation = (|| -> Result<_, BoxError> {
            let edit = send(&request_tx, &reply_rx, 1, Command::SetB(3))?;
            let undo = send(&request_tx, &reply_rx, 2, Command::Undo)?;
            let redo = send(&request_tx, &reply_rx, 3, Command::Redo)?;
            let layout = send(&request_tx, &reply_rx, 4, Command::SetLayout)?;
            let old = send(&request_tx, &reply_rx, 5, Command::StartCurrent)?;
            let changed = send(&request_tx, &reply_rx, 6, Command::SetB(7))?;
            let pumped = send(&request_tx, &reply_rx, 7, Command::PumpOld)?;
            let stopped = send(&request_tx, &reply_rx, 8, Command::StopOld)?;
            let fresh = send(&request_tx, &reply_rx, 9, Command::StartFresh)?;
            send(&request_tx, &reply_rx, 10, Command::Shutdown)?;
            Ok((
                edit, undo, redo, layout, old, changed, pumped, stopped, fresh,
            ))
        })();
        drop(request_tx);
        let worker_result = handle
            .join()
            .map_err(|_| error("authoring worker panicked"))?;
        let (edit, undo, redo, layout, old, changed, pumped, stopped, fresh) = conversation?;
        worker_result?;

        assert!(edit.revision < undo.revision && undo.revision < redo.revision);
        assert!(layout.layout_was_no_work);
        assert_eq!(old.plan_current, Some(true));
        assert_eq!(changed.plan_current, Some(false));
        assert_eq!(pumped.total, Some(4));
        assert_eq!(stopped.plan_current, Some(false));
        assert_eq!(fresh.total, Some(8));
        println!(
            "authoring: undo/redo current; layout no-work; old=4 stale; fresh=8; worker joined"
        );
        Ok(())
    })
}

fn main() -> Result<(), BoxError> {
    workflow()
}

#[cfg(test)]
mod tests {
    #[test]
    fn background_authoring_preserves_freshness_history_and_running_plan() {
        super::workflow().unwrap();
    }

    #[test]
    fn worker_reports_bad_sequence_and_exits_when_host_disconnects() {
        use std::sync::mpsc::sync_channel;
        use std::thread;

        let (package, registry) = super::host_registry().unwrap();
        thread::scope(|scope| {
            let (request_tx, request_rx) = sync_channel(1);
            let (reply_tx, reply_rx) = sync_channel(1);
            let worker_package = package.clone();
            let worker_registry = std::sync::Arc::clone(&registry);
            let handle = scope.spawn(move || {
                super::worker(request_rx, reply_tx, worker_registry, worker_package)
            });
            request_tx
                .send(super::Request {
                    sequence: 2,
                    command: super::Command::Shutdown,
                })
                .unwrap();
            assert!(
                matches!(reply_rx.recv().unwrap(), Err(message) if message.contains("out of order"))
            );
            assert!(handle.join().unwrap().is_err());

            let (request_tx, request_rx) = sync_channel(1);
            let (reply_tx, reply_rx) = sync_channel(1);
            let worker_package = package.clone();
            let worker_registry = std::sync::Arc::clone(&registry);
            let handle = scope.spawn(move || {
                super::worker(request_rx, reply_tx, worker_registry, worker_package)
            });
            drop(request_tx);
            assert!(reply_rx.recv().is_err());
            assert!(handle.join().unwrap().is_err());
        });
    }
}
