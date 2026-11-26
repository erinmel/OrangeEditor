//! Connections set up from what adapters say about themselves, with secrets kept in
//! the host's credential store and only their references in the document.
use orange_host::{
    AdapterAbout, Connection, ConnectionScope, Host, HostOptions, SecretError, SecretStore,
    SettingText, secret_name,
};
use orange_testkit::id;
use serde_json::{Map, Value as Json, json};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

fn empty() -> String {
    json!({"format": "orange.graph", "format_version": 1,
        "graph_id": "20000000-0000-4000-8000-0000000000c3", "revision": 1,
        "package_lock": [], "type_lock": [], "events": [], "connections": [],
        "signatures": {}, "functions": [], "exports": [], "editor": {},
        "variables": [], "nodes": [], "edges": []})
    .to_string()
}

#[derive(Debug, Default)]
struct Store(
    Mutex<BTreeMap<String, String>>,
    std::sync::atomic::AtomicUsize,
);
impl SecretStore for Store {
    fn get(&self, name: &str) -> Result<String, SecretError> {
        if !name.starts_with("approval/") {
            self.1.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        self.0
            .lock()
            .unwrap()
            .get(name)
            .cloned()
            .ok_or(SecretError::NotFound)
    }
    fn replace(&self, name: &str, secret: &str) -> Result<(), SecretError> {
        let _ = self
            .0
            .lock()
            .unwrap()
            .insert(name.to_owned(), secret.to_owned());
        Ok(())
    }
}

#[test]
fn local_credential_checks_distinguish_present_missing_and_unavailable() {
    let store = Arc::new(Store::default());
    store.replace("example-token", "fixture-secret").unwrap();
    let (host, _) = Host::new(HostOptions {
        secrets: Some(store.clone()),
        ..HostOptions::default()
    })
    .unwrap();
    assert_eq!(
        host.credential_available("credential://example-token"),
        Ok(true)
    );
    assert_eq!(host.credential_available("credential://missing"), Ok(false));
    assert_eq!(
        host.credential_available("credential://approval/private"),
        Err(SecretError::InvalidName)
    );
    assert_eq!(
        host.credential_available("example-token"),
        Err(SecretError::InvalidName)
    );
    assert_eq!(store.1.load(std::sync::atomic::Ordering::Relaxed), 2);
    let (host, _) = Host::new(HostOptions::default()).unwrap();
    assert!(matches!(
        host.credential_available("credential://example-token"),
        Err(SecretError::Unavailable(_))
    ));
}

#[derive(Debug)]
struct Thing;
impl orange_host::Session for Thing {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// An adapter that names itself and has a password; it opens only with the password
/// "open sesame".
#[derive(Debug)]
struct Locked;
impl orange_host::ConnectionAdapter for Locked {
    fn open(&self, settings: &Map<String, Json>) -> Result<Arc<dyn orange_host::Session>, String> {
        match settings.get("auth").and_then(|auth| auth.get("password")) {
            Some(Json::String(password)) if password == "open sesame" => Ok(Arc::new(Thing)),
            Some(Json::String(password)) => Err(format!("rejected password: {password}")),
            _ => Err("missing password".into()),
        }
    }
    fn about(&self) -> AdapterAbout {
        AdapterAbout {
            label: "Locked box".into(),
            purpose: "Opens with a password.".into(),
        }
    }
    fn texts(&self) -> Vec<SettingText> {
        vec![SettingText {
            setting: "auth/password".into(),
            label: "Password".into(),
            secret: true,
            ..SettingText::default()
        }]
    }
}

/// An adapter that says nothing about itself.
#[derive(Debug)]
struct Plain;
impl orange_host::ConnectionAdapter for Plain {
    fn open(&self, _: &Map<String, Json>) -> Result<Arc<dyn orange_host::Session>, String> {
        Ok(Arc::new(Thing))
    }
}

#[derive(Debug)]
struct Nothing;
impl orange_host::ServiceNode for Nothing {
    fn run(
        &self,
        _: &orange_host::Call<'_>,
    ) -> Result<Vec<orange_host::Value>, orange_host::KernelFault> {
        Ok(Vec::new())
    }
}

#[test]
fn a_service_policy_sees_the_evaluated_input_before_a_provider_runs() {
    use orange_host::{ConnectionAuthorizer, ConnectionRequest, ServiceOperation, ServiceRequest};
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Debug)]
    struct Policy(Arc<AtomicUsize>);
    impl ConnectionAuthorizer for Policy {
        fn allows(&self, _: ConnectionRequest<'_>) -> bool {
            true
        }
        fn allows_operation(&self, request: ServiceRequest<'_>) -> bool {
            assert_eq!(request.operation, ServiceOperation::Effect);
            let [orange_host::Value::Str(target)] = request.inputs else {
                panic!("expected the resolved service input");
            };
            assert_eq!(target.as_str(), "example");
            self.0.fetch_add(1, Ordering::Relaxed);
            false
        }
    }

