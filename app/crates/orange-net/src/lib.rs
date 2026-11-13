//! The base network package: HTTP and WebSocket connections and the nodes that use them.
//!
//! It is an ordinary [`ServicePackage`]: an application adds [`package`] to its host and
//! documents can then declare `org.orange.net/http` and `org.orange.net/websocket`
//! connections and use the request, send and message nodes. Nothing here is special to the host; another crate can
//! provide a better HTTP node, or a different protocol, the same way.
//!
//! `http://`, `https://`, `ws://`, `wss://`, `mqtt://` and `mqtts://` endpoints are supported. TLS is rustls with the ring
//! provider; server certificates are verified against the bundled Mozilla roots
//! (`webpki-roots`), not the operating system's store, and there is no way to disable
//! verification.
// Product code reports failures as typed errors; each remaining panic is a documented
// internal invariant (10, "Panic policy").
#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]

mod mqtt;
pub mod oauth;
mod ws;

pub use ws::MAX_QUEUED_MESSAGES;

use orange_sdk::{
    Call, ConnectionAdapter, EmitError, Emitter, FaultText, KernelFault, Listeners, ServiceNode,
    ServicePackage, ServiceSource, Session, Value,
};
use serde_json::{Map, Value as Json};
use std::any::Any;
use std::io::Read;
use std::sync::Arc;
use std::time::Duration;
use ureq::Agent;
use ureq::http::{Method, Request};

/// Package namespace of the definitions and adapter kinds here.
pub const PACKAGE: &str = "org.orange.net";
/// Adapter kind of an HTTP connection. Settings: `base_url` (required, `http://…` or `https://…`),
/// `timeout_ms` (whole request, default 30,000) and the authorization settings
/// `bearer` (a token, sent as `Authorization: Bearer …`), `basic` (`{"user", "password"}`)
/// and `headers` (an object of header names and values sent with every request), or
/// `oauth` (`{"token_url", "client_id", "client_secret"}` for the client-credentials
/// grant, or with `"refresh_token"` for the refresh grant; optional `scope` and
/// `client_auth`: `body` or `basic`), which gets an access token when first needed,
/// replaces it before it expires and once more if the server answers 401.
/// `stream_timeout_ms` (default 600,000) limits one streamed response. A document
/// writes a secret value as `credential://<name>`; the host supplies it.
pub const HTTP_ADAPTER: &str = "org.orange.net/http";
/// Sends one request on an HTTP connection. Inputs `method`, `path`, `content` (the request body); outputs
/// `status` and `response`. Any HTTP status is a result, not a failure.
pub const HTTP_REQUEST: &str = "org.orange.net/http-request";

/// Sends one request with a binary body on an HTTP connection and returns the response
/// as bytes, whatever it holds: protobuf, an encrypted payload, an image. Inputs
/// `method`, `path`, `content_type` (empty for none), `headers` (a list of `Name: value`
/// lines sent with this request only) and `content`; outputs `status`,
/// `response_type` (the response's content type) and `response`.
pub const HTTP_REQUEST_BYTES: &str = "org.orange.net/http-request-bytes";
/// Queues one binary message on a WebSocket connection. Input `data`.
pub const WS_SEND_BYTES: &str = "org.orange.net/ws-send-bytes";
/// Entry fired once per binary message received on a WebSocket connection, with the
/// bytes as `payload`.
pub const WS_BINARY: &str = "org.orange.net/ws-binary";

/// Adapter kind of a WebSocket connection. Settings: `url` (required, `ws://…` or
/// `wss://…`) and, for the opening handshake, `bearer`, `basic` and `headers` as for
/// an HTTP connection. The session connects in the background and reconnects with backoff.
pub const WS_ADAPTER: &str = "org.orange.net/websocket";
/// Queues one text message on a WebSocket connection. Input `text`.
pub const WS_SEND: &str = "org.orange.net/ws-send";
/// Entry fired once per text message received on a WebSocket connection, with the
/// text as `payload`.
pub const WS_MESSAGE: &str = "org.orange.net/ws-message";

