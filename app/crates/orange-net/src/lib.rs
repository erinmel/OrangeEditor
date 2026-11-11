//! The base network package: HTTP connections and a request node.
//!
//! It is an ordinary [`ServicePackage`]: an application adds [`package`] to its host and
//! documents can then declare `org.orange.net/http` connections and use
//! `org.orange.net/http-request`. Nothing here is special to the host; another crate can
//! provide a better HTTP node, or a different protocol, the same way.
//!
//! Only `http://` endpoints are reachable. The repository's dependency policy admits no
//! TLS implementation yet, so `https://` is refused when the connection is opened rather
//! than failing later on the wire.
// Product code reports failures as typed errors; each remaining panic is a documented
// internal invariant (10, "Panic policy").
#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]

use orange_host::{
    Call, ConnectionAdapter, FaultText, KernelFault, ServiceNode, ServicePackage, Session, Value,
};
use serde_json::{Map, Value as Json};
use std::any::Any;
use std::sync::Arc;
use std::time::Duration;
use ureq::Agent;
use ureq::http::{Method, Request};

/// Package namespace of the definitions and adapter kinds here.
pub const PACKAGE: &str = "org.orange.net";
/// Adapter kind of an HTTP connection. Settings: `base_url` (required, `http://…`),
/// `timeout_ms` (whole request, default 30,000).
pub const HTTP_ADAPTER: &str = "org.orange.net/http";
/// Sends one request on an HTTP connection. Inputs `method`, `path`, `content` (the request body); outputs
/// `status` and `response`. Any HTTP status is a result, not a failure.
pub const HTTP_REQUEST: &str = "org.orange.net/http-request";

/// Largest response body a request node returns.
pub const MAX_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;
const DEFAULT_TIMEOUT_MS: u64 = 30_000;

const fn fault(code: &'static str, key: &'static str, message: &'static str) -> KernelFault {
    KernelFault {
        code: FaultText::Static(code),
        message_key: FaultText::Static(key),
        message,
    }
}
const NO_SESSION: KernelFault = fault(
    "org.orange.net/Connection",
    "org.orange.net/connection-kind",
    "The node needs an HTTP connection.",
);
const BAD_REQUEST: KernelFault = fault(
    "org.orange.net/Request",
    "org.orange.net/request-invalid",
    "The method or path is not a valid HTTP request.",
);
const TRANSPORT: KernelFault = fault(
    "org.orange.net/Transport",
    "org.orange.net/transport-failed",
    "The request could not be completed.",
);
const RESPONSE: KernelFault = fault(
    "org.orange.net/Response",
    "org.orange.net/response-unreadable",
    "The response body is too large or is not text.",
);

/// An HTTP connection: one pooled client bound to a base URL.
#[derive(Debug)]
struct HttpSession {
    agent: Agent,
    base_url: String,
}
impl Session for HttpSession {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Debug)]
struct HttpAdapter;
impl ConnectionAdapter for HttpAdapter {
    fn open(&self, settings: &Map<String, Json>) -> Result<Arc<dyn Session>, String> {
        let base_url = settings
            .get("base_url")
            .and_then(Json::as_str)
            .ok_or("`base_url` is required")?;
        if base_url.starts_with("https://") {
            return Err("https is not available in this build; use an http:// endpoint".into());
        }
        if !base_url.starts_with("http://") {
            return Err("`base_url` must start with http://".into());
        }
        let timeout = match settings.get("timeout_ms") {
            None => DEFAULT_TIMEOUT_MS,
            Some(value) => value
                .as_u64()
                .filter(|ms| *ms > 0)
                .ok_or("`timeout_ms` must be a positive integer")?,
        };
        // Opening only builds the client; nothing is sent until a node runs.
        let agent = Agent::config_builder()
            .timeout_global(Some(Duration::from_millis(timeout)))
            .http_status_as_error(false)
            .build()
            .new_agent();
        Ok(Arc::new(HttpSession {
            agent,
            base_url: base_url.trim_end_matches('/').to_owned(),
        }))
    }
}

#[derive(Debug)]
struct HttpRequest;
impl ServiceNode for HttpRequest {
    fn run(&self, call: &Call<'_>) -> Result<Vec<Value>, KernelFault> {
        let session = call
            .session
            .and_then(|session| session.as_any().downcast_ref::<HttpSession>())
            .ok_or(NO_SESSION)?;
        let [Value::Str(method), Value::Str(path), Value::Str(body)] = call.inputs else {
            return Err(BAD_REQUEST);
        };
        let method = Method::from_bytes(method.as_bytes()).map_err(|_| BAD_REQUEST)?;
        if !path.is_empty() && !path.starts_with('/') {
            return Err(BAD_REQUEST);
        }
        let request = Request::builder()
            .method(method)
            .uri(format!("{}{path}", session.base_url))
            .body(body.as_bytes().to_vec())
            .map_err(|_| BAD_REQUEST)?;
        let mut response = session.agent.run(request).map_err(|_| TRANSPORT)?;
        let status = i64::from(response.status().as_u16());
        let text = response
            .body_mut()
            .with_config()
            .limit(MAX_RESPONSE_BYTES)
            .read_to_string()
            .map_err(|_| RESPONSE)?;
        Ok(vec![Value::I64(status), Value::Str(Arc::new(text))])
    }
}

const HTTP_REQUEST_DEFINITION: &str = r#"{
  "definition_id": "org.orange.net/http-request",
  "definition_version": 1,
  "config_version": 1,
  "kind": "async",
  "suspends": "always",
  "determinism": "external",
  "connection": true,
  "ports": [
    {"port_id": "method", "direction": "input", "kind": "data", "type": "core/string@1"},
    {"port_id": "path", "direction": "input", "kind": "data", "type": "core/string@1"},
    {"port_id": "content", "direction": "input", "kind": "data", "type": "core/string@1"},
    {"port_id": "status", "direction": "output", "kind": "data", "type": "core/i64@1",
      "source": "execution", "available_on": ["then"]},
    {"port_id": "response", "direction": "output", "kind": "data", "type": "core/string@1",
      "source": "execution", "available_on": ["then"]}
  ]
}"#;

/// The package to pass to `Host::with_services`.
pub fn package() -> ServicePackage {
    let package = ServicePackage::new(PACKAGE, env!("CARGO_PKG_VERSION"))
        .adapter(HTTP_ADAPTER, HttpAdapter)
        .node(HTTP_REQUEST_DEFINITION, HttpRequest);
    match package {
        Ok(package) => package,
        // The definition is a constant of this crate, checked by its tests.
        Err(error) => unreachable!("built-in definition is valid: {error}"),
    }
}
