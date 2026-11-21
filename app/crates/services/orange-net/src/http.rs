//! The HTTP connection and the request nodes that use it.
use crate::{
    DEFAULT_STREAM_TIMEOUT_MS, DEFAULT_TIMEOUT_MS, MAX_HEADERS, MAX_RESPONSE_BYTES,
    MAX_STREAM_LINE_BYTES, auth_headers, fault, oauth,
};
use orange_sdk::{
    Call, ConnectionAdapter, Definition, EmitError, Emitter, JsonText, KernelFault, ListRepr,
    Listeners, MapRepr, Renew, ServiceNode, ServiceSource, Session, Value,
};
use serde_json::{Map, Value as Json};
use std::any::Any;
use std::collections::BTreeMap;
use std::io::Read;
use std::sync::Arc;
use std::time::Duration;
use ureq::Agent;
use ureq::http::{HeaderMap, HeaderName, HeaderValue, Method, Request};

const STRING_MAP: &str = "core/map@1<core/string@1,core/string@1>";
const RESPONSE_HEADERS: &str = "core/map@1<core/string@1,core/list@1<core/string@1>>";
const MAX_QUERY_PAIRS: usize = 64;
const MAX_REQUEST_TARGET_BYTES: usize = 16 * 1024;
const MAX_REQUEST_HEADER_BYTES: usize = 32 * 1024;