/// Adapter kind of an MQTT 3.1.1 connection. Settings: `url` (required, `mqtt://host`
/// or `mqtts://host` for TLS, with an optional `:port`), `username` and `password`
/// (each can be a `credential://<name>` reference), `client_id` (generated when
/// absent), `topics` (the topic filters to subscribe to), `qos` (0 or 1, default 0,
/// for subscriptions and publishing), `keep_alive_s` (5 to 3,600, default 30) and, for
/// a broker with a private certificate authority, `ca_file` (a PEM file whose
/// authorities are then the only ones trusted).
pub const MQTT_ADAPTER: &str = "org.orange.net/mqtt";
/// Queues one text message for a topic on an MQTT connection. Inputs `topic`, `text`
/// and `retain`.
pub const MQTT_PUBLISH: &str = "org.orange.net/mqtt-publish";
/// Queues one binary message for a topic. Inputs `topic`, `data` and `retain`.
pub const MQTT_PUBLISH_BYTES: &str = "org.orange.net/mqtt-publish-bytes";
/// Entry fired once per message received on an MQTT connection's topics whose payload
/// is text, with `[topic, text]` as `payload`.
pub const MQTT_MESSAGE: &str = "org.orange.net/mqtt-message";
/// Entry fired once per message received on an MQTT connection's topics, with the
/// payload bytes as `payload`. A connection that needs to tell topics apart for
/// binary messages subscribes to one topic.
pub const MQTT_BINARY: &str = "org.orange.net/mqtt-binary";

/// Largest response body a request node returns.
pub const MAX_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;
const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_STREAM_TIMEOUT_MS: u64 = 600_000;
/// Longest line of a streamed response.
pub const MAX_STREAM_LINE_BYTES: usize = 1024 * 1024;
/// Sends a request like [`HTTP_REQUEST_BYTES`] and reads the response as it arrives:
/// each non-empty line fires the connection's [`HTTP_STREAM_LINE`] entries, and the node
/// continues when the response ends, with `status` and the number of `lines`. It is
/// how a graph follows server-sent events or a response generated piece by piece.
pub const HTTP_STREAM: &str = "org.orange.net/http-stream";
/// Entry fired once per line of a response streamed on an HTTP connection, with the
/// line as `payload`.
pub const HTTP_STREAM_LINE: &str = "org.orange.net/http-stream-line";

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
const AUTHORIZATION: KernelFault = fault(
    "org.orange.net/Authorization",
    "org.orange.net/authorization-failed",
    "The authorization server did not issue an access token.",
);
const RESPONSE: KernelFault = fault(
    "org.orange.net/Response",
    "org.orange.net/response-unreadable",
    "The response body is too large or, for a text request, is not text.",
);

/// Most headers one connection adds.
pub const MAX_HEADERS: usize = 32;

pub(crate) fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut text = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let group = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let bits = u32::from(group[0]) << 16 | u32::from(group[1]) << 8 | u32::from(group[2]);
        for (index, shift) in [18u32, 12, 6, 0].into_iter().enumerate() {
            if index <= chunk.len() {
                text.push(char::from(ALPHABET[(bits >> shift & 63) as usize]));
            } else {
                text.push('=');
            }
        }
    }
    text
}

/// The headers a connection's `bearer`, `basic` and `headers` settings add to what it
/// sends. Error text names the setting, never its value.
pub(crate) fn auth_headers(settings: &Map<String, Json>) -> Result<Vec<(String, String)>, String> {
    let value = |text: &str| !text.is_empty() && !text.chars().any(char::is_control);
    let mut headers = Vec::new();
    if let Some(extra) = settings.get("headers") {
        let extra = extra
            .as_object()
            .ok_or("`headers` must be an object of header names and values")?;
        for (name, text) in extra {
            let named = !name.is_empty()
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'));
            let text = text.as_str().filter(|text| value(text));
            match (named, text) {
                (true, Some(text)) => headers.push((name.clone(), text.to_owned())),
                _ => return Err("`headers` has a name or value that is not valid".into()),
            }
        }
    }
    let authorization = match (settings.get("bearer"), settings.get("basic")) {
        (None, None) => None,
        (Some(_), Some(_)) => return Err("give `bearer` or `basic`, not both".into()),
        (Some(token), None) => {
            let token = token
                .as_str()
                .filter(|token| value(token) && !token.contains(' '))
                .ok_or("`bearer` must be a token")?;
            Some(format!("Bearer {token}"))
        }
        (None, Some(basic)) => {
            let part = |name: &str| basic.get(name).and_then(Json::as_str).filter(|t| value(t));
            let (user, password) = part("user")
                .filter(|user| !user.contains(':'))
                .zip(part("password"))
                .ok_or("`basic` must be an object with `user` and `password`")?;
            Some(format!(
                "Basic {}",
                base64(format!("{user}:{password}").as_bytes())
            ))
        }
    };
    if let Some(authorization) = authorization {
        if headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("authorization"))
        {
            return Err("`headers` already sets Authorization".into());
        }
        headers.push(("Authorization".to_owned(), authorization));
    }
    if headers.len() > MAX_HEADERS {
        return Err(format!("more than {MAX_HEADERS} headers"));
    }
    Ok(headers)
}

