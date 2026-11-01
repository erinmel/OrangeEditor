//! cargo run -p orange-bundled --example headless_authoring --locked
//! A deterministic host-side composition of retained authoring, history and execution.

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
const FINALIZE_PUMP_SLICES: usize = 32;
const MAX_HISTORY: usize = 64;

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
    #[cfg(test)]
    FatalExecutionForTest,
    #[cfg(test)]
    PanicExecutionForTest,
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
    active_run_id: Option<u64>,
    active_inspection_current: Option<bool>,
    active_current_total: Option<u64>,
    active_inspected_total: Option<u64>,
    terminal_run_id: Option<u64>,
    terminal_total: Option<u64>,
    terminal_stopped: Option<bool>,
    layout_was_no_work: bool,
    waiting: Option<bool>,
}

struct RunSlot {
    id: u64,
    plan: Arc<ControlPlan>,
    run: Run,
    inspected_total: Option<u64>,
    wait_entered: bool,
}

#[derive(Clone, Copy, Debug)]
struct TerminalInspection {
    run_id: u64,
    total: Option<u64>,
    stopped_marker: Option<bool>,
}

#[derive(Default, Debug)]
struct CleanupSummary {
    stopped_runs: usize,
    disposed_runs: usize,
    total_at_cleanup: Option<u64>,
    stopped_marker_at_cleanup: Option<bool>,
    pending_after_cleanup: Option<usize>,
    reached_stopped_state: bool,
    failure: Option<String>,
}

struct HostState {
    document: GraphDocument,
    analysis: AnalysisSession,
    undo: Vec<Inverse>,
    redo: Vec<Inverse>,
    run_slot: Option<RunSlot>,
    terminal: Option<TerminalInspection>,
    next_run_id: u64,
    registry: Arc<RegistrySnapshot>,
    package: MathPackage,
    cleanup: CleanupSummary,
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
    let mut wait = core(4, "wait");
    wait["inputs"] = json!({"duration":{"type":"core/duration@1","value":"100"}});
    let mut on_stop = core(5, "on-stop");
    on_stop["activation"] = json!({"mode":"queue","capacity":1,
        "overflow":"drop-newest","on_fault":"continue","order":0});
    let mut mark_stopped = core(6, "set-variable");
    mark_stopped["config"] = json!({"variable":id(101)});
    mark_stopped["inputs"] = json!({"value":{"type":"core/bool@1","value":true}});
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
                "scope":"run","init":{"type":"core/u64@1","value":"0"}},
                {"id":id(101),"name":"stopped","type":"core/bool@1",
                "scope":"run","init":{"type":"core/bool@1","value":false}}],
            "nodes":[start,add,set,wait,on_stop,mark_stopped],
            "edges":[edge(1,1,"then",4,"in"),edge(2,4,"then",3,"in"),
                edge(3,2,"sum",3,"value"),edge(4,5,"then",6,"in")]})
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
        run_slot: None,
        terminal: None,
        next_run_id: 1,
        registry,
        package,
        cleanup: CleanupSummary::default(),
    })
}

