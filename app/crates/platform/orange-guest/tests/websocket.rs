//! The 1.1 `settings` and `websocket` imports: settings always linked, messages only
//! with the grant, only from effects, and only through the session the host hands to
//! the call. The package never sees the connection's address or credentials.
use std::sync::{Arc, Mutex};

use orange_guest::{Grants, GuestAccess, GuestEngine, GuestError, GuestLimits, GuestPlugin};
use orange_model::types::TypeRegistryBuilder;
use orange_sdk::{Message, MessageError, Session};

/// The core `evaluate` writes the node's settings to `copy`; a variant lifts it as the
/// effect export, because only effects may write files. `execute(definition, _)` sends
/// `definition` as a text message and writes `sent`, or the error word, to `copy`.
const GUEST: &str = r#"(component
  (import "orange:expansion/files@1.1.0" (instance $files
    (type $bytes (list u8))
    (type $write-result (result (error string)))
    (export "write" (func (param "path" string) (param "data" $bytes) (result $write-result)))))
  (import "orange:expansion/settings@1.1.0" (instance $settings
    (export "get" (func (result string)))))
  (import "orange:expansion/websocket@1.1.0" (instance $ws
    (type $sent (result (error string)))
    (export "send-text" (func (param "text" string) (result $sent)))))
  (alias export $files "write" (func $write))
  (alias export $settings "get" (func $get))
  (alias export $ws "send-text" (func $send))
  (core module $memory
    (memory (export "mem") 1)
    (data (i32.const 1024) "copy")
    (data (i32.const 1040) "sent")
    (global $next (mut i32) (i32.const 32768))
    (func (export "realloc") (param i32 i32 i32 i32) (result i32) (local $at i32)
      global.get $next local.set $at
      global.get $next local.get 3 i32.add i32.const 8 i32.add i32.const -8 i32.and
      global.set $next
      local.get $at))
  (core instance $mem (instantiate $memory))
  (core func $lower-write (canon lower (func $write)
    (memory (core memory $mem "mem")) (realloc (core func $mem "realloc"))))
  (core func $lower-get (canon lower (func $get)
    (memory (core memory $mem "mem")) (realloc (core func $mem "realloc"))))
  (core func $lower-send (canon lower (func $send)
    (memory (core memory $mem "mem")) (realloc (core func $mem "realloc"))))
  (core module $m
    (import "host" "mem" (memory 1))
    (import "host" "write" (func $write (param i32 i32 i32 i32 i32)))
    (import "host" "get" (func $get (param i32)))
    (import "host" "send" (func $send (param i32 i32 i32)))
    (func (export "evaluate") (param i32 i32 i32 i32) (result i32)
      i32.const 200 call $get
      i32.const 1024 i32.const 4
      i32.const 200 i32.load i32.const 204 i32.load
      i32.const 300 call $write
      i32.const 0)
    (func (export "execute") (param i32 i32 i32 i32) (result i32)
      local.get 0 local.get 1 i32.const 200 call $send
      i32.const 1024 i32.const 4
      (if (result i32 i32) (i32.load8_u (i32.const 200))
        (then i32.const 204 i32.load i32.const 208 i32.load)
        (else i32.const 1040 i32.const 4))
      i32.const 300 call $write
      i32.const 0))
  (core instance $host
    (export "mem" (memory $mem "mem"))
    (export "write" (func $lower-write))
    (export "get" (func $lower-get))
    (export "send" (func $lower-send)))
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
  (func $execute (param "definition" string) (param "inputs" $values) (result $out)
    (canon lift (core func $i "execute") (memory (core memory $mem "mem")) (realloc (core func $mem "realloc"))))
  (instance $nodes (export "value" (type $value)) (export "node-error" (type $error))
    (export "evaluate" (func $evaluate)))
  (export "orange:expansion/nodes@1.1.0" (instance $nodes))
  (instance $effects (export "value" (type $value)) (export "node-error" (type $error))
    (export "execute" (func $execute)))
  (export "orange:expansion/effects@1.1.0" (instance $effects)))"#;

/// A host session that records what it is asked to send, or refuses with `refuse`.
#[derive(Debug, Default)]
struct Recorded {
    sent: Mutex<Vec<Message>>,
    refuse: Option<MessageError>,
}
impl Session for Recorded {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn send_message(&self, message: Message) -> Result<(), MessageError> {
        if let Some(error) = self.refuse {
            return Err(error);
        }
        self.sent.lock().unwrap().push(message);
        Ok(())
    }
}

#[test]
fn a_package_sends_through_the_hosts_session_only_with_the_grant() {
    let outside =
        std::env::temp_dir().join(format!("orange-guest-websocket-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&outside);
    let data = outside.join("data");
    let access = GuestAccess {
        data_dir: Some(data.clone()),
        ..GuestAccess::default()
    };
    let engine = GuestEngine::new(GuestLimits::default()).unwrap();
    let component = engine.compile(GUEST.as_bytes()).unwrap();
    let types = TypeRegistryBuilder::core().build().unwrap();
    let copy = || std::fs::read_to_string(data.join("copy")).unwrap();

    // Without the grant the component is not instantiated.
    let files_only = Grants {
        files: true,
        ..Grants::default()
    };
    assert_eq!(
        GuestPlugin::with_access(&component, files_only, access.clone()).unwrap_err(),
        GuestError::Link
    );
    let granted = Grants {
        files: true,
        websocket: true,
        ..Grants::default()
    };
    let mut plugin = GuestPlugin::with_access(&component, granted, access.clone()).unwrap();

    // The node's settings name its connection; nothing secret is in them.
    let settings = r#"{"connection":"40000000-0000-4000-8000-000000000001"}"#;
    let reader = GUEST.replace(
        "(canon lift (core func $i \"execute\")",
        "(canon lift (core func $i \"evaluate\")",
    );
    let reader = engine.compile(reader.as_bytes()).unwrap();
    let mut reading = GuestPlugin::with_access(&reader, granted, access.clone()).unwrap();
    reading.set_settings(settings.to_owned());
    assert_eq!(reading.execute(&types, "x", &[], &[]), Ok(Ok(vec![])));
    assert_eq!(copy(), settings);

    // No session for the call: nothing to send on.
    assert_eq!(plugin.execute(&types, "hello", &[], &[]), Ok(Ok(vec![])));
    assert_eq!(copy(), "unavailable");

    let session = Arc::new(Recorded::default());
    plugin.set_session(Some(Arc::clone(&session) as Arc<dyn Session>));
    assert_eq!(plugin.execute(&types, "hello", &[], &[]), Ok(Ok(vec![])));
    assert_eq!(copy(), "sent");
    assert_eq!(
        *session.sent.lock().unwrap(),
        [Message::Text("hello".into())]
    );

    // A full queue on the host reports `quota-exceeded`, not the host's text.
    let full = Arc::new(Recorded {
        refuse: Some(MessageError::Full),
        ..Recorded::default()
    });
    plugin.set_session(Some(full as Arc<dyn Session>));
    assert_eq!(plugin.execute(&types, "more", &[], &[]), Ok(Ok(vec![])));
    assert_eq!(copy(), "quota-exceeded");
    let _ = std::fs::remove_dir_all(&outside);
}