const NO_SESSION: KernelFault = fault(
    "org.orange.net/Connection",
    "org.orange.net/connection-kind",
    "The node needs an HTTP connection.",
);
const BAD_REQUEST: KernelFault = fault(
    "org.orange.net/Request",
    "org.orange.net/request-invalid",
    "Check the HTTP method, path, query parameters, and request headers.",
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
    /// Sends one request with the connection's headers and the validated extras.
    /// With OAuth the request carries the current access token;
    /// if the server answers 401 the token is replaced and the request sent once more.
    fn send(
        &self,
        method: &str,
        path: &str,
        content_type: &str,
        extra: &[(HeaderName, HeaderValue)],
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
        extra: &[(HeaderName, HeaderValue)],
        body: Vec<u8>,
        streamed: bool,
        token: Option<&str>,
    ) -> Result<ureq::http::Response<ureq::Body>, KernelFault> {
        let request = self.build_request(method, path, content_type, extra, body, token)?;
        let agent = if streamed {
            &self.stream_agent
        } else {
            &self.agent
        };
        agent.run(request).map_err(|_| TRANSPORT)
    }

    fn build_request(
        &self,
        method: &str,
        path: &str,
        content_type: &str,
        extra: &[(HeaderName, HeaderValue)],
        body: Vec<u8>,
        token: Option<&str>,
    ) -> Result<Request<Vec<u8>>, KernelFault> {
        let header_bytes = extra
            .iter()
            .map(|(name, value)| name.as_str().len() + value.len())
            .sum::<usize>();
        if header_bytes.saturating_add(content_type.len()) > MAX_REQUEST_HEADER_BYTES
            || content_type.chars().any(char::is_control)
        {
            return Err(BAD_REQUEST);
        }
        let method = Method::from_bytes(method.as_bytes()).map_err(|_| BAD_REQUEST)?;
        if (!path.is_empty() && !matches!(path.as_bytes()[0], b'/' | b'?'))
            || path.contains('#')
            || path.len() > MAX_REQUEST_TARGET_BYTES
        {
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
        for (name, value) in extra {
            if (!content_type.is_empty() && *name == ureq::http::header::CONTENT_TYPE)
                || self
                    .headers
                    .iter()
                    .any(|(part, _)| part.eq_ignore_ascii_case(name.as_str()))
            {
                return Err(BAD_REQUEST);
            }
            request = request.header(name, value.clone());
        }
        request.body(body).map_err(|_| BAD_REQUEST)
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
    fn about(&self) -> orange_sdk::AdapterAbout {
        orange_sdk::AdapterAbout {
            label: "HTTP / HTTPS".into(),
            purpose: "Sends requests to a web address, such as a REST API.".into(),
        }
    }
    fn texts(&self) -> Vec<orange_sdk::SettingText> {
        let mut texts = vec![orange_sdk::SettingText {
            required: true,
            ..crate::text_setting(
                "base_url",
                "Base URL",
                "Where requests go; each request's path is added to it.",
                "https://api.example.com",
            )
        }];
        texts.extend(crate::auth_texts());
        texts.extend([
            crate::text_setting(
                "oauth/token_url",
                "OAuth token address",
                "Where OAuth access tokens are fetched from.",
                "https://auth.example.com/token",
            ),
            crate::text_setting("oauth/client_id", "OAuth client ID", "The client's ID.", ""),
            crate::secret_setting(
                "oauth/client_secret",
                "OAuth client secret",
                "The client's secret, for the client-credentials grant.",
            ),
            crate::secret_setting(
                "oauth/refresh_token",
                "OAuth refresh token",
                "A refresh token, for the refresh grant.",
            ),
            crate::text_setting(
                "oauth/scope",
                "OAuth scope",
                "The scopes asked for, separated by spaces.",
                "read write",
            ),
        ]);
        texts
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
        crate::connection_uri(base_url, ["http", "https"])
            .filter(|uri| uri.query().is_none())
            .ok_or("`base_url` must be an absolute http:// or https:// address without user info, query or fragment")?;
        let timeout = match settings.get("timeout_ms") {
            None => DEFAULT_TIMEOUT_MS,
            Some(value) => value
                .as_u64()
                .filter(|ms| *ms > 0)
                .ok_or("`timeout_ms` must be a positive integer")?,
        };
        let headers = auth_headers(settings)?;
        // Request and profile headers can carry credentials under any application name.
        let configure = || {
            Agent::config_builder()
                .http_status_as_error(false)
                .max_redirects(0)
        };
        // Opening only builds the client; nothing is sent until a node runs.
        let agent = configure()
            .timeout_global(Some(Duration::from_millis(timeout)))
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
        let stream_agent = configure()
            .timeout_connect(Some(Duration::from_millis(timeout)))
            .timeout_recv_response(Some(Duration::from_millis(timeout)))
            .timeout_recv_body(Some(Duration::from_millis(stream_timeout)))
            .build()
            .new_agent();
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
    fn fields(&self) -> Vec<orange_sdk::SettingField> {
        #[expect(clippy::cast_precision_loss, reason = "minutes in milliseconds")]
        let field =
            |setting: &str, label: &str, help: &str, default: u64| orange_sdk::SettingField {
                setting: setting.into(),
                label: label.into(),
                help: help.into(),
                unit: "ms".into(),
                min: 1.0,
                max: 3_600_000.0,
                default: Some(default as f64),
                whole: true,
            };
        vec![
            field(
                "timeout_ms",
                "Timeout",
                "How long a whole request may take.",
                DEFAULT_TIMEOUT_MS,
            ),
            field(
                "stream_timeout_ms",
                "Stream timeout",
                "How long one streamed response may last.",
                DEFAULT_STREAM_TIMEOUT_MS,
            ),
        ]
    }
}

#[derive(Debug)]
pub(crate) struct HttpRequest;
impl ServiceNode for HttpRequest {
    fn run(&self, call: &Call<'_>) -> Result<Vec<Value>, KernelFault> {
        let session = http_session(call)?;
        let [
            Value::Str(method),
            Value::Str(path),
            Value::Str(body),
            Value::Str(content_type),
            Value::Str(headers),
            Value::Map(header_fields),
            Value::Map(query),
        ] = call.inputs
        else {
            return Err(BAD_REQUEST);
        };
        let mut headers = parse_header_lines(headers.lines())?;
        append_header_fields(&mut headers, header_fields)?;
        let response = session.send(
            method,
            &request_path(path, query)?,
            content_type,
            &headers,
            body.as_bytes().to_vec(),
            false,
        )?;
        text_response(response, false)
    }
}

#[derive(Debug)]
pub(crate) struct HttpRequestJson;
impl ServiceNode for HttpRequestJson {
    fn run(&self, call: &Call<'_>) -> Result<Vec<Value>, KernelFault> {
        let session = http_session(call)?;
        let [
            Value::Str(method),
            Value::Str(path),
            Value::Json(body),
            Value::Map(fields),
            Value::Map(query),
        ] = call.inputs
        else {
            return Err(BAD_REQUEST);
        };
        let mut headers = Vec::new();
        append_header_fields(&mut headers, fields)?;
        let response = session.send(
            method,
            &request_path(path, query)?,
            "application/json",
            &headers,
            body.as_str().as_bytes().to_vec(),
            false,
        )?;
        text_response(response, true)
    }
}

fn text_response(
    mut response: ureq::http::Response<ureq::Body>,
    parse_json: bool,
) -> Result<Vec<Value>, KernelFault> {
    let status = response.status().as_u16();
    let text = response
        .body_mut()
        .with_config()
        .limit(MAX_RESPONSE_BYTES)
        .read_to_string()
        .map_err(|_| RESPONSE)?;
    let json = parse_json.then(|| {
        JsonText::try_from(text.as_str()).map_or(Value::None, |text| Value::Json(Arc::new(text)))
    });
    let mut outputs = vec![
        Value::I64(i64::from(status)),
        Value::Bool(success(status)),
        Value::Str(Arc::new(text)),
        response_headers(response.headers())?,
    ];
    outputs.extend(json);
    Ok(outputs)
}

fn append_header_fields(
    headers: &mut Vec<(HeaderName, HeaderValue)>,
    fields: &MapRepr,
) -> Result<(), KernelFault> {
    for (name, value) in fields.entries() {
        let (Value::Str(name), Value::Str(value)) = (name, value) else {
            return Err(BAD_REQUEST);
        };
        add_header(headers, name, value)?;
    }
    Ok(())
}

fn parse_header_lines<'a>(
    lines: impl IntoIterator<Item = &'a str>,
) -> Result<Vec<(HeaderName, HeaderValue)>, KernelFault> {
    let mut headers = Vec::new();
    for line in lines.into_iter().filter(|line| !line.trim().is_empty()) {
        let (name, value) = line.split_once(':').ok_or(BAD_REQUEST)?;
        add_header(&mut headers, name.trim(), value.trim())?;
    }
    Ok(headers)
}

fn add_header(
    headers: &mut Vec<(HeaderName, HeaderValue)>,
    name: &str,
    value: &str,
) -> Result<(), KernelFault> {
    let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| BAD_REQUEST)?;
    if headers.len() >= MAX_HEADERS
        || matches!(
            name.as_str(),
            "authorization"
                | "proxy-authorization"
                | "host"
                | "content-length"
                | "transfer-encoding"
                | "connection"
                | "upgrade"
                | "trailer"
        )
        || headers.iter().any(|(existing, _)| *existing == name)
    {
        return Err(BAD_REQUEST);
    }
    let value = HeaderValue::from_str(value).map_err(|_| BAD_REQUEST)?;
    headers.push((name, value));
    Ok(())
}

fn request_path(path: &str, query: &MapRepr) -> Result<String, KernelFault> {
    if query.entries().len() > MAX_QUERY_PAIRS || path.len() > MAX_REQUEST_TARGET_BYTES {
        return Err(BAD_REQUEST);
    }
    let mut target = path.to_owned();
    let mut separator = if path.contains('?') { '&' } else { '?' };
    for (key, value) in query.entries() {
        let (Value::Str(key), Value::Str(value)) = (key, value) else {
            return Err(BAD_REQUEST);
        };
        target.push(separator);
        separator = '&';
        encode_query(key, &mut target)?;
        target.push('=');
        encode_query(value, &mut target)?;
    }
    Ok(target)
}

fn encode_query(text: &str, target: &mut String) -> Result<(), KernelFault> {
    const HEX: &[u8] = b"0123456789ABCDEF";
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            target.push(char::from(byte));
        } else {
            target.push('%');
            target.push(char::from(HEX[usize::from(byte >> 4)]));
            target.push(char::from(HEX[usize::from(byte & 15)]));
        }
        if target.len() > MAX_REQUEST_TARGET_BYTES {
            return Err(BAD_REQUEST);
        }
    }
    Ok(())
}

