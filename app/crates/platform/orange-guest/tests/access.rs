//! The `files` and `http` imports: linked only when granted, confined to the package's
//! data directory and to the application's network client.
use std::sync::{Arc, Mutex};

use orange_guest::{
    Grants, GuestAccess, GuestEngine, GuestError, GuestLimits, GuestPlugin, HttpClient,
    HttpRequest, HttpResponse,
};
use orange_model::types::TypeRegistryBuilder;

/// `evaluate(definition, _)` writes a file named `definition` holding its own name,
/// reads it back and writes what it got (the content, or the error word) to `copy`.
/// `execute(definition, _)` sends `GET definition` and writes the response body (after
/// the status and headers in the response record), or the error word, to `copy`.
const GUEST: &str = r#"(component
  (import "orange:expansion/files@1.0.0" (instance $files
    (type $bytes (list u8))
    (type $read-result (result $bytes (error string)))
    (export "read" (func (param "path" string) (result $read-result)))
    (type $write-result (result (error string)))
    (export "write" (func (param "path" string) (param "data" $bytes) (result $write-result)))))
  (import "orange:expansion/http@1.0.0" (instance $http
    (type $bytes (list u8))
    (type $header (tuple string string))
    (type $headers (list $header))
    (type $request (record (field "method" string) (field "url" string)
      (field "headers" $headers) (field "body" $bytes)))
    (export "request" (type $request-x (eq $request)))
    (type $response (record (field "status" u16) (field "headers" $headers)
      (field "body" $bytes)))
    (export "response" (type $response-x (eq $response)))
    (type $send-result (result $response-x (error string)))
    (export "send" (func (param "request" $request-x) (result $send-result)))))
  (alias export $files "read" (func $read))
  (alias export $files "write" (func $write))
  (alias export $http "send" (func $send))
  (core module $memory
    (memory (export "mem") 1)
    (data (i32.const 1024) "copy")
    (data (i32.const 1040) "GET")
    (global $next (mut i32) (i32.const 32768))
    (func (export "realloc") (param i32 i32 i32 i32) (result i32) (local $at i32)
      global.get $next local.set $at
      global.get $next local.get 3 i32.add i32.const 8 i32.add i32.const -8 i32.and
      global.set $next
      local.get $at))
  (core instance $mem (instantiate $memory))
  (core func $lower-read (canon lower (func $read)
    (memory (core memory $mem "mem")) (realloc (core func $mem "realloc"))))
  (core func $lower-write (canon lower (func $write)
    (memory (core memory $mem "mem")) (realloc (core func $mem "realloc"))))
  (core func $lower-send (canon lower (func $send)
    (memory (core memory $mem "mem")) (realloc (core func $mem "realloc"))))
  (core module $m
    (import "host" "mem" (memory 1))
    (import "host" "read" (func $read (param i32 i32 i32)))
    (import "host" "write" (func $write (param i32 i32 i32 i32 i32)))
    (import "host" "send" (func $send (param i32 i32 i32 i32 i32 i32 i32 i32 i32)))
    (func (export "evaluate") (param i32 i32 i32 i32) (result i32)
      local.get 0 local.get 1 local.get 0 local.get 1 i32.const 100 call $write
      local.get 0 local.get 1 i32.const 200 call $read
      i32.const 1024 i32.const 4
      i32.const 204 i32.load i32.const 208 i32.load
      i32.const 300 call $write
      i32.const 0)
    (func (export "execute") (param i32 i32 i32 i32) (result i32)
      i32.const 1040 i32.const 3
      local.get 0 local.get 1
      i32.const 0 i32.const 0
      i32.const 0 i32.const 0
      i32.const 200 call $send
      i32.const 1024 i32.const 4
      (if (result i32 i32) (i32.load8_u (i32.const 200))
        (then i32.const 204 i32.load i32.const 208 i32.load)
        (else i32.const 216 i32.load i32.const 220 i32.load))
      i32.const 300 call $write
      i32.const 0))
  (core instance $host
    (export "mem" (memory $mem "mem"))
    (export "read" (func $lower-read))
    (export "write" (func $lower-write))
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
  (export "orange:expansion/nodes@1.0.0" (instance $nodes))
  (instance $effects (export "value" (type $value)) (export "node-error" (type $error))
    (export "execute" (func $execute)))
  (export "orange:expansion/effects@1.0.0" (instance $effects)))"#;

#[derive(Debug, Default)]
struct Recorded(Mutex<Vec<HttpRequest>>);
impl HttpClient for Recorded {
    fn send(&self, request: &HttpRequest) -> Result<HttpResponse, String> {
        self.0.lock().unwrap().push(request.clone());
        if request.url.ends_with("/down") {
            return Err("connection refused by 10.0.0.7".into());
        }
        Ok(HttpResponse {
            status: 200,
            headers: vec![("x-served-by".into(), "test".into())],
            body: b"hello".to_vec(),
        })
    }
}

const BOTH: Grants = Grants {
    log: false,
    files: true,
    http: true,
    http_local: false,
    compute: false,
};