/// An HTTP connection: one pooled client bound to a base URL.
struct HttpSession {
    agent: Agent,
    /// For responses read as they arrive: no limit on the whole exchange.
    stream_agent: Agent,
    base_url: String,
    headers: Vec<(String, String)>,
    oauth: Option<oauth::OAuth>,
    /// The entries that receive the lines of streamed responses.
    listeners: Arc<Listeners>,
}
/// Header values are left out: they can be credentials.
impl std::fmt::Debug for HttpSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpSession")
            .field("base_url", &self.base_url)
            .field("headers", &self.headers.len())
            .finish_non_exhaustive()
    }
}
impl HttpSession {
    /// Sends one request with the connection's headers, then `extra`, each a
    /// `Name: value` line. With OAuth the request carries the current access token;
    /// if the server answers 401 the token is replaced and the request sent once more.
    fn send(
        &self,
        method: &str,
        path: &str,
        content_type: &str,
        extra: &[Arc<String>],
        body: Vec<u8>,
        streamed: bool,
    ) -> Result<ureq::http::Response<ureq::Body>, KernelFault> {
        let Some(oauth) = &self.oauth else {
            return self.send_once(method, path, content_type, extra, body, streamed, None);
        };
        let token = oauth.token(false).map_err(|_| AUTHORIZATION)?;
        let response = self.send_once(
            method,
            path,
            content_type,
            extra,
            body.clone(),
            streamed,
            Some(&token),
        )?;
        if response.status().as_u16() != 401 {
            return Ok(response);
        }
        let token = oauth.token(true).map_err(|_| AUTHORIZATION)?;
        self.send_once(
            method,
            path,
            content_type,
            extra,
            body,
            streamed,
            Some(&token),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn send_once(
        &self,
        method: &str,
        path: &str,
        content_type: &str,
        extra: &[Arc<String>],
        body: Vec<u8>,
        streamed: bool,
        token: Option<&str>,
    ) -> Result<ureq::http::Response<ureq::Body>, KernelFault> {
        if extra.len() > MAX_HEADERS {
            return Err(BAD_REQUEST);
        }
        let method = Method::from_bytes(method.as_bytes()).map_err(|_| BAD_REQUEST)?;
        if !path.is_empty() && !path.starts_with('/') {
            return Err(BAD_REQUEST);
        }
        let mut request = Request::builder()
            .method(method)
            .uri(format!("{}{path}", self.base_url));
        for (name, value) in &self.headers {
            request = request.header(name.as_str(), value.as_str());
        }
        if let Some(token) = token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        if !content_type.is_empty() {
            request = request.header("Content-Type", content_type);
        }
        for line in extra {
            let (name, value) = line.split_once(':').ok_or(BAD_REQUEST)?;
            let (name, value) = (name.trim(), value.trim());
            let valid = !name.is_empty()
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
                && !value.chars().any(char::is_control);
            if !valid {
                return Err(BAD_REQUEST);
            }
            request = request.header(name, value);
        }
        let request = request.body(body).map_err(|_| BAD_REQUEST)?;
        let agent = if streamed {
            &self.stream_agent
        } else {
            &self.agent
        };
        agent.run(request).map_err(|_| TRANSPORT)
    }
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
        let stream_timeout = match settings.get("stream_timeout_ms") {
            None => DEFAULT_STREAM_TIMEOUT_MS,
            Some(value) => value
                .as_u64()
                .filter(|ms| *ms > 0)
                .ok_or("`stream_timeout_ms` must be a positive integer")?,
        };
        // A streamed response may last long: only reaching the server and the whole
        // body have limits.
        let stream_agent = Agent::config_builder()
            .timeout_connect(Some(Duration::from_millis(timeout)))
            .timeout_recv_response(Some(Duration::from_millis(timeout)))
            .timeout_recv_body(Some(Duration::from_millis(stream_timeout)))
            .http_status_as_error(false)
            .build()
            .new_agent();
        let headers = auth_headers(settings)?;
        let oauth = match settings.get("oauth") {
            None => None,
            Some(_)
                if headers
                    .iter()
                    .any(|(name, _)| name.eq_ignore_ascii_case("authorization")) =>
            {
                return Err("give `oauth` or another authorization, not both".into());
            }
            Some(setting) => Some(oauth::OAuth::from_setting(setting)?),
        };
        Ok(Arc::new(HttpSession {
            agent,
            stream_agent,
            base_url: base_url.trim_end_matches('/').to_owned(),
            headers,
            oauth,
            listeners: Arc::new(Listeners::new()),
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
        let mut response = session.send(method, path, "", &[], body.as_bytes().to_vec(), false)?;
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

#[derive(Debug)]
struct HttpRequestBytes;
impl ServiceNode for HttpRequestBytes {
    fn run(&self, call: &Call<'_>) -> Result<Vec<Value>, KernelFault> {
        let session = call
            .session
            .and_then(|session| session.as_any().downcast_ref::<HttpSession>())
            .ok_or(NO_SESSION)?;
        let [
            Value::Str(method),
            Value::Str(path),
            Value::Str(content_type),
            Value::List(headers),
            Value::Bytes(body),
        ] = call.inputs
        else {
            return Err(BAD_REQUEST);
        };
        let orange_sdk::ListRepr::Str(headers) = headers.as_ref() else {
            return Err(BAD_REQUEST);
        };
        if content_type.chars().any(char::is_control) {
            return Err(BAD_REQUEST);
        }
        let mut response = session.send(
            method,
            path,
            content_type,
            headers,
            body.as_ref().clone(),
            false,
        )?;
        let status = i64::from(response.status().as_u16());
        let kind = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let bytes = response
            .body_mut()
            .with_config()
            .limit(MAX_RESPONSE_BYTES)
            .read_to_vec()
            .map_err(|_| RESPONSE)?;
        Ok(vec![
            Value::I64(status),
            Value::Str(Arc::new(kind)),
            Value::Bytes(Arc::new(bytes)),
        ])
    }
}

/// Sends a request and hands the response to the connection's line entries as it
/// arrives, one line at a time, then continues with the status.
#[derive(Debug)]
struct HttpStream;
impl ServiceNode for HttpStream {
    fn run(&self, call: &Call<'_>) -> Result<Vec<Value>, KernelFault> {
        use std::io::BufRead;
        let session = call
            .session
            .and_then(|session| session.as_any().downcast_ref::<HttpSession>())
            .ok_or(NO_SESSION)?;
        let [
            Value::Str(method),
            Value::Str(path),
            Value::Str(content_type),
            Value::List(headers),
            Value::Bytes(body),
        ] = call.inputs
        else {
            return Err(BAD_REQUEST);
        };
        let orange_sdk::ListRepr::Str(headers) = headers.as_ref() else {
            return Err(BAD_REQUEST);
        };
        let mut response = session.send(
            method,
            path,
            content_type,
            headers,
            body.as_ref().clone(),
            true,
        )?;
        let status = i64::from(response.status().as_u16());
        let mut reader = std::io::BufReader::new(response.body_mut().as_reader());
        let mut line = Vec::new();
        let mut lines = 0i64;
        loop {
            line.clear();
            // One byte over the limit tells a line that is too long from one that fits.
            let read = (&mut reader)
                .take(MAX_STREAM_LINE_BYTES as u64 + 1)
                .read_until(b'\n', &mut line)
                .map_err(|_| TRANSPORT)?;
            if read == 0 {
                break;
            }
            if line.len() > MAX_STREAM_LINE_BYTES {
                return Err(RESPONSE);
            }
            if call.lease.is_cancelled() {
                break;
            }
            let text = String::from_utf8_lossy(&line);
            let text = text.trim_end_matches(['\n', '\r']);
            if text.is_empty() {
                continue;
            }
            lines += 1;
            let payload = Value::Str(Arc::new(text.to_owned()));
            let listeners = session.listeners.emitters();
            for emitter in listeners {
                // A flow that is behind slows the reading down instead of losing a
                // line; the server then sees a slow reader.
                while emitter.emit(Some(payload.clone())) == Err(EmitError::Busy) {
                    if call.lease.is_cancelled() {
                        return Ok(vec![Value::I64(status), Value::I64(lines)]);
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
        }
        Ok(vec![Value::I64(status), Value::I64(lines)])
    }
}

/// Fires its entry once per line of every response streamed on the connection.
#[derive(Debug)]
struct HttpStreamLine;
impl ServiceSource for HttpStreamLine {
    fn start(
        &self,
        session: Option<Arc<dyn Session>>,
        emitter: Emitter,
    ) -> Result<Box<dyn Send>, String> {
        let http = session
            .as_deref()
            .and_then(|session| session.as_any().downcast_ref::<HttpSession>())
            .ok_or("the entry needs an HTTP connection")?;
        Ok(Listeners::listen(
            Arc::clone(&http.listeners),
            |listeners| listeners,
            (),
            emitter,
        ))
    }
}

const HTTP_STREAM_DEFINITION: &str = r#"{
  "definition_id": "org.orange.net/http-stream",
  "label": {"key": "org.orange.net/http-stream-label", "fallback": "HTTP request (streamed)"},
  "description": {"key": "org.orange.net/http-stream-description",
    "fallback": "Sends a request and delivers the response line by line as it arrives."},
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
    {"port_id": "content_type", "direction": "input", "kind": "data", "type": "core/string@1"},
    {"port_id": "headers", "direction": "input", "kind": "data",
      "type": "core/list@1<core/string@1>"},
    {"port_id": "content", "direction": "input", "kind": "data", "type": "core/bytes@1"},
    {"port_id": "status", "direction": "output", "kind": "data", "type": "core/i64@1",
      "source": "execution", "available_on": ["then"]},
    {"port_id": "lines", "direction": "output", "kind": "data", "type": "core/i64@1",
      "source": "execution", "available_on": ["then"]}
  ]
}"#;

const HTTP_STREAM_LINE_DEFINITION: &str = r#"{
  "definition_id": "org.orange.net/http-stream-line",
  "label": {"key": "org.orange.net/http-stream-line-label", "fallback": "On HTTP stream line"},
  "editor": {"category_id": "org.orange.net/category-network",
    "category": {"key": "org.orange.net/category-network", "fallback": "Network"}},
  "definition_version": 1,
  "config_version": 1,
  "kind": "entry",
  "determinism": "external",
  "connection": true,
  "ports": [
    {"port_id": "payload", "direction": "output", "kind": "data", "type": "core/string@1",
      "source": "payload"}
  ]
}"#;

const HTTP_REQUEST_BYTES_DEFINITION: &str = r#"{
  "definition_id": "org.orange.net/http-request-bytes",
  "label": {"key": "org.orange.net/http-request-bytes-label", "fallback": "HTTP request (binary)"},
  "description": {"key": "org.orange.net/http-request-bytes-description",
    "fallback": "Sends a request with a binary body and returns the response as bytes."},
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
    {"port_id": "content_type", "direction": "input", "kind": "data", "type": "core/string@1"},
    {"port_id": "headers", "direction": "input", "kind": "data",
      "type": "core/list@1<core/string@1>"},
    {"port_id": "content", "direction": "input", "kind": "data", "type": "core/bytes@1"},
    {"port_id": "status", "direction": "output", "kind": "data", "type": "core/i64@1",
      "source": "execution", "available_on": ["then"]},
    {"port_id": "response_type", "direction": "output", "kind": "data", "type": "core/string@1",
      "source": "execution", "available_on": ["then"]},
    {"port_id": "response", "direction": "output", "kind": "data", "type": "core/bytes@1",
      "source": "execution", "available_on": ["then"]}
  ]
}"#;

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

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Makes a new feed-signing key. The first value is the private key, as text to keep
/// in a credential store and nowhere else; the second is the public key in the form a
/// manifest's `update.key` takes, `ed25519:<64 hex>`.
pub fn new_feed_key() -> Result<(String, String), String> {
    use ring::signature::KeyPair;
    let failed = || "the key could not be generated".to_owned();
    let private = ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
        .map_err(|_| failed())?;
    let pair =
        ring::signature::Ed25519KeyPair::from_pkcs8(private.as_ref()).map_err(|_| failed())?;
    Ok((
        hex(private.as_ref()),
        format!("ed25519:{}", hex(pair.public_key().as_ref())),
    ))
}

/// Signs the exact bytes of a feed with a private key from [`new_feed_key`]. Returns
/// the content of the feed's `.sig` document and the public key that verifies it.
pub fn sign_feed(feed: &[u8], private_key: &str) -> Result<(String, String), String> {
    use ring::signature::KeyPair;
    let pair = unhex(private_key.trim())
        .and_then(|bytes| ring::signature::Ed25519KeyPair::from_pkcs8(&bytes).ok())
        .ok_or("the private key is not a feed-signing key")?;
    Ok((
        hex(pair.sign(feed).as_ref()),
        format!("ed25519:{}", hex(pair.public_key().as_ref())),
    ))
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
        .adapter(MQTT_ADAPTER, mqtt::MqttAdapter)
        .node(HTTP_REQUEST_DEFINITION, HttpRequest)
        .and_then(|package| package.node(HTTP_REQUEST_BYTES_DEFINITION, HttpRequestBytes))
        .and_then(|package| package.node(HTTP_STREAM_DEFINITION, HttpStream))
        .and_then(|package| package.source(HTTP_STREAM_LINE_DEFINITION, HttpStreamLine))
        .and_then(|package| package.node(ws::WS_SEND_DEFINITION, ws::WsSend { binary: false }))
        .and_then(|package| package.node(ws::WS_SEND_BYTES_DEFINITION, ws::WsSend { binary: true }))
        .and_then(|package| {
            package.source(ws::WS_MESSAGE_DEFINITION, ws::WsMessage { binary: false })
        })
        .and_then(|package| {
            package.source(ws::WS_BINARY_DEFINITION, ws::WsMessage { binary: true })
        })
        .and_then(|package| {
            package.node(
                mqtt::PUBLISH_DEFINITION,
                mqtt::MqttPublish { binary: false },
            )
        })
        .and_then(|package| {
            package.node(
                mqtt::PUBLISH_BYTES_DEFINITION,
                mqtt::MqttPublish { binary: true },
            )
        })
        .and_then(|package| {
            package.source(
                mqtt::MESSAGE_DEFINITION,
                mqtt::MqttMessage { binary: false },
            )
        })
        .and_then(|package| {
            package.source(mqtt::BINARY_DEFINITION, mqtt::MqttMessage { binary: true })
        });
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
    fn a_generated_key_signs_a_feed_that_its_public_key_verifies() {
        let (private, public) = super::new_feed_key().unwrap();
        let (other_private, other_public) = super::new_feed_key().unwrap();
        assert_ne!(public, other_public);
        let feed = br#"{"format": "orange.update-feed", "format_version": 1, "releases": []}"#;
        let (signature, signer) = super::sign_feed(feed, &private).unwrap();
        assert_eq!(signer, public);
        assert_eq!(signature.len(), 128);
        assert_eq!(verify_feed(feed, signature.as_bytes(), &public), Ok(()));
        assert!(verify_feed(feed, signature.as_bytes(), &other_public).is_err());
        let (forged, _) = super::sign_feed(feed, &other_private).unwrap();
        assert!(verify_feed(feed, forged.as_bytes(), &public).is_err());
        // The public key is not a private key, and neither is arbitrary text.
        assert!(super::sign_feed(feed, &public).is_err());
        assert!(super::sign_feed(feed, "00ff").is_err());
    }

    #[test]
    fn the_digest_of_a_file_has_the_feed_form() {
        assert_eq!(
            sha256(b"abc"),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
