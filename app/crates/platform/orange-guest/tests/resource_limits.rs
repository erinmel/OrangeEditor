//! Hostile and exact-bound resource admission through the public guest APIs.
use orange_guest::{
    Grants, GuestEngine, GuestError, GuestInstance, GuestLimits, GuestPlugin, GuestTrap,
};
use orange_model::types::TypeRegistryBuilder;

const MEMORIES: &str = r#"(component
  (core module $m
    (memory $a 1 2) (memory $b 1)
    (func (export "a") (param i32) (result i32) local.get 0 memory.grow $a)
    (func (export "b") (param i32) (result i32) local.get 0 memory.grow $b))
  (core instance $i (instantiate $m))
  (func (export "a") (param "pages" u32) (result s32) (canon lift (core func $i "a")))
  (func (export "b") (param "pages" u32) (result s32) (canon lift (core func $i "b"))))"#;

const TABLES: &str = r#"(component
  (core module $m
    (table $a 4 6 funcref) (table $b 4 funcref)
    (func (export "a") (param i32) (result i32) ref.null func local.get 0 table.grow $a)
    (func (export "b") (param i32) (result i32) ref.null func local.get 0 table.grow $b))
  (core instance $i (instantiate $m))
  (func (export "a") (param "elements" u32) (result s32) (canon lift (core func $i "a")))
  (func (export "b") (param "elements" u32) (result s32) (canon lift (core func $i "b"))))"#;

#[test]
fn memory_and_table_totals_cover_initial_allocations_and_growth() {
    let engine = GuestEngine::new(GuestLimits {
        memory_bytes: 65_536,
        ..Default::default()
    })
    .unwrap();
    assert!(GuestInstance::new(&engine.compile(MEMORIES.as_bytes()).unwrap()).is_err());
    let engine = GuestEngine::new(GuestLimits {
        memory_bytes: 3 * 65_536,
        table_elements: 10,
        ..Default::default()
    })
    .unwrap();
    let mut memory = GuestInstance::new(&engine.compile(MEMORIES.as_bytes()).unwrap()).unwrap();
    assert_eq!(memory.call::<(u32,), (i32,)>("a", (1,)), Ok((1,)));
    assert_eq!(memory.call::<(u32,), (i32,)>("b", (1,)), Ok((-1,)));
    let mut tables = GuestInstance::new(&engine.compile(TABLES.as_bytes()).unwrap()).unwrap();
    assert_eq!(tables.call::<(u32,), (i32,)>("a", (2,)), Ok((4,)));
    assert_eq!(tables.call::<(u32,), (i32,)>("b", (1,)), Ok((-1,)));
    let engine = GuestEngine::new(GuestLimits {
        table_elements: 7,
        ..Default::default()
    })
    .unwrap();
    assert!(GuestInstance::new(&engine.compile(TABLES.as_bytes()).unwrap()).is_err());
}

#[test]
fn declared_maximum_refusals_leave_capacity_for_another_memory_or_table() {
    let engine = GuestEngine::new(GuestLimits {
        memory_bytes: 4 * 65_536,
        table_elements: 12,
        ..Default::default()
    })
    .unwrap();
    let mut memory = GuestInstance::new(&engine.compile(MEMORIES.as_bytes()).unwrap()).unwrap();
    assert_eq!(memory.call::<(u32,), (i32,)>("a", (2,)), Ok((-1,)));
    assert_eq!(memory.call::<(u32,), (i32,)>("b", (2,)), Ok((1,)));
    let mut tables = GuestInstance::new(&engine.compile(TABLES.as_bytes()).unwrap()).unwrap();
    assert_eq!(tables.call::<(u32,), (i32,)>("a", (3,)), Ok((-1,)));
    assert_eq!(tables.call::<(u32,), (i32,)>("b", (4,)), Ok((4,)));
}

