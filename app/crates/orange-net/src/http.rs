//! The HTTP connection and the request nodes that use it.
use crate::{
    DEFAULT_STREAM_TIMEOUT_MS, DEFAULT_TIMEOUT_MS, MAX_HEADERS, MAX_RESPONSE_BYTES,
    MAX_STREAM_LINE_BYTES, auth_headers, fault, oauth,
};
use orange_sdk::{
    Call, ConnectionAdapter, Definition, EmitError, Emitter, KernelFault, Listeners, Renew,
    ServiceNode, ServiceSource, Session, Value,
};
use serde_json::{Map, Value as Json};
use std::any::Any;
use std::io::Read;
use std::sync::Arc;
use std::time::Duration;
use ureq::Agent;
use ureq::http::{Method, Request};

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
pub(crate) struct HttpAdapter;
impl ConnectionAdapter for HttpAdapter {
    fn open(&self, settings: &Map<String, Json>) -> Result<Arc<dyn Session>, String> {
        self.open_renewing(settings, Renew::none())
    }
    fn open_renewing(
        &self,
        settings: &Map<String, Json>,
        renew: Renew,
    ) -> Result<Arc<dyn Session>, String> {
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
            Some(setting) => Some(oauth::OAuth::from_setting(setting, renew)?),
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
pub(crate) struct HttpRequest;
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
pub(crate) struct HttpRequestBytes;
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
pub(crate) struct HttpStream;
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
pub(crate) struct HttpStreamLine;
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

pub(crate) fn http_stream_definition() -> String {
    Definition::node(crate::PACKAGE, "http-stream", "HTTP request (streamed)")
        .description("Sends a request and delivers the response line by line as it arrives.")
        .category("network", "Network")
        .connection()
        .input("method", "core/string@1")
        .input("path", "core/string@1")
        .input("content_type", "core/string@1")
        .input("headers", "core/list@1<core/string@1>")
        .input("content", "core/bytes@1")
        .output("status", "core/i64@1")
        .output("lines", "core/i64@1")
        .build()
}

pub(crate) fn http_stream_line_definition() -> String {
    Definition::entry(crate::PACKAGE, "http-stream-line", "On HTTP stream line")
        .category("network", "Network")
        .connection()
        .payload("core/string@1")
        .build()
}

pub(crate) fn http_request_bytes_definition() -> String {
    Definition::node(
        crate::PACKAGE,
        "http-request-bytes",
        "HTTP request (binary)",
    )
    .description("Sends a request with a binary body and returns the response as bytes.")
    .category("network", "Network")
    .connection()
    .input("method", "core/string@1")
    .input("path", "core/string@1")
    .input("content_type", "core/string@1")
    .input("headers", "core/list@1<core/string@1>")
    .input("content", "core/bytes@1")
    .output("status", "core/i64@1")
    .output("response_type", "core/string@1")
    .output("response", "core/bytes@1")
    .build()
}

pub(crate) fn http_request_definition() -> String {
    Definition::node(crate::PACKAGE, "http-request", "HTTP request")
        .category("network", "Network")
        .connection()
        .input("method", "core/string@1")
        .input("path", "core/string@1")
        .input("content", "core/string@1")
        .output("status", "core/i64@1")
        .output("response", "core/string@1")
        .build()
}
