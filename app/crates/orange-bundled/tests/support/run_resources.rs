//! Run-owned resources (C08a): a generation-checked table of host objects that each
//! run closes exactly once, on host close, at `Stopped` or when the run is dropped.
use super::*;
use orange_bundled::MathPackage;
use orange_model::catalog::RegistryBuilder;
use orange_model::id::TypeExpr;
use orange_model::types::TypeRegistryBuilder;
use orange_model::validation::HostOrigin;
use orange_runtime::{CloseReason, ResourceError, ResourceHandle, RunResource};
use std::sync::Mutex;

const SESSION: &str = "acme/session@1";
const FILE: &str = "acme/file@1";

type Log = Arc<Mutex<Vec<(u8, CloseReason)>>>;

struct Probe {
    log: Log,
    tag: u8,
    uses: u32,
}
impl RunResource for Probe {
    fn close(self: Box<Self>, reason: CloseReason) {
        self.log.lock().unwrap().push((self.tag, reason));
    }
}
struct Other;
impl RunResource for Other {
    fn close(self: Box<Self>, _: CloseReason) {}
}

fn probe(log: &Log, tag: u8) -> Box<Probe> {
    Box::new(Probe {
        log: log.clone(),
        tag,
        uses: 0,
    })
}
fn kind(ty: &str) -> TypeExpr {
    ty.parse().unwrap()
}

/// On Start waits, so the run stays `Running` until the host advances the clock. With
/// `fail`, it then reads an uninitialized variable under the stop-run fault policy.
fn waiting_run(options: RunOptions, fail: bool) -> Run {
    let mut types = TypeRegistryBuilder::core();
    for ty in [SESSION, FILE] {
        types.register_handle_kind("acme", &kind(ty)).unwrap();
    }
    let digest = format!("sha256:{}", "a".repeat(64));
    let package = MathPackage::new(digest.clone()).unwrap();
    let registry = package
        .register(
            RegistryBuilder::core(HostOrigin::new("0.0.0", digest).unwrap())
                .types(Arc::new(types.build().unwrap())),
        )
        .unwrap()
        .build()
        .unwrap();
    let mut start = core(1, "on-start");
    start["activation"]["on_fault"] = json!("stop-run");
    let mut nodes = vec![start, wait_node(2, 5)];
    let mut edges = vec![edge(1, 1, "then", 2, "in")];
    if fail {
        nodes.extend([
            variable_node(3, "get-variable", 100),
            variable_node(4, "set-variable", 101),
        ]);
        edges.extend([edge(2, 2, "then", 4, "in"), edge(3, 3, "value", 4, "value")]);
    }
    let mut saved: Json =
        serde_json::from_str(&to_json(&graph(&package, nodes, edges, vec![])).unwrap()).unwrap();
    saved["variables"] = json!([variable(100, "run", None), variable(101, "run", None)]);
    let document = from_json(&saved.to_string()).unwrap();
    let analysis = analyze(&document, &registry);
    let plan = Arc::new(
        compile_control(
            &document,
            registry,
            &analysis,
            &package,
            CompileOptions::default(),
        )
        .unwrap(),
    );
    let mut run = run_with_options(&document, plan, options);
    pump_until_idle(&mut run);
    assert_eq!(run.state(), RunState::Running);
    run
}

fn acquire(run: &mut Run, ty: &str, resource: Box<dyn RunResource>) -> ResourceHandle {
    run.acquire_resource(&kind(ty), resource).unwrap()
}

#[test]
fn handles_are_checked_by_run_generation_and_kind() {
    let log = Log::default();
    let mut run = waiting_run(RunOptions::default(), false);
    let session = acquire(&mut run, SESSION, probe(&log, 0));
    let file = acquire(&mut run, FILE, probe(&log, 1));
    assert_eq!(run.ownership().resources, 2);
    assert_eq!(run.resource_kind(file).unwrap(), &kind(FILE));

    run.resource_mut::<Probe>(session, &kind(SESSION))
        .unwrap()
        .uses += 1;
    assert_eq!(
        run.resource_mut::<Probe>(session, &kind(SESSION))
            .unwrap()
            .uses,
        1
    );
    assert_eq!(
        run.resource_mut::<Probe>(session, &kind(FILE)).err(),
        Some(ResourceError::WrongKind)
    );
    assert_eq!(
        run.resource_mut::<Other>(session, &kind(SESSION)).err(),
        Some(ResourceError::WrongKind)
    );

    run.close_resource(session).unwrap();
    assert_eq!(*log.lock().unwrap(), [(0, CloseReason::Closed)]);
    assert_eq!(run.close_resource(session), Err(ResourceError::StaleHandle));
    assert_eq!(
        run.resource_kind(session).err(),
        Some(ResourceError::StaleHandle)
    );
    // The freed slot is reused under a new generation; the old handle stays stale.
    let again = acquire(&mut run, SESSION, probe(&log, 2));
    assert_eq!(again.id().index, session.id().index);
    assert_ne!(again.id().generation, session.id().generation);
    assert_eq!(
        run.resource_mut::<Probe>(session, &kind(SESSION)).err(),
        Some(ResourceError::StaleHandle)
    );
    assert_eq!(
        run.resource_mut::<Probe>(again, &kind(SESSION))
            .unwrap()
            .uses,
        0
    );

    // Another run's table never answers for this run's handles, even at the same slot.
    let other_log = Log::default();
    let mut other = waiting_run(RunOptions::default(), false);
    let foreign = acquire(&mut other, SESSION, probe(&other_log, 9));
    assert_eq!(foreign.id().index, session.id().index);
    assert_eq!(run.close_resource(foreign), Err(ResourceError::WrongRun));
    assert_eq!(
        run.resource_kind(foreign).err(),
        Some(ResourceError::WrongRun)
    );
    assert_eq!(run.ownership().resources, 2);
    assert_eq!(other.ownership().resources, 1);
    assert!(other_log.lock().unwrap().is_empty());
}

