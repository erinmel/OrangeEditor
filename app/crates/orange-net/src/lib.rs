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

mod feed;
mod http;
mod mqtt;
pub mod oauth;
mod ws;

pub use feed::{download, fetch, new_feed_key, sha256, sign_feed, verify_feed};
pub use ws::MAX_QUEUED_MESSAGES;

use orange_sdk::{FaultText, KernelFault, ServicePackage};
use serde_json::{Map, Value as Json};

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

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The package to pass to `Host::with_services`.
pub fn package() -> ServicePackage {
    let package = ServicePackage::new(PACKAGE, env!("CARGO_PKG_VERSION"))
        .about(
            "Network",
            "HTTP requests and WebSocket connections, with TLS.",
        )
        .adapter(HTTP_ADAPTER, http::HttpAdapter)
        .adapter(WS_ADAPTER, ws::WsAdapter)
        .adapter(MQTT_ADAPTER, mqtt::MqttAdapter)
        .node(&http::http_request_definition(), http::HttpRequest)
        .and_then(|package| {
            package.node(
                &http::http_request_bytes_definition(),
                http::HttpRequestBytes,
            )
        })
        .and_then(|package| package.node(&http::http_stream_definition(), http::HttpStream))
        .and_then(|package| {
            package.source(&http::http_stream_line_definition(), http::HttpStreamLine)
        })
        .and_then(|package| package.node(&ws::ws_send_definition(), ws::WsSend { binary: false }))
        .and_then(|package| {
            package.node(&ws::ws_send_bytes_definition(), ws::WsSend { binary: true })
        })
        .and_then(|package| {
            package.source(
                &ws::ws_message_definition(),
                ws::WsMessage { binary: false },
            )
        })
        .and_then(|package| {
            package.source(&ws::ws_binary_definition(), ws::WsMessage { binary: true })
        })
        .and_then(|package| {
            package.node(
                &mqtt::publish_definition(),
                mqtt::MqttPublish { binary: false },
            )
        })
        .and_then(|package| {
            package.node(
                &mqtt::publish_bytes_definition(),
                mqtt::MqttPublish { binary: true },
            )
        })
        .and_then(|package| {
            package.source(
                &mqtt::message_definition(),
                mqtt::MqttMessage { binary: false },
            )
        })
        .and_then(|package| {
            package.source(
                &mqtt::binary_definition(),
                mqtt::MqttMessage { binary: true },
            )
        });
    match package {
        Ok(package) => package,
        // The definition is a constant of this crate, checked by its tests.
        Err(error) => unreachable!("built-in definition is valid: {error}"),
    }
}