fn host_registry() -> Result<(MathPackage, Arc<RegistrySnapshot>), BoxError> {
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

fn pump_until_wait(run: &mut Run) -> Result<bool, BoxError> {
    for _ in 0..64 {
        let progress = run.pump(1)?;
        if progress.next_wakeup.is_some() {
            return Ok(true);
        }
        if !progress.ready {
            return Ok(false);
        }
    }
    Err(error("run did not enter a wait within the bounded pump phase").into())
}

fn read_total(run: &Run) -> Option<u64> {
    match run.variable(id(100).parse().ok()?) {
        Some(Value::U64(value)) => Some(*value),
        _ => None,
    }
}

fn read_stopped(run: &Run) -> Option<bool> {
    match run.variable(id(101).parse().ok()?) {
        Some(Value::Bool(value)) => Some(*value),
        _ => None,
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
    remember(&mut state.undo, committed.inverse);
    state.redo.clear();
    Ok(())
}

fn remember(history: &mut Vec<Inverse>, inverse: Inverse) {
    if history.len() == MAX_HISTORY {
        history.remove(0);
    }
    history.push(inverse);
}

fn history(state: &mut HostState, undoing: bool) -> Result<(), BoxError> {
    let source = if undoing {
        &mut state.undo
    } else {
        &mut state.redo
    };
    let inverse = source.pop().ok_or_else(|| error("history is empty"))?;
    let committed = state.document.revert(inverse)?;
    state.analysis.update(
        &state.document,
        &state.registry,
        &committed.change_set,
        AnalysisOptions::default(),
    );
    if undoing {
        remember(&mut state.redo, committed.inverse);
    } else {
        remember(&mut state.undo, committed.inverse);
    }
    Ok(())
}

#[derive(Debug)]
enum CommandError {
    Recoverable(String),
    Fatal(BoxError),
}

impl From<BoxError> for CommandError {
    fn from(value: BoxError) -> Self {
        Self::Fatal(value)
    }
}

impl From<orange_runtime::RunError> for CommandError {
    fn from(value: orange_runtime::RunError) -> Self {
        Self::Fatal(Box::new(value))
    }
}

fn fatal<E: Error + Send + Sync + 'static>(value: E) -> CommandError {
    CommandError::Fatal(Box::new(value))
}

fn execute(
    state: &mut HostState,
    sequence: u64,
    command: Command,
) -> Result<(Ack, bool), CommandError> {
    let mut layout_was_no_work = false;
    match command {
        Command::SetB(value) => set_b(state, value)?,
        Command::Undo => {
            history(state, true).map_err(|cause| CommandError::Recoverable(cause.to_string()))?
        }
        Command::Redo => {
            history(state, false).map_err(|cause| CommandError::Recoverable(cause.to_string()))?
        }
        Command::SetLayout => {
            let committed = state
                .document
                .apply([DocCommand::Layout(LayoutCommand::Set {
                    path: vec!["nodes".into(), id(2), "position".into()],
                    value: Some(json!({"x": 20, "y": 30})),
                })])
                .map_err(fatal)?;
            let work = state.analysis.update(
                &state.document,
                &state.registry,
                &committed.change_set,
                AnalysisOptions::default(),
            );
            layout_was_no_work = work.work == orange_model::validation::SessionWork::None;
            remember(&mut state.undo, committed.inverse);
            state.redo.clear();
        }
        Command::StartCurrent => {
            if state.run_slot.is_some() {
                return Err(CommandError::Recoverable(
                    "activeRun: stop the current run before starting another".into(),
                ));
            }
            let plan = compile(state)?;
            let run = Run::start(
                Arc::clone(&plan),
                &state.document,
                &state.registry,
                42,
                RunOptions::default(),
            )?;
            let run_id = state.next_run_id;
            state.next_run_id += 1;
            state.run_slot = Some(RunSlot {
                id: run_id,
                plan,
                run,
                inspected_total: None,
                wait_entered: false,
            });
            let slot = state.run_slot.as_mut().expect("run slot just installed");
            slot.wait_entered = pump_until_wait(&mut slot.run)?;
            if !slot.wait_entered {
                return Err(fatal(error("run did not enter its expected wait")));
            }
        }
        Command::PumpOld => {
            let slot = state
                .run_slot
                .as_mut()
                .ok_or_else(|| CommandError::Recoverable("run is missing".into()))?;
            pump_to_idle(&mut slot.run)?;
            slot.inspected_total = read_total(&slot.run);
            slot.wait_entered = false;
        }
        Command::StopOld => {
            let slot = state
                .run_slot
                .as_mut()
                .ok_or_else(|| CommandError::Recoverable("run is missing".into()))?;
            slot.run.request_stop(StopMode::Graceful)?;
            if slot.run.state() != orange_runtime::RunState::Stopped {
                pump_to_idle(&mut slot.run)?;
            }
            let terminal = TerminalInspection {
                run_id: slot.id,
                total: read_total(&slot.run).or(slot.inspected_total),
                stopped_marker: read_stopped(&slot.run),
            };
            slot.run.dispose()?;
            state.run_slot = None;
            state.terminal = Some(terminal);
            state.cleanup.stopped_runs += 1;
            state.cleanup.disposed_runs += 1;
        }
        Command::StartFresh => {
            if state.run_slot.is_some() {
                return Err(CommandError::Recoverable(
                    "activeRun: stop the current run before starting another".into(),
                ));
            }
            let plan = compile(state)?;
            let run = Run::start(
                Arc::clone(&plan),
                &state.document,
                &state.registry,
                43,
                RunOptions::default(),
            )?;
            let run_id = state.next_run_id;
            state.next_run_id += 1;
            state.run_slot = Some(RunSlot {
                id: run_id,
                plan,
                run,
                inspected_total: None,
                wait_entered: false,
            });
            let slot = state.run_slot.as_mut().expect("run slot just installed");
            pump_to_idle(&mut slot.run)?;
            let total = read_total(&slot.run)
                .ok_or_else(|| fatal(error("fresh run did not publish a u64 total")))?;
            slot.run.request_stop(StopMode::Graceful)?;
            if slot.run.state() != orange_runtime::RunState::Stopped {
                pump_to_idle(&mut slot.run)?;
            }
            let terminal = TerminalInspection {
                run_id: slot.id,
                total: Some(total),
                stopped_marker: read_stopped(&slot.run),
            };
            slot.run.dispose()?;
            state.run_slot = None;
            state.terminal = Some(terminal);
            state.cleanup.stopped_runs += 1;
            state.cleanup.disposed_runs += 1;
        }
        Command::Shutdown => {
            return Ok((ack(state, sequence, layout_was_no_work), true));
        }
        #[cfg(test)]
        Command::FatalExecutionForTest => {
            return Err(fatal(error("injected fatal execution error")));
        }
        #[cfg(test)]
        Command::PanicExecutionForTest => panic!("injected host panic"),
    }
    Ok((ack(state, sequence, layout_was_no_work), false))
}

fn ack(state: &HostState, sequence: u64, layout_was_no_work: bool) -> Ack {
    let slot = state.run_slot.as_ref();
    Ack {
        sequence,
        revision: state.document.revision(),
        analysis_current: state
            .analysis
            .analysis()
            .is_current(&state.document, &state.registry),
        plan_current: slot.map(|slot| slot.plan.is_current(&state.document, &state.registry)),
        active_run_id: slot.map(|slot| slot.id),
        active_inspection_current: slot.map(|slot| {
            slot.plan.is_current(&state.document, &state.registry) && slot.inspected_total.is_some()
        }),
        active_current_total: slot.and_then(|slot| read_total(&slot.run)),
        active_inspected_total: slot.and_then(|slot| slot.inspected_total),
        terminal_run_id: state.terminal.map(|terminal| terminal.run_id),
        terminal_total: state.terminal.and_then(|terminal| terminal.total),
        terminal_stopped: state.terminal.and_then(|terminal| terminal.stopped_marker),
        layout_was_no_work,
        waiting: slot.map(|slot| slot.wait_entered),
    }
}

fn finalize_run(state: &mut HostState, mode: StopMode) {
    let Some(mut slot) = state.run_slot.take() else {
        return;
    };
    if slot.run.state() != orange_runtime::RunState::Stopped {
        if let Err(cause) = slot.run.request_stop(mode) {
            state.cleanup.failure = Some(format!("request stop: {cause}"));
        }
        for _ in 0..FINALIZE_PUMP_SLICES {
            if slot.run.state() == orange_runtime::RunState::Stopped {
                break;
            }
            match slot.run.pump(64) {
                Ok(progress) if !progress.ready => {
                    if let Some(at) = progress.next_wakeup {
                        if let Err(cause) = slot.run.advance_to(at) {
                            state.cleanup.failure =
                                Some(format!("advance during finalization: {cause}"));
                            break;
                        }
                    } else {
                        break;
                    }
                }
                Ok(_) => {}
                Err(cause) => {
                    state.cleanup.failure = Some(format!("pump during finalization: {cause}"));
                    break;
                }
            }
        }
    }
    state.cleanup.total_at_cleanup = read_total(&slot.run).or(slot.inspected_total);
    state.cleanup.stopped_marker_at_cleanup = read_stopped(&slot.run);
    state.cleanup.pending_after_cleanup = Some(slot.run.pending_continuations());
    if slot.run.state() == orange_runtime::RunState::Stopped {
        state.cleanup.reached_stopped_state = true;
        state.cleanup.stopped_runs += 1;
    } else {
        state.cleanup.failure.get_or_insert_with(|| {
            format!(
                "finalization limit reached with run state {:?}",
                slot.run.state()
            )
        });
    }
    if slot.run.dispose().is_ok() {
        state.cleanup.disposed_runs += 1;
    } else {
        state
            .cleanup
            .failure
            .get_or_insert_with(|| "run dispose failed".into());
    }
}

struct WorkerReport {
    result: Result<(), String>,
    cleanup: CleanupSummary,
}

fn worker(
    requests: Receiver<Request>,
    replies: SyncSender<Result<Ack, String>>,
    registry: Arc<RegistrySnapshot>,
    package: MathPackage,
) -> WorkerReport {
    let mut state = match setup(package, registry) {
        Ok(state) => state,
        Err(cause) => {
            return WorkerReport {
                result: Err(cause.to_string()),
                cleanup: CleanupSummary::default(),
            };
        }
    };
    let mut expected = 1;
    // An unwind ends this host session. AssertUnwindSafe permits only stop/discard
    // afterward; no editing, analysis reuse or normal execution resumes after a panic.
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        'worker: loop {
            let request = match requests.recv() {
                Ok(request) => request,
                Err(_) => break Err("host dropped request channel before shutdown".into()),
            };
            if request.sequence != expected {
                let message = "request sequence is out of order".to_owned();
                let _ = replies.send(Err(message.clone()));
                break Err(message);
            }
            expected += 1;
            let (reply, shutdown) = match execute(&mut state, request.sequence, request.command) {
                Ok(result) => result,
                Err(CommandError::Recoverable(message)) => {
                    if replies.send(Err(message)).is_err() {
                        break Err("host dropped reply channel".into());
                    }
                    continue 'worker;
                }
                Err(CommandError::Fatal(cause)) => {
                    let message = cause.to_string();
                    let _ = replies.send(Err(message.clone()));
                    break Err(message);
                }
            };
            if replies.send(Ok(reply)).is_err() {
                break Err("host dropped reply channel".into());
            }
            if shutdown {
                break Ok(());
            }
        }
    }));
    let result: Result<(), String> =
        outcome.unwrap_or_else(|_| Err("authoring worker panicked; session ended".into()));
    let mode = if result.is_ok() {
        StopMode::Graceful
    } else {
        StopMode::Immediate
    };
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        finalize_run(&mut state, mode)
    }))
    .is_err()
    {
        state.cleanup.failure = Some("cleanup panicked; run discarded".into());
    }
    let result = match (result, state.cleanup.failure.as_ref()) {
        (Ok(()), Some(failure)) => Err(format!("worker cleanup failed: {failure}")),
        (Err(cause), Some(failure)) => Err(format!("{cause}; worker cleanup failed: {failure}")),
        (result, None) => result,
    };
    WorkerReport {
        result,
        cleanup: state.cleanup,
    }
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
            let active_reject = send(&request_tx, &reply_rx, 6, Command::StartFresh).unwrap_err();
            assert!(active_reject.to_string().contains("activeRun"));
            let changed = send(&request_tx, &reply_rx, 7, Command::SetB(7))?;
            let pumped = send(&request_tx, &reply_rx, 8, Command::PumpOld)?;
            let stopped = send(&request_tx, &reply_rx, 9, Command::StopOld)?;
            let restarted = send(&request_tx, &reply_rx, 10, Command::StartCurrent)?;
            let restarted_pump = send(&request_tx, &reply_rx, 11, Command::PumpOld)?;
            let restarted_stop = send(&request_tx, &reply_rx, 12, Command::StopOld)?;
            let fresh = send(&request_tx, &reply_rx, 13, Command::StartFresh)?;
            send(&request_tx, &reply_rx, 14, Command::Shutdown)?;
            Ok((
                edit,
                undo,
                redo,
                layout,
                old,
                changed,
                pumped,
                stopped,
                restarted,
                restarted_pump,
                restarted_stop,
                fresh,
            ))
        })();
        drop(request_tx);
        drop(reply_rx);
        let report = handle
            .join()
            .map_err(|_| error("authoring worker panicked"))?;
        let (
            edit,
            undo,
            redo,
            layout,
            old,
            changed,
            pumped,
            stopped,
            restarted,
            restarted_pump,
            restarted_stop,
            fresh,
        ) = conversation?;
        report.result.map_err(|message| error(&message))?;
        assert!(edit.revision < undo.revision && undo.revision < redo.revision);
        assert!(layout.layout_was_no_work);
        assert_eq!(old.plan_current, Some(true));
        assert_eq!(old.active_inspection_current, Some(false));
        assert_eq!(old.waiting, Some(true));
        assert_eq!(changed.plan_current, Some(false));
        assert_eq!(pumped.active_inspected_total, Some(4));
        assert_eq!(pumped.active_inspection_current, Some(false));
        assert_eq!(stopped.terminal_total, Some(4));
        assert_eq!(stopped.terminal_stopped, Some(true));
        assert_eq!(restarted.active_current_total, Some(0));
        assert_eq!(restarted.terminal_total, Some(4));
        assert_ne!(restarted.active_run_id, restarted.terminal_run_id);
        assert_eq!(restarted.waiting, Some(true));
        assert_eq!(restarted_pump.active_inspected_total, Some(8));
        assert_eq!(restarted_stop.terminal_total, Some(8));
        assert_eq!(fresh.terminal_total, Some(8));
        assert_eq!(fresh.terminal_stopped, Some(true));
        assert_eq!(report.cleanup.stopped_runs, 3);
        assert_eq!(report.cleanup.disposed_runs, 3);
        assert_eq!(report.cleanup.failure, None);
        println!(
            "authoring: undo/redo current; wait resumed old=4; restart initialized=0 then 8; On Stop marker set"
        );
        Ok(())
    })
}