#[test]
fn refused_acquisitions_hand_the_resource_back_unclosed() {
    let log = Log::default();
    let options = RunOptions {
        limits: orange_runtime::RunLimits {
            max_resources: 1,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut run = waiting_run(options, false);
    for ty in [
        "acme/unknown@1",
        "core/u64@1",
        "core/handle@1<acme/session@1>",
    ] {
        let refused = run.acquire_resource(&kind(ty), probe(&log, 0)).unwrap_err();
        assert_eq!(refused.error, ResourceError::UnknownKind, "{ty}");
    }
    let held = acquire(&mut run, SESSION, probe(&log, 1));
    let refused = run
        .acquire_resource(&kind(SESSION), probe(&log, 2))
        .unwrap_err();
    assert_eq!(refused.error, ResourceError::ResourceLimit);
    // The host still owns a refused resource and may close it itself.
    refused.resource.close(CloseReason::Closed);
    assert_eq!(*log.lock().unwrap(), [(2, CloseReason::Closed)]);

    run.close_resource(held).unwrap();
    let last = acquire(&mut run, SESSION, probe(&log, 3));
    run.request_stop(orange_runtime::StopMode::Graceful)
        .unwrap();
    assert_eq!(run.state(), RunState::Draining);
    let refused = run
        .acquire_resource(&kind(SESSION), probe(&log, 4))
        .unwrap_err();
    assert_eq!(refused.error, ResourceError::NotRunning);
    // Accepted work may still use what the run owns while it drains.
    assert!(run.resource_mut::<Probe>(last, &kind(SESSION)).is_ok());
    assert_eq!(log.lock().unwrap().len(), 2);
}

#[test]
fn stopping_closes_every_resource_once_newest_first() {
    let log = Log::default();
    let mut run = waiting_run(RunOptions::default(), false);
    let handles: Vec<_> = (0..3)
        .map(|tag| acquire(&mut run, SESSION, probe(&log, tag)))
        .collect();
    run.close_resource(handles[1]).unwrap();
    run.request_stop(orange_runtime::StopMode::Graceful)
        .unwrap();
    // Graceful drain keeps them for accepted work; the wait finishes at 5.
    assert_eq!(run.ownership().resources, 2);
    run.advance_to(5).unwrap();
    pump_until_idle(&mut run);
    assert_eq!(run.state(), RunState::Stopped);
    assert_eq!(run.ownership().resources, 0);
    assert_eq!(
        *log.lock().unwrap(),
        [
            (1, CloseReason::Closed),
            (2, CloseReason::RunStopped),
            (0, CloseReason::RunStopped)
        ]
    );
    assert_eq!(
        run.close_resource(handles[0]),
        Err(ResourceError::StaleHandle)
    );
    run.dispose().unwrap();
    assert_eq!(
        run.close_resource(handles[2]),
        Err(ResourceError::StaleHandle)
    );
    assert_eq!(log.lock().unwrap().len(), 3);
}

#[test]
fn immediate_stop_fault_and_drop_each_close_with_their_reason() {
    let log = Log::default();
    let mut stopped = waiting_run(RunOptions::default(), false);
    acquire(&mut stopped, SESSION, probe(&log, 0));
    stopped.stop().unwrap();
    pump_until_idle(&mut stopped);
    assert_eq!(stopped.state(), RunState::Stopped);

    let mut faulted = waiting_run(RunOptions::default(), true);
    acquire(&mut faulted, FILE, probe(&log, 1));
    faulted.advance_to(5).unwrap();
    pump_until_idle(&mut faulted);
    assert_eq!(faulted.state(), RunState::Stopped);

    let mut dropped = waiting_run(RunOptions::default(), false);
    acquire(&mut dropped, SESSION, probe(&log, 2));
    drop(dropped);

    assert_eq!(
        *log.lock().unwrap(),
        [
            (0, CloseReason::RunStopped),
            (1, CloseReason::RunFaulted),
            (2, CloseReason::RunDropped)
        ]
    );
}
