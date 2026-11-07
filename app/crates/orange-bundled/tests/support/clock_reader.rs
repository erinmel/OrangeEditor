//! `core/now` clock readers (C04b3): a volatile host input read once per epoch from the
//! run's virtual clock, never folded as a constant.
use super::*;

const FIRST: u32 = 100;
const SECOND: u32 = 101;

fn duration_variable(n: u32) -> Json {
    json!({"id": id(n), "name": format!("v{n}"), "type": "core/duration@1", "scope": "run"})
}

fn read(run: &Run, n: u32) -> Option<i64> {
    match run.variable(id(n).parse().unwrap()) {
        Some(Value::Duration(at)) => Some(*at),
        None => None,
        other => panic!("unexpected clock value {other:?}"),
    }
}

#[test]
fn now_reads_virtual_time_at_each_consumption() {
    // Start → set FIRST ← now → wait 7 → set SECOND ← now: one Now node, two epochs.
    let (doc, plan) = compile(
        vec![
            core(1, "on-start"),
            variable_node(2, "set-variable", FIRST),
            wait_node(3, 7),
            variable_node(4, "set-variable", SECOND),
            core(5, "now"),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 3, "in"),
            edge(3, 3, "then", 4, "in"),
            edge(4, 5, "elapsed", 2, "value"),
            edge(5, 5, "elapsed", 4, "value"),
        ],
        vec![duration_variable(FIRST), duration_variable(SECOND)],
    );
    let mut run = run(&doc, plan);
    pump_until_idle(&mut run);
    assert_eq!((read(&run, FIRST), read(&run, SECOND)), (Some(0), None));
    run.advance_to(12).unwrap();
    pump_until_idle(&mut run);
    // The wait resumes at 12, not at its 7 ns deadline: Now reads the actor's clock.
    assert_eq!((read(&run, FIRST), read(&run, SECOND)), (Some(0), Some(12)));
}

#[test]
fn now_differs_from_a_late_timer_tick_scheduled_time() {
    let mut on_timer = core(1, "on-timer");
    on_timer["config"] = json!({"interval": "10"});
    on_timer["activation"] = activation(0, 4, "fault");
    let (doc, plan) = compile(
        vec![
            on_timer,
            variable_node(2, "set-variable", FIRST),
            variable_node(3, "set-variable", SECOND),
            core(4, "now"),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 1, "scheduled_at", 2, "value"),
            edge(3, 2, "then", 3, "in"),
            edge(4, 4, "elapsed", 3, "value"),
        ],
        vec![duration_variable(FIRST), duration_variable(SECOND)],
    );
    let mut run = run(&doc, plan);
    for (now, expected) in [(25, (20, 25)), (30, (30, 30))] {
        run.advance_to(now).unwrap();
        pump_until_idle(&mut run);
        assert_eq!(
            (read(&run, FIRST), read(&run, SECOND)),
            (Some(expected.0), Some(expected.1))
        );
    }
}

#[test]
fn now_is_a_registered_input_free_clock_reader() {
    let (_, registry) = catalog();
    let definition = registry
        .definition(&orange_model::document::DefinitionRef {
            id: "core/now".parse().unwrap(),
            version: 1,
        })
        .unwrap();
    assert_eq!(
        definition.volatility,
        Some(orange_model::catalog::Volatility::Clock)
    );
    let orange_model::catalog::Ports::Declared(ports) = &definition.ports else {
        panic!("core/now has fixed ports");
    };
    assert_eq!(ports.len(), 1);
    assert_eq!(ports[0].port_id.as_str(), "elapsed");
}