fn main() -> Result<(), BoxError> {
    workflow()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authoring_keeps_plan_run_and_inspection_bound_together() {
        workflow().unwrap();
    }

    #[test]
    fn empty_undo_is_recoverable_and_worker_keeps_serving() {
        let (package, registry) = host_registry().unwrap();
        thread::scope(|scope| {
            let (request_tx, request_rx) = sync_channel(1);
            let (reply_tx, reply_rx) = sync_channel(1);
            let handle = scope.spawn(move || worker(request_rx, reply_tx, registry, package));
            request_tx
                .send(Request {
                    sequence: 1,
                    command: Command::Undo,
                })
                .unwrap();
            assert!(
                matches!(reply_rx.recv().unwrap(), Err(message) if message.contains("history is empty"))
            );
            request_tx
                .send(Request {
                    sequence: 2,
                    command: Command::SetB(9),
                })
                .unwrap();
            assert!(reply_rx.recv().unwrap().is_ok());
            request_tx
                .send(Request {
                    sequence: 3,
                    command: Command::Shutdown,
                })
                .unwrap();
            assert!(reply_rx.recv().unwrap().is_ok());
            let report = handle.join().unwrap();
            assert!(report.result.is_ok());
            assert_eq!(report.cleanup.disposed_runs, 0);
        });
    }

    #[test]
    fn disconnect_and_bad_sequence_finalize_active_run_once() {
        let (package, registry) = host_registry().unwrap();
        thread::scope(|scope| {
            for bad_sequence in [false, true] {
                let (request_tx, request_rx) = sync_channel(1);
                let (reply_tx, reply_rx) = sync_channel(1);
                let handle = scope.spawn({
                    let package = package.clone();
                    let registry = Arc::clone(&registry);
                    move || worker(request_rx, reply_tx, registry, package)
                });
                request_tx
                    .send(Request {
                        sequence: 1,
                        command: Command::StartCurrent,
                    })
                    .unwrap();
                let started = reply_rx.recv().unwrap().unwrap();
                assert_eq!(started.waiting, Some(true));
                if bad_sequence {
                    request_tx
                        .send(Request {
                            sequence: 3,
                            command: Command::Shutdown,
                        })
                        .unwrap();
                    assert!(
                        matches!(reply_rx.recv().unwrap(), Err(message) if message.contains("out of order"))
                    );
                } else {
                    drop(request_tx);
                    assert!(reply_rx.recv().is_err());
                }
                let report = handle.join().unwrap();
                assert!(report.result.is_err());
                assert_eq!(report.cleanup.stopped_runs, 1);
                assert_eq!(report.cleanup.disposed_runs, 1);
                assert!(report.cleanup.reached_stopped_state);
                assert_eq!(report.cleanup.pending_after_cleanup, Some(0));
                assert_eq!(report.cleanup.stopped_marker_at_cleanup, Some(false));
                assert_eq!(report.cleanup.failure, None);
            }
        });
    }

    #[test]
    fn reply_disconnect_finalizes_active_run() {
        let (package, registry) = host_registry().unwrap();
        thread::scope(|scope| {
            let (request_tx, request_rx) = sync_channel(1);
            let (reply_tx, reply_rx) = sync_channel(1);
            let handle = scope.spawn(move || worker(request_rx, reply_tx, registry, package));
            request_tx
                .send(Request {
                    sequence: 1,
                    command: Command::StartCurrent,
                })
                .unwrap();
            let started = reply_rx.recv().unwrap().unwrap();
            assert_eq!(started.waiting, Some(true));
            drop(reply_rx);
            drop(request_tx);
            let report = handle.join().unwrap();
            assert!(report.result.is_err());
            assert_eq!(report.cleanup.stopped_runs, 1);
            assert_eq!(report.cleanup.disposed_runs, 1);
            assert_eq!(report.cleanup.total_at_cleanup, Some(0));
            assert!(report.cleanup.reached_stopped_state);
            assert_eq!(report.cleanup.pending_after_cleanup, Some(0));
            assert_eq!(report.cleanup.stopped_marker_at_cleanup, Some(false));
            assert_eq!(report.cleanup.failure, None);
        });
    }

    #[test]
    fn fatal_execution_error_finalizes_active_run() {
        let (package, registry) = host_registry().unwrap();
        thread::scope(|scope| {
            let (request_tx, request_rx) = sync_channel(1);
            let (reply_tx, reply_rx) = sync_channel(1);
            let handle = scope.spawn(move || worker(request_rx, reply_tx, registry, package));
            request_tx
                .send(Request {
                    sequence: 1,
                    command: Command::StartCurrent,
                })
                .unwrap();
            let started = reply_rx.recv().unwrap().unwrap();
            assert_eq!(started.waiting, Some(true));
            request_tx
                .send(Request {
                    sequence: 2,
                    command: Command::FatalExecutionForTest,
                })
                .unwrap();
            assert!(
                matches!(reply_rx.recv().unwrap(), Err(message) if message.contains("fatal execution"))
            );
            let report = handle.join().unwrap();
            assert!(report.result.is_err());
            assert_eq!(report.cleanup.stopped_runs, 1);
            assert_eq!(report.cleanup.disposed_runs, 1);
            assert!(report.cleanup.reached_stopped_state);
            assert_eq!(report.cleanup.pending_after_cleanup, Some(0));
            assert_eq!(report.cleanup.stopped_marker_at_cleanup, Some(false));
            assert_eq!(report.cleanup.failure, None);
        });
    }

    #[test]
    fn active_run_rejects_repeated_start_without_replacement() {
        let (package, registry) = host_registry().unwrap();
        thread::scope(|scope| {
            let (request_tx, request_rx) = sync_channel(1);
            let (reply_tx, reply_rx) = sync_channel(1);
            let handle = scope.spawn(move || worker(request_rx, reply_tx, registry, package));
            for (sequence, command) in [
                (1, Command::StartCurrent),
                (2, Command::StartCurrent),
                (3, Command::PumpOld),
            ] {
                request_tx.send(Request { sequence, command }).unwrap();
                let response = reply_rx.recv().unwrap();
                if sequence == 2 {
                    assert!(matches!(response, Err(message) if message.contains("activeRun")));
                } else {
                    assert_eq!(response.unwrap().waiting, Some(sequence == 1));
                }
            }
            request_tx
                .send(Request {
                    sequence: 4,
                    command: Command::Shutdown,
                })
                .unwrap();
            let _ = reply_rx.recv().unwrap();
            drop(request_tx);
            let report = handle.join().unwrap();
            assert!(report.result.is_ok());
            assert_eq!(report.cleanup.stopped_runs, 1);
            assert_eq!(report.cleanup.disposed_runs, 1);
            assert_eq!(report.cleanup.stopped_marker_at_cleanup, Some(true));
        });
    }

    #[test]
    fn shutdown_drains_an_active_wait_and_runs_on_stop_once() {
        let (package, registry) = host_registry().unwrap();
        thread::scope(|scope| {
            let (request_tx, request_rx) = sync_channel(1);
            let (reply_tx, reply_rx) = sync_channel(1);
            let handle = scope.spawn(move || worker(request_rx, reply_tx, registry, package));
            let started = send(&request_tx, &reply_rx, 1, Command::StartCurrent).unwrap();
            assert_eq!(started.waiting, Some(true));
            send(&request_tx, &reply_rx, 2, Command::Shutdown).unwrap();
            drop(request_tx);
            drop(reply_rx);
            let report = handle.join().unwrap();
            assert!(report.result.is_ok());
            assert_eq!(report.cleanup.stopped_marker_at_cleanup, Some(true));
            assert_eq!(report.cleanup.pending_after_cleanup, Some(0));
            assert_eq!(report.cleanup.stopped_runs, 1);
            assert_eq!(report.cleanup.disposed_runs, 1);
            assert_eq!(report.cleanup.failure, None);
        });
    }

    #[test]
    fn history_retains_only_the_configured_recent_edits() {
        let (package, registry) = host_registry().unwrap();
        let mut state = setup(package, registry).unwrap();
        for value in 1..=MAX_HISTORY + 1 {
            set_b(&mut state, value as u64).unwrap();
        }
        for _ in 0..MAX_HISTORY {
            history(&mut state, true).unwrap();
        }
        assert!(history(&mut state, true).is_err());
        assert!(state.analysis.is_current(&state.document, &state.registry));
        let plan = compile(&state).unwrap();
        let mut run = Run::start(
            plan,
            &state.document,
            &state.registry,
            99,
            Default::default(),
        )
        .unwrap();
        pump_to_idle(&mut run).unwrap();
        // The discarded oldest inverse leaves the first edit (b=1) applied.
        assert_eq!(read_total(&run), Some(2));
        run.request_stop(StopMode::Immediate).unwrap();
        assert_eq!(run.state(), orange_runtime::RunState::Stopped);
        run.dispose().unwrap();
    }

    #[test]
    fn host_unwind_ends_the_session_and_retires_the_active_wait() {
        let (package, registry) = host_registry().unwrap();
        thread::scope(|scope| {
            let (request_tx, request_rx) = sync_channel(1);
            let (reply_tx, reply_rx) = sync_channel(1);
            let handle = scope.spawn(move || worker(request_rx, reply_tx, registry, package));
            assert_eq!(
                send(&request_tx, &reply_rx, 1, Command::StartCurrent)
                    .unwrap()
                    .waiting,
                Some(true)
            );
            request_tx
                .send(Request {
                    sequence: 2,
                    command: Command::PanicExecutionForTest,
                })
                .unwrap();
            assert!(reply_rx.recv().is_err());
            drop(request_tx);
            let report = handle.join().unwrap();
            assert!(report.result.unwrap_err().contains("session ended"));
            assert_eq!(report.cleanup.stopped_runs, 1);
            assert_eq!(report.cleanup.disposed_runs, 1);
            assert_eq!(report.cleanup.pending_after_cleanup, Some(0));
            assert_eq!(report.cleanup.failure, None);
        });
    }

    #[test]
    fn fresh_execution_uses_the_current_graph_without_a_scripted_result_requirement() {
        let (package, registry) = host_registry().unwrap();
        let mut state = setup(package, registry).unwrap();
        let (reply, _) = execute(&mut state, 1, Command::StartFresh).unwrap();
        assert_eq!(reply.active_run_id, None);
        assert_eq!(reply.terminal_total, Some(3));
        assert_eq!(reply.terminal_stopped, Some(true));
        assert_eq!(state.cleanup.disposed_runs, 1);
    }
}