fn response_headers(headers: &HeaderMap) -> Result<Value, KernelFault> {
    let mut grouped = BTreeMap::<&str, Vec<Arc<String>>>::new();
    for (name, value) in headers {
        grouped.entry(name.as_str()).or_default().push(Arc::new(
            String::from_utf8_lossy(value.as_bytes()).into_owned(),
        ));
    }
    let entries: Vec<_> = grouped
        .into_iter()
        .map(|(name, values)| {
            (
                Value::Str(Arc::new(name.to_owned())),
                Value::List(Arc::new(ListRepr::Str(values.into()))),
            )
        })
        .collect();
    MapRepr::new(entries)
        .map(|map| Value::Map(Arc::new(map)))
        .map_err(|_| RESPONSE)
}

/// The HTTP session a request node is bound to.
fn http_session<'c>(call: &Call<'c>) -> Result<&'c HttpSession, KernelFault> {
    call.session
        .and_then(|session| session.as_any().downcast_ref::<HttpSession>())
        .ok_or(NO_SESSION)
}

/// Sends the request a binary or streamed request node describes: method, path,
/// content type, header lines and a byte body.
fn send_binary(
    call: &Call<'_>,
    streamed: bool,
) -> Result<ureq::http::Response<ureq::Body>, KernelFault> {
    let session = http_session(call)?;
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
    let ListRepr::Str(headers) = headers.as_ref() else {
        return Err(BAD_REQUEST);
    };
    let headers = parse_header_lines(headers.iter().map(|line| line.as_str()))?;
    session.send(
        method,
        path,
        content_type,
        &headers,
        body.as_ref().clone(),
        streamed,
    )
}

