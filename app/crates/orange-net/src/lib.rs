//! The base network package: HTTP and WebSocket connections and the nodes that use them.
//!
//! It is an ordinary [`ServicePackage`]: an application adds [`package`] to its host and
//! documents can then declare `org.orange.net/http` and `org.orange.net/websocket`
//! connections and use the request, send and message nodes. Nothing here is special to the host; another crate can
//! provide a better HTTP node, or a different protocol, the same way.
//!
//! `http://`, `https://`, `ws://` and `wss://` endpoints are supported. TLS is rustls with the ring
//! provider; server certificates are verified against the bundled Mozilla roots
//! (`webpki-roots`), not the operating system's store, and there is no way to disable
//! verification.
// Product code reports failures as typed errors; each remaining panic is a documented
// internal invariant (10, "Panic policy").
#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]

mod ws;

pub use ws::MAX_QUEUED_MESSAGES;

use orange_sdk::{
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
/// Adapter kind of an HTTP connection. Settings: `base_url` (required, `http://…` or `https://…`),
/// `timeout_ms` (whole request, default 30,000).
pub const HTTP_ADAPTER: &str = "org.orange.net/http";
/// Sends one request on an HTTP connection. Inputs `method`, `path`, `content` (the request body); outputs
/// `status` and `response`. Any HTTP status is a result, not a failure.
pub const HTTP_REQUEST: &str = "org.orange.net/http-request";

/// Adapter kind of a WebSocket connection. Setting: `url` (required, `ws://…` or
/// `wss://…`). The session connects in the background and reconnects with backoff.
pub const WS_ADAPTER: &str = "org.orange.net/websocket";
/// Queues one text message on a WebSocket connection. Input `text`.
pub const WS_SEND: &str = "org.orange.net/ws-send";
/// Entry fired once per text message received on a WebSocket connection, with the
/// text as `payload`.
pub const WS_MESSAGE: &str = "org.orange.net/ws-message";

/// Largest response body a request node returns.
pub const MAX_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;
const DEFAULT_TIMEOUT_MS: u64 = 30_000;

pub(crate) const fn fault(
    code: &'static str,
    key: &'static str,
    message: &'static str,
) -> KernelFault {
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
        if !base_url.starts_with("http://") && !base_url.starts_with("https://") {
            return Err("`base_url` must start with http:// or https://".into());
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
  "label": {"key": "org.orange.net/http-request-label", "fallback": "HTTP request"},
  "editor": {"category_id": "org.orange.net/category-network",
    "category": {"key": "org.orange.net/category-network", "fallback": "Network"}},
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

/// Downloads an `https://` URL, following redirects only to other `https://` URLs: how
/// an application reads a package's update feed or a release archive. Anything but
/// status 200, or a body over `max_bytes`, is an error. This is not a graph node and
/// needs no host.
pub fn download(url: &str, max_bytes: u64) -> Result<Vec<u8>, String> {
    if !url.starts_with("https://") {
        return Err("the URL must start with https://".into());
    }
    let agent = Agent::config_builder()
        .timeout_global(Some(Duration::from_millis(DEFAULT_TIMEOUT_MS)))
        .https_only(true)
        .http_status_as_error(false)
        .build()
        .new_agent();
    let mut response = agent
        .get(url)
        .call()
        .map_err(|_| "the download could not be completed".to_owned())?;
    let status = response.status().as_u16();
    if status != 200 {
        return Err(format!("the server answered with status {status}"));
    }
    response
        .body_mut()
        .with_config()
        .limit(max_bytes)
        .read_to_vec()
        .map_err(|_| "the download is too large or was cut short".to_owned())
}

/// Sends one HTTPS request and returns the status and body: what an application's
/// client for plugins holding the `http` grant does. Redirects are not followed, so
/// the request reaches only the host it names; a body over `max_bytes` is an error.
pub fn fetch(
    method: &str,
    url: &str,
    headers: &[(String, String)],
    body: &[u8],
    max_bytes: u64,
) -> Result<(u16, Vec<u8>), String> {
    if !url.starts_with("https://") {
        return Err("the URL must start with https://".into());
    }
    let agent: Agent = Agent::config_builder()
        .timeout_global(Some(Duration::from_millis(DEFAULT_TIMEOUT_MS)))
        .https_only(true)
        .max_redirects(0)
        .http_status_as_error(false)
        .build()
        .new_agent();
    let method = Method::from_bytes(method.as_bytes()).map_err(|_| "the method is not valid")?;
    let mut request = Request::builder().method(method).uri(url);
    for (name, value) in headers {
        request = request.header(name.as_str(), value.as_str());
    }
    let request = request
        .body(body.to_vec())
        .map_err(|_| "the request is not valid".to_owned())?;
    let mut response = agent
        .run(request)
        .map_err(|_| "the request could not be completed".to_owned())?;
    let status = response.status().as_u16();
    let body = response
        .body_mut()
        .with_config()
        .limit(max_bytes)
        .read_to_vec()
        .map_err(|_| "the response is too large or was cut short".to_owned())?;
    Ok((status, body))
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    let digits = text.as_bytes();
    if !digits.len().is_multiple_of(2) {
        return None;
    }
    digits
        .chunks(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).ok()?;
            u8::from_str_radix(text, 16).ok()
        })
        .collect()
}

/// Checks that `signature` is the publisher's Ed25519 signature over exactly `feed`.
/// `key` is a package's `update.key` (`ed25519:<64 hex>`); `signature` is the content of
/// the feed's `.sig` document, 128 hexadecimal digits, surrounding white space ignored.
pub fn verify_feed(feed: &[u8], signature: &[u8], key: &str) -> Result<(), String> {
    let refused = || "the update feed is not signed by the package's key".to_owned();
    let key = key
        .strip_prefix("ed25519:")
        .and_then(unhex)
        .filter(|key| key.len() == 32)
        .ok_or("the package's update key is not an Ed25519 public key")?;
    let signature = std::str::from_utf8(signature)
        .ok()
        .and_then(|text| unhex(text.trim()))
        .filter(|signature| signature.len() == 64)
        .ok_or_else(refused)?;
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, key)
        .verify(feed, &signature)
        .map_err(|_| refused())
}