#[test]
fn finite_memory_and_table_counts_are_enforced() {
    let engine = GuestEngine::new(GuestLimits {
        memories: 1,
        tables: 1,
        ..Default::default()
    })
    .unwrap();
    assert!(GuestInstance::new(&engine.compile(MEMORIES.as_bytes()).unwrap()).is_err());
    assert!(GuestInstance::new(&engine.compile(TABLES.as_bytes()).unwrap()).is_err());
    let huge = TABLES.replace("4 6 funcref", "100000 funcref");
    let engine = GuestEngine::new(GuestLimits::default()).unwrap();
    assert!(GuestInstance::new(&engine.compile(huge.as_bytes()).unwrap()).is_err());
    // The selected no-threads Wasmtime profile refuses shared memory, whose growth
    // does not invoke ResourceLimiter. It must not bypass aggregate admission.
    let shared =
        "(component (core module $m (memory 1 2 shared)) (core instance (instantiate $m)))";
    assert_eq!(
        engine.compile(shared.as_bytes()).unwrap_err(),
        GuestError::Compile
    );
}

/// Aliasing descriptors amplify64KiB guest memory into2MiB of host strings.
const AMPLIFY: &str = r#"(component
  (core module $m
    (memory (export "mem") 1)
    (func (export "realloc") (param i32 i32 i32 i32) (result i32) i32.const 32768)
    (func $values (export "values") (result i32) (local $i i32)
      (loop $l
        local.get $i i32.const 8 i32.mul i32.const 4096 i32.store
        local.get $i i32.const 8 i32.mul i32.const 4 i32.add i32.const 4096 i32.store
        local.get $i i32.const 1 i32.add local.tee $i i32.const 512 i32.lt_u br_if $l)
      i32.const 8192 i32.const 0 i32.store
      i32.const 8196 i32.const 512 i32.store
      i32.const 8192)
    (func (export "evaluate") (param i32 i32 i32 i32) (result i32) (local $i i32)
      ;; 512 twelve-byte values at 16384: case 0 (literal), then the shared string.
      (loop $l
        local.get $i i32.const 12 i32.mul i32.const 16388 i32.add i32.const 4096 i32.store
        local.get $i i32.const 12 i32.mul i32.const 16392 i32.add i32.const 4096 i32.store
        local.get $i i32.const 1 i32.add local.tee $i i32.const 512 i32.lt_u br_if $l)
      i32.const 8200 i32.const 0 i32.store
      i32.const 8204 i32.const 16384 i32.store
      i32.const 8208 i32.const 512 i32.store
      i32.const 8200))
  (core instance $i (instantiate $m))
  (type $strings (list string))
  (type $value (variant (case "literal" string) (case "bytes" (list u8))
    (case "i32-list" (list s32)) (case "i64-list" (list s64))
    (case "u32-list" (list u32)) (case "u64-list" (list u64))
    (case "f32-list" (list f32)) (case "f64-list" (list f64))))
  (type $values (list $value))
  (type $error (record (field "code" string) (field "message-key" string)))
  (type $out (result $values (error $error)))
  (func (export "values") (result $strings)
    (canon lift (core func $i "values") (memory (core memory $i "mem"))))
  (func $evaluate (param "definition" string) (param "inputs" $values) (result $out)
    (canon lift (core func $i "evaluate") (memory (core memory $i "mem")) (realloc (core func $i "realloc"))))
  (instance $nodes (export "value" (type $value)) (export "node-error" (type $error))
    (export "evaluate" (func $evaluate)))
  (export "orange:expansion/nodes@1.0.0" (instance $nodes))
  (instance $effects (export "value" (type $value)) (export "node-error" (type $error))
    (export "execute" (func $evaluate)))
  (export "orange:expansion/effects@1.0.0" (instance $effects)))"#;