#[test]
fn granted_imports_reach_only_the_data_directory_and_the_hosts_client() {
    let outside = std::env::temp_dir().join(format!("orange-guest-access-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&outside);
    let data = outside.join("data").join("org.example.files");
    let client = Arc::new(Recorded::default());
    let access = GuestAccess {
        data_dir: Some(data.clone()),
        http: Some(Arc::clone(&client) as Arc<dyn HttpClient>),
        ..GuestAccess::default()
    };
    let engine = GuestEngine::new(GuestLimits::default()).unwrap();
    let component = engine.compile(GUEST.as_bytes()).unwrap();
    let types = TypeRegistryBuilder::core().build().unwrap();

    // Without both grants the component is not instantiated at all.
    for grants in [
        Grants::default(),
        Grants {
            files: true,
            ..Grants::default()
        },
        Grants {
            http: true,
            ..Grants::default()
        },
    ] {
        assert_eq!(
            GuestPlugin::with_access(&component, grants, access.clone()).unwrap_err(),
            GuestError::Link
        );
    }
    let mut plugin = GuestPlugin::with_access(&component, BOTH, access.clone()).unwrap();
    let copy = || std::fs::read_to_string(data.join("copy")).unwrap();
    assert_eq!(
        plugin.evaluate(&types, "notes/a.txt", &[], &[]),
        Ok(Ok(vec![]))
    );
    assert!(!data.exists());
    assert!(client.0.lock().unwrap().is_empty());

    // The same guest file operation succeeds through an effect export.
    let file_guest = GUEST.replace("(core func $i \"execute\")", "(core func $i \"evaluate\")");
    let file_component = engine.compile(file_guest.as_bytes()).unwrap();
    let mut files = GuestPlugin::with_access(&file_component, BOTH, access.clone()).unwrap();

    // Files: written under the data directory, with directories made on the way.
    assert_eq!(
        files.execute(&types, "notes/a.txt", &[], &[]),
        Ok(Ok(vec![]))
    );
    assert_eq!(
        std::fs::read_to_string(data.join("notes").join("a.txt")).unwrap(),
        "notes/a.txt"
    );
    assert_eq!(copy(), "notes/a.txt");
    // A path that would leave the directory is refused for writing and for reading.
    for escape in ["../escape.txt", "/escape.txt", "notes/../../escape.txt"] {
        assert_eq!(files.execute(&types, escape, &[], &[]), Ok(Ok(vec![])));
        assert_eq!(copy(), "invalid-path", "{escape}");
    }
    assert!(!outside.join("escape.txt").exists());
    assert!(!outside.join("data").join("escape.txt").exists());

    // HTTP: the request goes to the application's client; `http` sends only HTTPS.
    assert_eq!(
        plugin.execute(&types, "https://example.org/status", &[], &[]),
        Ok(Ok(vec![]))
    );
    assert_eq!(copy(), "hello");
    {
        let sent = client.0.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].method, "GET");
        assert_eq!(sent[0].url, "https://example.org/status");
        assert_eq!(
            sent[0].max_response_bytes,
            GuestLimits::default().transfer_bytes
        );
    }
    assert_eq!(
        plugin.execute(&types, "http://example.org/status", &[], &[]),
        Ok(Ok(vec![]))
    );
    assert_eq!(copy(), "not-allowed");
    assert_eq!(
        plugin.execute(&types, "http://127.0.0.1:8880/status", &[], &[]),
        Ok(Ok(vec![]))
    );
    assert_eq!(copy(), "not-allowed");
    assert_eq!(
        plugin.execute(&types, "ftp://example.org/status", &[], &[]),
        Ok(Ok(vec![]))
    );
    assert_eq!(copy(), "invalid-request");
    assert_eq!(client.0.lock().unwrap().len(), 1);
    // A failed request reports a fixed word, not the client's text.
    assert_eq!(
        plugin.execute(&types, "https://example.org/down", &[], &[]),
        Ok(Ok(vec![]))
    );
    assert_eq!(copy(), "failed");

    // Granted but with nothing to reach: the imports link and answer `unavailable`.
    let files_only = GuestAccess {
        data_dir: Some(data.clone()),
        ..GuestAccess::default()
    };
    let mut plugin = GuestPlugin::with_access(&component, BOTH, files_only).unwrap();
    assert_eq!(
        plugin.execute(&types, "https://example.org/status", &[], &[]),
        Ok(Ok(vec![]))
    );
    assert_eq!(copy(), "unavailable");
    let _ = std::fs::remove_dir_all(&outside);
}

/// `http-local` alone links the `http` import, reaches a local server over plain HTTP,
/// and does not reach the internet.
#[test]
fn http_local_reaches_local_servers_only() {
    let outside = std::env::temp_dir().join(format!("orange-guest-local-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&outside);
    let data = outside.join("data");
    let client = Arc::new(Recorded::default());
    let access = GuestAccess {
        data_dir: Some(data.clone()),
        http: Some(Arc::clone(&client) as Arc<dyn HttpClient>),
        ..GuestAccess::default()
    };
    let engine = GuestEngine::new(GuestLimits::default()).unwrap();
    let component = engine.compile(GUEST.as_bytes()).unwrap();
    let types = TypeRegistryBuilder::core().build().unwrap();
    let local = Grants {
        files: true,
        http_local: true,
        ..Grants::default()
    };
    let mut plugin = GuestPlugin::with_access(&component, local, access).unwrap();
    let copy = || std::fs::read_to_string(data.join("copy")).unwrap();
    for url in [
        "http://127.0.0.1:8880/status",
        "http://[::1]/status",
        "http://studio.local/",
    ] {
        assert_eq!(plugin.execute(&types, url, &[], &[]), Ok(Ok(vec![])));
        assert_eq!(copy(), "hello", "{url}");
    }
    assert_eq!(
        plugin.execute(&types, "https://example.org/status", &[], &[]),
        Ok(Ok(vec![]))
    );
    assert_eq!(copy(), "not-allowed");
    assert_eq!(client.0.lock().unwrap().len(), 3);
    let _ = std::fs::remove_dir_all(&outside);
}