/// The lowercase hexadecimal SHA-256 of `bytes` as `sha256:<64 hex>`: the form a feed's
/// `content_digest` has for a native plugin library.
pub fn sha256(bytes: &[u8]) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, bytes);
    let mut text = String::from("sha256:");
    for byte in digest.as_ref() {
        text.push_str(&format!("{byte:02x}"));
    }
    text
}

/// The package to pass to `Host::with_services`.
pub fn package() -> ServicePackage {
    let package = ServicePackage::new(PACKAGE, env!("CARGO_PKG_VERSION"))
        .about(
            "Network",
            "HTTP requests and WebSocket connections, with TLS.",
        )
        .adapter(HTTP_ADAPTER, HttpAdapter)
        .adapter(WS_ADAPTER, ws::WsAdapter)
        .node(HTTP_REQUEST_DEFINITION, HttpRequest)
        .and_then(|package| package.node(ws::WS_SEND_DEFINITION, ws::WsSend))
        .and_then(|package| package.source(ws::WS_MESSAGE_DEFINITION, ws::WsMessage));
    match package {
        Ok(package) => package,
        // The definition is a constant of this crate, checked by its tests.
        Err(error) => unreachable!("built-in definition is valid: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::{sha256, verify_feed};
    use ring::signature::{Ed25519KeyPair, KeyPair};

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn a_feed_is_accepted_only_with_its_publishers_signature() {
        let publisher = Ed25519KeyPair::from_seed_unchecked(&[7; 32]).unwrap();
        let other = Ed25519KeyPair::from_seed_unchecked(&[8; 32]).unwrap();
        let key = format!("ed25519:{}", hex(publisher.public_key().as_ref()));
        let feed = br#"{"format": "orange.update-feed", "format_version": 1, "releases": []}"#;
        let signature = format!("{}\n", hex(publisher.sign(feed).as_ref()));

        assert_eq!(verify_feed(feed, signature.as_bytes(), &key), Ok(()));
        // A changed feed, another signer, a damaged signature and a bad key are refused.
        assert!(verify_feed(b"{}", signature.as_bytes(), &key).is_err());
        let forged = hex(other.sign(feed).as_ref());
        assert!(verify_feed(feed, forged.as_bytes(), &key).is_err());
        assert!(verify_feed(feed, b"00", &key).is_err());
        assert!(verify_feed(feed, b"not hex", &key).is_err());
        assert!(verify_feed(feed, signature.as_bytes(), "ed25519:00").is_err());
    }

    #[test]
    fn the_digest_of_a_file_has_the_feed_form() {
        assert_eq!(
            sha256(b"abc"),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
