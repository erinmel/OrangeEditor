//! K02a probe: one engine, isolated instances, deterministic fuel, host interruption,
//! memory limits, stack overflow and refused ungranted imports (ADR-03 falsification).
mod archive_nodes;
mod plugin;
mod resource_limits;

use orange_guest::{GuestEngine, GuestError, GuestInstance, GuestLimits, GuestTrap};

/// Core module behind every probe component, lifted to typed component exports.
const PROBE: &str = r#"
(component
  (core module $m
    (memory (export "mem") 1)
    (func $add (export "add") (param i64 i64) (result i64)
      local.get 0 local.get 1 i64.add)
    (func (export "fail") (result i64) unreachable)
    (func (export "spin") (result i64) (loop $l br $l) i64.const 0)
    (func $deep (export "deep") (param i64) (result i64)
      local.get 0 i64.const 1 i64.add call $deep)
    (func (export "grow") (param i32) (result i32) local.get 0 memory.grow))
  (core instance $i (instantiate $m))
  (func (export "add") (param "a" s64) (param "b" s64) (result s64)
    (canon lift (core func $i "add")))
  (func (export "fail") (result s64) (canon lift (core func $i "fail")))
  (func (export "spin") (result s64) (canon lift (core func $i "spin")))
  (func (export "deep") (param "n" s64) (result s64) (canon lift (core func $i "deep")))
  (func (export "grow") (param "pages" u32) (result s32) (canon lift (core func $i "grow"))))
"#;

fn engine(limits: GuestLimits) -> GuestEngine {
    GuestEngine::new(limits).unwrap()
}

#[test]
fn a_component_export_runs_and_reports_typed_results() {
    let engine = engine(GuestLimits::default());
    let component = engine.compile(PROBE.as_bytes()).unwrap();
    let mut instance = GuestInstance::new(&component).unwrap();
    assert_eq!(
        instance.call::<(i64, i64), (i64,)>("add", (40, 2)),
        Ok((42,))
    );
    assert_eq!(
        instance.call::<(i64, i64), (i64,)>("add", (i64::MAX, 1)),
        Ok((i64::MIN,))
    );
    // A missing export or a mismatched signature is refused without poisoning.
    assert_eq!(
        instance.call::<(), (i64,)>("missing", ()),
        Err(GuestError::Export)
    );
    assert_eq!(
        instance.call::<(i32,), (i64,)>("add", (1,)),
        Err(GuestError::Export)
    );
    assert!(!instance.is_poisoned());
    assert_eq!(
        engine.compile(b"not a component").unwrap_err(),
        GuestError::Compile
    );
}

#[test]
fn a_trap_poisons_only_its_instance() {
    let engine = engine(GuestLimits::default());
    let component = engine.compile(PROBE.as_bytes()).unwrap();
    let mut broken = GuestInstance::new(&component).unwrap();
    let mut healthy = GuestInstance::new(&component).unwrap();
    assert_eq!(
        broken.call::<(), (i64,)>("fail", ()),
        Err(GuestError::Trap(GuestTrap::Unreachable))
    );
    assert!(broken.is_poisoned());
    assert_eq!(
        broken.call::<(i64, i64), (i64,)>("add", (1, 1)),
        Err(GuestError::Poisoned)
    );
    assert_eq!(healthy.call::<(i64, i64), (i64,)>("add", (1, 1)), Ok((2,)));
    let mut fresh = GuestInstance::new(&component).unwrap();
    assert_eq!(fresh.call::<(i64, i64), (i64,)>("add", (2, 2)), Ok((4,)));
}

#[test]
fn fuel_bounds_each_call_deterministically() {
    let limits = GuestLimits {
        fuel_per_call: 100_000,
        ..GuestLimits::default()
    };
    let engine = engine(limits);
    let component = engine.compile(PROBE.as_bytes()).unwrap();
    let mut instance = GuestInstance::new(&component).unwrap();
    // Each call starts from the same budget; cheap calls leave the same remainder.
    instance.call::<(i64, i64), (i64,)>("add", (1, 2)).unwrap();
    let after_first = instance.remaining_fuel();
    instance.call::<(i64, i64), (i64,)>("add", (3, 4)).unwrap();
    assert_eq!(instance.remaining_fuel(), after_first);
    assert!(after_first < limits.fuel_per_call);
    assert_eq!(
        instance.call::<(), (i64,)>("spin", ()),
        Err(GuestError::Trap(GuestTrap::OutOfFuel))
    );
    assert_eq!(instance.remaining_fuel(), 0);
}

#[test]
fn memory_growth_stops_at_the_instance_limit() {
    let engine = engine(GuestLimits {
        memory_bytes: 4 * 65_536,
        ..GuestLimits::default()
    });
    let component = engine.compile(PROBE.as_bytes()).unwrap();
    let mut instance = GuestInstance::new(&component).unwrap();
    // memory.grow returns the old page count, or -1 when the host refuses.
    assert_eq!(instance.call::<(u32,), (i32,)>("grow", (3,)), Ok((1,)));
    assert_eq!(instance.call::<(u32,), (i32,)>("grow", (1,)), Ok((-1,)));
    assert!(!instance.is_poisoned());
    // A memory larger than the limit cannot be instantiated at all.
    let tiny = GuestEngine::new(GuestLimits {
        memory_bytes: 1024,
        ..GuestLimits::default()
    })
    .unwrap();
    let component = tiny.compile(PROBE.as_bytes()).unwrap();
    assert!(GuestInstance::new(&component).is_err());
}

#[test]
fn unbounded_recursion_traps_with_stack_overflow() {
    let engine = engine(GuestLimits {
        fuel_per_call: u64::MAX,
        ..GuestLimits::default()
    });
    let component = engine.compile(PROBE.as_bytes()).unwrap();
    let mut instance = GuestInstance::new(&component).unwrap();
    assert_eq!(
        instance.call::<(i64,), (i64,)>("deep", (0,)),
        Err(GuestError::Trap(GuestTrap::StackOverflow))
    );
}

#[test]
fn ungranted_imports_are_refused_at_instantiation() {
    let engine = engine(GuestLimits::default());
    let wants_host = r#"
(component
  (import "host-secret" (func $secret (result s64)))
  (core func $lowered (canon lower (func $secret)))
  (core module $m
    (import "host" "secret" (func (result i64)))
    (func (export "leak") (result i64) call 0))
  (core instance $host (export "secret" (func $lowered)))
  (core instance $i (instantiate $m (with "host" (instance $host))))
  (func (export "leak") (result s64) (canon lift (core func $i "leak"))))
"#;
    let component = engine.compile(wants_host.as_bytes()).unwrap();
    assert_eq!(
        GuestInstance::new(&component).unwrap_err(),
        GuestError::Link
    );
    for error in [
        GuestError::Link,
        GuestError::Trap(GuestTrap::OutOfFuel),
        GuestError::Poisoned,
    ] {
        assert!(error.message_key().starts_with("core/guest-"));
    }
}