#[test]
fn canonical_transfer_budget_refuses_aliased_results_before_typed_publication() {
    let engine = GuestEngine::new(GuestLimits {
        memory_bytes: 65_536,
        ..Default::default()
    })
    .unwrap();
    let component = engine.compile(AMPLIFY.as_bytes()).unwrap();
    let mut raw = GuestInstance::new(&component).unwrap();
    assert_eq!(
        raw.call::<(), (Vec<String>,)>("values", ()).unwrap_err(),
        GuestError::Trap(GuestTrap::Other)
    );
    assert!(raw.is_poisoned());
    let mut plugin = GuestPlugin::new(&component, Grants::default()).unwrap();
    let types = TypeRegistryBuilder::core().build().unwrap();
    assert_eq!(
        plugin.evaluate(&types, "x", &[], &[]),
        Err(GuestError::Trap(GuestTrap::Other))
    );
    assert!(plugin.is_poisoned());
    // Raising just the transfer budget admits the same small guest. Each call
    // gets a fresh transfer budget, independent of computation fuel and memory.
    let engine = GuestEngine::new(GuestLimits {
        memory_bytes: 65_536,
        transfer_bytes: 3 * 1024 * 1024,
        ..Default::default()
    })
    .unwrap();
    let component = engine.compile(AMPLIFY.as_bytes()).unwrap();
    let mut raw = GuestInstance::new(&component).unwrap();
    for _ in 0..2 {
        let (values,) = raw.call::<(), (Vec<String>,)>("values", ()).unwrap();
        assert_eq!(
            values.iter().map(String::len).sum::<usize>(),
            2 * 1024 * 1024
        );
        assert!(raw.remaining_fuel() > 9_000_000);
    }
    let mut plugin = GuestPlugin::new(&component, Grants::default()).unwrap();
    assert_eq!(
        plugin.evaluate(&types, "x", &[], &[]),
        Err(GuestError::OutputContract)
    );
    assert!(!plugin.is_poisoned());
}

#[test]
fn imported_log_strings_obey_transfer_admission_before_the_retained_log_limit() {
    let text = r#"(component
      (import "orange:expansion/log@1.0.0" (instance $log
        (export "write" (func (param "message" string)))))
      (alias export $log "write" (func $write))
      (core module $memory
        (memory (export "mem") 1)
        (func (export "realloc") (param i32 i32 i32 i32) (result i32) i32.const 32768))
      (core instance $mem (instantiate $memory))
      (core func $lower (canon lower (func $write) (memory (core memory $mem "mem"))))
      (core module $m
        (import "host" "write" (func $write (param i32 i32)))
        (func (export "evaluate") (param i32 i32 i32 i32) (result i32)
          local.get 0 local.get 1 call $write i32.const 0))
      (core instance $host (export "write" (func $lower)))
      (core instance $i (instantiate $m (with "host" (instance $host))))
      (type $value (variant (case "literal" string) (case "bytes" (list u8))
        (case "i32-list" (list s32)) (case "i64-list" (list s64))
        (case "u32-list" (list u32)) (case "u64-list" (list u64))
        (case "f32-list" (list f32)) (case "f64-list" (list f64))))
      (type $values (list $value))
      (type $error (record (field "code" string) (field "message-key" string)))
      (type $out (result $values (error $error)))
      (func $evaluate (param "definition" string) (param "inputs" $values) (result $out)
        (canon lift (core func $i "evaluate") (memory (core memory $mem "mem")) (realloc (core func $mem "realloc"))))
      (instance $nodes (export "value" (type $value)) (export "node-error" (type $error))
        (export "evaluate" (func $evaluate)))
      (export "orange:expansion/nodes@1.0.0" (instance $nodes))
      (instance $effects (export "value" (type $value)) (export "node-error" (type $error))
        (export "execute" (func $evaluate)))
      (export "orange:expansion/effects@1.0.0" (instance $effects)))"#;
    let types = TypeRegistryBuilder::core().build().unwrap();
    let message = "x".repeat(4096);
    for (transfer_bytes, allowed) in [(1024, false), (8192, true)] {
        let engine = GuestEngine::new(GuestLimits {
            transfer_bytes,
            ..Default::default()
        })
        .unwrap();
        let component = engine.compile(text.as_bytes()).unwrap();
        let mut plugin = GuestPlugin::new(
            &component,
            Grants {
                log: true,
                files: false,
                http: false,
            },
        )
        .unwrap();
        let result = plugin.evaluate(&types, &message, &[], &[]);
        if allowed {
            assert_eq!(result, Ok(Ok(vec![])));
            assert_eq!(plugin.take_log(), ["x".repeat(orange_guest::MAX_LOG_BYTES)]);
        } else {
            assert_eq!(result, Err(GuestError::Trap(GuestTrap::Other)));
            assert!(plugin.take_log().is_empty());
        }
    }
}