    struct Provider(Arc<AtomicUsize>);
    impl orange_host::ServiceNode for Provider {
        fn run(
            &self,
            _: &orange_host::Call<'_>,
        ) -> Result<Vec<orange_host::Value>, orange_host::KernelFault> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(Vec::new())
        }
    }

    let policy_calls = Arc::new(AtomicUsize::new(0));
    let provider_calls = Arc::new(AtomicUsize::new(0));
    let definition = orange_sdk::Definition::node("acme.policy", "work", "Work")
        .input("target", "core/string@1")
        .build();
    let package = orange_host::ServicePackage::new("acme.policy", "1.0.0")
        .node(&definition, Provider(Arc::clone(&provider_calls)))
        .unwrap();
    let (mut host, events) = Host::with_services(
        HostOptions {
            connection_authorizer: Some(Arc::new(Policy(Arc::clone(&policy_calls)))),
            ..HostOptions::default()
        },
        vec![package],
    )
    .unwrap();
    let mut draft = host.draft(&empty()).unwrap();
    let trigger = host
        .add_node(&mut draft, "core/on-manual", [0.0, 0.0])
        .unwrap();
    let effect = host
        .add_node(&mut draft, "acme.policy/work", [200.0, 0.0])
        .unwrap();
    host.connect(&mut draft, (trigger, "then"), (effect, "in"))
        .unwrap();
    host.set_input(&mut draft, (effect, "target"), Some("example"))
        .unwrap();
    let flow = host.open(&draft.text().unwrap()).unwrap();
    host.trigger(flow, trigger, None).unwrap();
    orange_testkit::until("the policy refusal", || {
        events.try_iter().any(|event| match event.kind {
            orange_host::FlowEvent::Node(step) => step
                .fault
                .is_some_and(|failure| failure.fault.code.as_str() == "host/ServiceDenied"),
            _ => false,
        })
    });
    assert_eq!(policy_calls.load(Ordering::Relaxed), 1);
    assert_eq!(provider_calls.load(Ordering::Relaxed), 0);
    host.close(flow).unwrap();
}

fn host(store: &Arc<Store>) -> Host {
    use orange_sdk::Definition;
    let opens = Definition::node("acme.kit", "opens", "Opens the box")
        .uses("acme.kit/locked")
        .build();
    let box_kit = orange_host::ServicePackage::new("acme.kit", "1.0.0")
        .adapter("acme.kit/locked", Locked)
        .node(&opens, Nothing)
        .unwrap();
    let other = orange_host::ServicePackage::new("other.kit", "1.0.0")
        .adapter("other.kit/plain-pipe", Plain);
    let options = HostOptions {
        secrets: Some(Arc::clone(store) as Arc<dyn SecretStore>),
        ..HostOptions::default()
    };
    Host::with_services(options, vec![box_kit, other])
        .unwrap()
        .0
}

#[test]
fn every_adapter_is_offered_by_name_from_every_package() {
    let host = host(&Arc::default());
    let list = host.adapter_list();
    let shown: Vec<_> = list
        .iter()
        .map(|adapter| (adapter.id.as_str(), adapter.label.as_str()))
        .collect();
    // Sorted by name; one that gives no name is called after its kind.
    assert_eq!(
        shown,
        [
            ("acme.kit/locked", "Locked box"),
            ("other.kit/plain-pipe", "Plain pipe")
        ]
    );
    assert_eq!(list[0].purpose, "Opens with a password.");
    assert_eq!(list.len(), host.adapters().len());
    assert!(host.adapter_texts("acme.kit/locked")[0].secret);
    assert!(host.adapter_texts("other.kit/plain-pipe").is_empty());
}