#[derive(Debug)]
pub(crate) struct HttpRequestBytes;
impl ServiceNode for HttpRequestBytes {
    fn run(&self, call: &Call<'_>) -> Result<Vec<Value>, KernelFault> {
        let mut response = send_binary(call, false)?;
        let status = response.status().as_u16();
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
            Value::I64(i64::from(status)),
            Value::Bool(success(status)),
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
        let session = http_session(call)?;
        let mut response = send_binary(call, true)?;
        let code = response.status().as_u16();
        let (status, ok) = (Value::I64(i64::from(code)), Value::Bool(success(code)));
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
                        return Ok(vec![status, ok, Value::I64(lines)]);
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
        }
        Ok(vec![status, ok, Value::I64(lines)])
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

/// Whether `status` says the server did what was asked: 200 to 299. Any other status,
/// such as 404 or 500, is still an answer and comes back as data; only a request that
/// could not be completed at all fails the node.
pub(crate) const fn success(status: u16) -> bool {
    matches!(status, 200..=299)
}

/// A text input left empty: no body, no content type.
fn no_text() -> Json {
    serde_json::json!({"type": "core/string@1", "value": ""})
}

/// A request sent with nothing more than its method and path: no content type, no
/// extra headers and no body.
fn bare(definition: Definition) -> Definition {
    definition
        .input_default(
            "method",
            "core/string@1",
            serde_json::json!({"type": "core/string@1", "value": "GET"}),
        )
        .input_default("path", "core/string@1", no_text())
        .input_default("content_type", "core/string@1", no_text())
        .input_default(
            "headers",
            "core/list@1<core/string@1>",
            serde_json::json!({"type": "core/list@1<core/string@1>", "value": []}),
        )
        .input_default(
            "content",
            "core/bytes@1",
            serde_json::json!({"type": "core/bytes@1", "value": ""}),
        )
}

pub(crate) fn http_stream_definition() -> String {
    bare(
        Definition::node(crate::PACKAGE, "http-stream", "HTTP request (streamed)")
            .description("Sends a request and delivers the response line by line as it arrives. Any status is a result; `success` is true for 200 to 299, and the node fails only when the request cannot be completed.")
            .category("network", "Network")
            .keywords("http, https, streaming response, sse")
            .uses(crate::HTTP_ADAPTER),
    )
    .output("status", "core/i64@1")
    .output("success", "core/bool@1")
    .output("lines", "core/i64@1")
    .build()
}

pub(crate) fn http_stream_line_definition() -> String {
    Definition::entry(crate::PACKAGE, "http-stream-line", "On HTTP stream line")
        .category("network", "Network")
        .keywords("http, https, streaming response, sse")
        .uses(crate::HTTP_ADAPTER)
        .payload("core/string@1")
        .build()
}

pub(crate) fn http_request_bytes_definition() -> String {
    bare(
        Definition::node(
            crate::PACKAGE,
            "http-request-bytes",
            "HTTP request (binary)",
        )
        .description("Sends a request with a binary body and returns the response as bytes. Any status is a result; `success` is true for 200 to 299, and the node fails only when the request cannot be completed.")
        .category("network", "Network")
        .keywords("http, https, binary, bytes")
        .uses(crate::HTTP_ADAPTER),
    )
    .output("status", "core/i64@1")
    .output("success", "core/bool@1")
    .output("response_type", "core/string@1")
    .output("response", "core/bytes@1")
    .build()
}

pub(crate) fn http_request_definition() -> String {
    Definition::node(crate::PACKAGE, "http-request", "HTTP request")
        .version(2)
        .replaces("http-request", 1, &[])
        .description("Sends a request over HTTP or HTTPS and returns the response as text. Choose a connection, enter a path starting with /, and optionally set a content type, body and headers (one Name: value per line). Each HTTP status is a result. The failed route handles requests that could not be completed.")
        .category("network", "Network")
        .keywords("HTTP, HTTPS, REST, API, GET, POST, JSON, headers")
        .uses(crate::HTTP_ADAPTER)
        .input_default("method", "core/string@1", serde_json::json!({"type": "core/string@1", "value": "GET"}))
        .input_default("path", "core/string@1", no_text())
        .input_default("content", "core/string@1", no_text())
        .input_default("content_type", "core/string@1", no_text())
        .input_default("headers", "core/string@1", no_text())
        .input_default("header_fields", STRING_MAP, empty_map())
        .input_default("query", STRING_MAP, empty_map())
        .output("status", "core/i64@1")
        .output("success", "core/bool@1")
        .output("response", "core/string@1")
        .output("response_headers", RESPONSE_HEADERS)
        .build()
}

pub(crate) fn http_request_json_definition() -> String {
    Definition::node(crate::PACKAGE, "http-request-json", "HTTP JSON Request")
        .description("Sends a JSON body and returns the HTTP status, response headers, response text, and an optional JSON result.")
        .category("network", "Network")
        .keywords("http, https, rest, api, post, json, payload")
        .uses(crate::HTTP_ADAPTER)
        .input_default("method", "core/string@1", serde_json::json!({"type": "core/string@1", "value": "POST"}))
        .input_default("path", "core/string@1", no_text())
        .input_default("content", "core/json@1", serde_json::json!({"type": "core/json@1", "value": "null"}))
        .input_default("header_fields", STRING_MAP, empty_map())
        .input_default("query", STRING_MAP, empty_map())
        .output("status", "core/i64@1")
        .output("success", "core/bool@1")
        .output("response", "core/string@1")
        .output("response_headers", RESPONSE_HEADERS)
        .output("json", "core/option@1<core/json@1>")
        .build()
}

fn empty_map() -> Json {
    serde_json::json!({"type": STRING_MAP, "value": []})
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use ureq::unversioned::resolver::DefaultResolver;
    use ureq::unversioned::transport::{
        Buffers, ConnectionDetails, Connector, LazyBuffers, NextTimeout, Transport,
    };

    #[derive(Debug)]
    struct MockApi(Arc<Mutex<Vec<u8>>>);

    impl MockApi {
        fn session(&self, base_url: &str, headers: Vec<(String, String)>) -> HttpSession {
            let agent = Agent::with_parts(
                Agent::config_builder()
                    .proxy(None)
                    .http_status_as_error(false)
                    .build(),
                Self(Arc::clone(&self.0)),
                DefaultResolver::default(),
            );
            HttpSession {
                stream_agent: agent.clone(),
                agent,
                base_url: base_url.to_owned(),
                headers,
                oauth: None,
                listeners: Arc::new(Listeners::new()),
            }
        }
    }

    impl ConnectionAdapter for MockApi {
        fn open(&self, settings: &Map<String, Json>) -> Result<Arc<dyn Session>, String> {
            let address = settings
                .get("base_url")
                .and_then(Json::as_str)
                .ok_or("base URL required")?;
            Ok(Arc::new(self.session(address, auth_headers(settings)?)))
        }
    }

    impl Connector for MockApi {
        type Out = MockHttp;

        fn connect(
            &self,
            details: &ConnectionDetails<'_>,
            _: Option<()>,
        ) -> Result<Option<MockHttp>, ureq::Error> {
            Ok(Some(MockHttp {
                buffers: LazyBuffers::new(details.config.input_buffer_size(), details.config.output_buffer_size()),
                request: Arc::clone(&self.0),
                reply: &b"HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"ok\":true}"[..],
            }))
        }
    }

    #[derive(Debug)]
    struct MockHttp {
        buffers: LazyBuffers,
        request: Arc<Mutex<Vec<u8>>>,
        reply: &'static [u8],
    }

    impl Transport for MockHttp {
        fn buffers(&mut self) -> &mut dyn Buffers {
            &mut self.buffers
        }

        fn transmit_output(&mut self, amount: usize, _: NextTimeout) -> Result<(), ureq::Error> {
            self.request
                .lock()
                .unwrap()
                .extend_from_slice(&self.buffers.output()[..amount]);
            Ok(())
        }

        fn await_input(&mut self, _: NextTimeout) -> Result<bool, ureq::Error> {
            let count = self.reply.read(self.buffers.input_append_buf())?;
            self.buffers.input_appended(count);
            Ok(count != 0)
        }

        fn is_open(&mut self) -> bool {
            !self.reply.is_empty()
        }
    }

    #[test]
    fn the_mock_api_receives_the_method_path_authentication_headers_and_json_body() {
        let request = Arc::new(Mutex::new(Vec::new()));
        let settings = serde_json::json!({
            "api_key": {"header": "X-Integration-Key", "token": "mock-key"},
            "headers": {"X-Client": "Orange"}
        });
        let session = MockApi(Arc::clone(&request)).session(
            "http://127.0.0.1/v1",
            auth_headers(settings.as_object().unwrap()).unwrap(),
        );
        let json_body = b"{\n  \"message\": \"hello\"\n}";
        let mut response = session
            .send(
                "POST",
                "/anything?test=1",
                "application/json",
                &parse_header_lines(["Accept: application/json", "X-Trace: sample"]).unwrap(),
                json_body.to_vec(),
                false,
            )
            .unwrap();
        assert_eq!(response.status(), 201);
        assert_eq!(
            response.body_mut().read_to_string().unwrap(),
            r#"{"ok":true}"#
        );

        let request = request.lock().unwrap();
        let (head, body) = std::str::from_utf8(&request)
            .unwrap()
            .split_once("\r\n\r\n")
            .unwrap();
        assert!(head.starts_with("POST /v1/anything?test=1 HTTP/1.1\r\n"));
        let head = head.to_ascii_lowercase();
        for header in [
            "content-type: application/json",
            "accept: application/json",
            "x-trace: sample",
            "x-integration-key: mock-key",
            "x-client: orange",
        ] {
            assert!(head.contains(header), "{head}");
        }
        assert_eq!(body.as_bytes(), json_body);
    }

    #[test]
    fn an_http_graph_observes_the_status_and_body_without_output_wires() {
        use orange_host::{FlowEvent, Host, HostOptions, ServicePackage};

        let package = ServicePackage::new(crate::PACKAGE, env!("CARGO_PKG_VERSION"))
            .adapter(crate::HTTP_ADAPTER, MockApi(Arc::default()))
            .node(&http_request_definition(), HttpRequest)
            .unwrap();
        let (mut host, events) =
            Host::with_services(HostOptions::default(), vec![package]).unwrap();
        let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
        let trigger = host
            .add_node(&mut draft, "core/on-manual", [0.0, 0.0])
            .unwrap();
        let request = host
            .add_node(&mut draft, crate::HTTP_REQUEST, [260.0, 0.0])
            .unwrap();
        let mut connection = host.declarations(&draft).connections.remove(0);
        connection.settings = serde_json::json!({"base_url": "http://127.0.0.1"});
        host.declare_connection(&mut draft, &connection).unwrap();
        host.connect(&mut draft, (trigger, "then"), (request, "in"))
            .unwrap();

        let flow = host.open(&draft.text().unwrap()).unwrap();
        host.observe(flow, true).unwrap();
        host.trigger(flow, trigger, None).unwrap();
        let mut results = BTreeMap::new();
        orange_testkit::until("the unwired response", || {
            while let Ok(event) = events.try_recv() {
                if event.flow == flow
                    && let FlowEvent::Ports(ports) = event.kind
                {
                    results.extend(ports.into_iter().filter_map(|(node, port, value)| {
                        (node == request).then(|| (port.to_string(), value))
                    }));
                }
            }
            results.len() == 4
        });
        assert_eq!(results["status"], Value::I64(201));
        assert_eq!(results["success"], Value::Bool(true));
        assert_eq!(
            results["response"],
            Value::Str(Arc::new(r#"{"ok":true}"#.into()))
        );
        host.close(flow).unwrap();
    }

    #[test]
    fn only_2xx_is_success_and_every_status_is_an_answer() {
        assert!(success(200) && success(204) && success(299));
        assert!(!success(199) && !success(301) && !success(404) && !success(500));
    }

    #[test]
    fn request_headers_cannot_override_framing_or_profile_credentials() {
        assert!(parse_header_lines(["Host: other.example"]).is_err());
        assert!(parse_header_lines(["Transfer-Encoding: chunked"]).is_err());
        assert!(parse_header_lines(["Content-Length: 1"]).is_err());
        assert!(parse_header_lines(["X-Test: 1", "x-test: 2"]).is_err());
        assert!(add_header(&mut Vec::new(), "X-Test", "value\r\nOther: 1").is_err());

        let session = MockApi(Arc::default()).session(
            "http://127.0.0.1",
            vec![("X-Profile-Key".into(), "fixture".into())],
        );
        let headers = parse_header_lines(["x-profile-key: replacement"]).unwrap();
        assert!(
            session
                .build_request("GET", "/", "", &headers, Vec::new(), None)
                .is_err()
        );
    }
}