#[test]
fn a_typed_secret_goes_to_the_store_and_the_document_keeps_its_reference() {
    let store = Arc::new(Store::default());
    let host = host(&store);
    let mut draft = host.draft(&empty()).unwrap();
    let connection = Connection {
        id: None,
        name: "vault".into(),
        adapter: "acme.kit/locked".into(),
        scope: ConnectionScope::Graph,
        settings: json!({}),
    };
    let secrets = [("auth/password".to_owned(), "open sesame".to_owned())];
    let made = host
        .declare_connection_with_secrets(&mut draft, &connection, &secrets)
        .unwrap();
    let name = secret_name(&made, "auth/password");
    assert!(orange_host::is_secret_name(&name), "{name}");
    assert_eq!(store.get(&name).unwrap(), "open sesame");
    let text = draft.text().unwrap();
    assert!(!text.contains("open sesame"), "{text}");
    let declared = &host.declarations(&draft).connections[0];
    assert_eq!(
        declared.settings,
        json!({"auth": {"password": format!("credential://{name}")}})
    );

    // Authoring checks do not resolve a saved credential reference.
    let Json::Object(settings) = declared.settings.clone() else {
        panic!("settings are an object")
    };
    assert!(host.test_connection("acme.kit/locked", &settings).is_err());
    let _ = store.replace(&name, "xyzzy");
    let refused = host
        .test_connection("acme.kit/locked", &settings)
        .unwrap_err();
    assert!(!refused.contains("xyzzy"), "{refused}");
    let Json::Object(typed) = json!({"auth": {"password": "unsaved secret"}}) else {
        unreachable!()
    };
    let refused = host.test_connection("acme.kit/locked", &typed).unwrap_err();
    assert!(!refused.contains("unsaved secret"), "{refused}");
    let missing = json!({"auth": {"password": "credential://nowhere"}});
    let Json::Object(missing) = missing else {
        unreachable!()
    };
    assert!(host.test_connection("acme.kit/locked", &missing).is_err());

    // A node that uses it is listed as one of its users.
    let node = host
        .add_node(&mut draft, "acme.kit/opens", [0.0, 0.0])
        .unwrap();
    assert_eq!(host.connection_users(&draft, &made), [node]);
    assert!(host.connection_users(&draft, &id(99)).is_empty());
}

#[test]
fn imported_credentials_require_approval_for_the_exact_profile() {
    let store = Arc::new(Store::default());
    store.replace("fixtures/password", "open sesame").unwrap();
    let mut host = host(&store);
    let mut draft = host.draft(&empty()).unwrap();
    let node = host
        .add_node(&mut draft, "acme.kit/opens", [0.0, 0.0])
        .unwrap();
    let mut connection = host.declarations(&draft).connections.remove(0);
    let id = connection.id.clone().unwrap();
    connection.settings = json!({"auth": {"password": "credential://fixtures/password"}});
    host.declare_connection(&mut draft, &connection).unwrap();
    let before = draft.text().unwrap();
    assert!(
        host.open(&before)
            .unwrap_err()
            .to_string()
            .contains("approve this connection")
    );
    assert_eq!(store.1.load(std::sync::atomic::Ordering::Relaxed), 0);
    host.approve_connection_credentials(&draft, &id).unwrap();
    let flow = host.open(&before).unwrap();
    assert_eq!(store.1.load(std::sync::atomic::Ordering::Relaxed), 1);
    connection.settings["endpoint"] = json!("changed");
    host.declare_connection(&mut draft, &connection).unwrap();
    assert!(
        host.update(flow, &draft.text().unwrap())
            .unwrap_err()
            .to_string()
            .contains("approve this connection")
    );
    assert_eq!(host.connection_users(&draft, &id), [node]);
    assert_eq!(store.1.load(std::sync::atomic::Ordering::Relaxed), 1);
    host.close(flow).unwrap();
}

#[test]
fn authoring_and_run_entry_points_share_the_graph_envelope_checks() {
    let (mut host, _) = Host::new(HostOptions::default()).unwrap();
    let clean = empty();
    let flow = host.open(&clean).unwrap();
    let duplicate = clean.replacen("\"editor\":{}", "\"editor\":{},\"editor\":{}", 1);
    assert_ne!(clean, duplicate);
    for error in [
        host.bind(&duplicate).unwrap_err(),
        host.preview(
            &duplicate,
            &[(orange_testkit::node(1), "value".parse().unwrap())],
        )
        .unwrap_err(),
        host.update(flow, &duplicate).unwrap_err(),
    ] {
        assert!(error.to_string().contains("duplicate"), "{error}");
    }
    host.close(flow).unwrap();
}

#[test]
fn a_secret_is_not_kept_without_a_store() {
    let host = Host::with_services(
        HostOptions::default(),
        vec![
            orange_host::ServicePackage::new("acme.kit", "1.0.0")
                .adapter("acme.kit/locked", Locked),
        ],
    )
    .unwrap()
    .0;
    let mut draft = host.draft(&empty()).unwrap();
    let connection = Connection {
        id: None,
        name: "vault".into(),
        adapter: "acme.kit/locked".into(),
        scope: ConnectionScope::Graph,
        settings: json!({}),
    };
    let secrets = [("auth/password".to_owned(), "open sesame".to_owned())];
    assert!(
        host.declare_connection_with_secrets(&mut draft, &connection, &secrets)
            .is_err()
    );
    assert!(host.declarations(&draft).connections.is_empty());
    assert!(!draft.text().unwrap().contains("open sesame"));
}
