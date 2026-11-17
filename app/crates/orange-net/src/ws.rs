//! WebSocket connections: one socket per session, shared by every node that names it.
//!
//! A session owns one I/O thread. It connects once a flow using it has attached its
//! triggers, reconnects with backoff, writes queued
//! outgoing messages and fans each received text message out to the entry nodes that
//! listen. Nodes never touch the socket: sending queues a message and returns, and a
//! listener is fired through its emitter, so a slow or paused flow cannot stall the
//! socket and the socket cannot stall a flow.
use orange_sdk::{
    Call, ConnectionAdapter, Definition, Emitter, KernelFault, Lifecycle, Listeners, ServiceNode,
    ServiceSource, Session, Value,
};
use serde_json::{Map, Value as Json};
use std::any::Any;
use std::collections::VecDeque;
use std::io::ErrorKind;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Error, Message, WebSocket};

use crate::fault;

/// Messages a session holds while its socket is busy or down. Sending beyond it fails.
pub const MAX_QUEUED_MESSAGES: usize = 256;
/// How long the I/O thread waits for incoming data before it looks at the outgoing
/// queue again; the longest a queued message waits on an idle socket.
const POLL: Duration = Duration::from_millis(10);
const FIRST_RETRY: Duration = Duration::from_millis(250);
const MAX_RETRY: Duration = Duration::from_secs(5);

const NO_SESSION: KernelFault = fault(
    "org.orange.net/Connection",
    "org.orange.net/connection-kind",
    "The node needs a WebSocket connection.",
);
const BAD_MESSAGE: KernelFault = fault(
    "org.orange.net/Request",
    "org.orange.net/message-invalid",
    "The message must be text for a text send and bytes for a binary send.",
);
const QUEUE_FULL: KernelFault = fault(
    "org.orange.net/Busy",
    "org.orange.net/send-queue-full",
    "The connection has too many messages waiting to be sent.",
);

/// What a listening entry fires for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Listen {
    Text,
    Binary,
    /// The socket has opened.
    Connected,
    /// An open socket has closed or failed.
    Disconnected,
}

/// One message waiting to be sent.
#[derive(Debug, Clone)]
enum Outgoing {
    Text(String),
    Binary(Vec<u8>),
}

struct Shared {
    url: String,
    /// Headers of the opening handshake; they can be credentials.
    headers: Vec<(String, String)>,
    outgoing: Mutex<VecDeque<Outgoing>>,
    /// Each listener with what it fires for.
    listeners: Listeners<Listen>,
    /// Whether the socket is open now.
    connected: AtomicBool,
    /// Started once a flow has attached its triggers; the socket is not opened before.
    life: Lifecycle,
}
impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shared")
            .field("url", &self.url)
            .field("headers", &self.headers.len())
            .finish_non_exhaustive()
    }
}
impl Shared {
    fn stopped(&self) -> bool {
        self.life.stopped()
    }
    /// Fires every entry listening for `what`. A flow that has ended is forgotten; one
    /// whose mailbox is full misses this firing, as its bounded mailbox requires.
    fn deliver(&self, what: Listen, payload: &Value) {
        self.listeners
            .emit(|listens| (*listens == what).then(|| payload.clone()));
    }
}

/// A WebSocket connection. Dropping the last reference closes the socket.
#[derive(Debug)]
struct WsSession(Arc<Shared>);
impl Session for WsSession {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn start(&self) {
        self.0.life.start();
    }
}
impl Drop for WsSession {
    fn drop(&mut self) {
        self.0.life.stop();
    }
}

fn set_poll(socket: &WebSocket<MaybeTlsStream<TcpStream>>) {
    let stream = match socket.get_ref() {
        MaybeTlsStream::Plain(stream) => stream,
        MaybeTlsStream::Rustls(stream) => &stream.sock,
        _ => return,
    };
    let _ = stream.set_read_timeout(Some(POLL));
}

/// Why a served socket stopped, as the disconnected entry's payload says it.
const CLOSED: &str = "closed by the server";
const FAILED: &str = "connection lost";

/// Serves one established socket until it fails, closes or the session ends, and says
/// why it stopped.
fn serve(shared: &Shared, socket: &mut WebSocket<MaybeTlsStream<TcpStream>>) -> &'static str {
    set_poll(socket);
    loop {
        if shared.stopped() {
            let _ = socket.close(None);
            let _ = socket.flush();
            return "connection closed";
        }
        loop {
            let next = shared
                .outgoing
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop_front();
            let Some(text) = next else { break };
            let message = match text.clone() {
                Outgoing::Text(text) => Message::text(text),
                Outgoing::Binary(data) => Message::binary(data),
            };
            if let Err(error) = socket.send(message) {
                // Not written: keep it for the next socket, in order.
                if !matches!(error, Error::Io(ref io) if io.kind() == ErrorKind::WouldBlock) {
                    shared
                        .outgoing
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .push_front(text);
                    return FAILED;
                }
            }
        }
        match socket.read() {
            Ok(Message::Text(text)) => {
                shared.deliver(
                    Listen::Text,
                    &Value::Str(Arc::new(text.as_str().to_owned())),
                );
            }
            Ok(Message::Binary(data)) => {
                shared.deliver(Listen::Binary, &Value::Bytes(Arc::new(data.to_vec())));
            }
            Ok(Message::Close(_)) => return CLOSED,
            Ok(_) => {}
            Err(Error::Io(error))
                if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(_) => return FAILED,
        }
    }
}

/// The opening request with the connection's headers. `None` when a header cannot be
/// sent, which the adapter already refused.
fn handshake(shared: &Shared) -> Option<tungstenite::handshake::client::Request> {
    use tungstenite::client::IntoClientRequest;
    use tungstenite::http::{HeaderName, HeaderValue};
    let mut request = shared.url.as_str().into_client_request().ok()?;
    for (name, value) in &shared.headers {
        let name = HeaderName::from_bytes(name.as_bytes()).ok()?;
        let mut value = HeaderValue::from_str(value).ok()?;
        value.set_sensitive(true);
        let _ = request.headers_mut().insert(name, value);
    }
    Some(request)
}

fn run(shared: &Shared) {
    // A server may speak first. Connecting only once a flow has attached its triggers
    // means its first messages have somewhere to go.
    if !shared.life.wait_started() {
        return;
    }
    let mut retry = FIRST_RETRY;
    while !shared.stopped() {
        if let Some(request) = handshake(shared)
            && let Ok((mut socket, _)) = tungstenite::connect(request)
        {
            retry = FIRST_RETRY;
            shared.connected.store(true, Ordering::Release);
            shared.deliver(Listen::Connected, &Value::Str(Arc::new(shared.url.clone())));
            let why = serve(shared, &mut socket);
            shared.connected.store(false, Ordering::Release);
            if !shared.stopped() {
                shared.deliver(Listen::Disconnected, &Value::Str(Arc::new(why.to_owned())));
            }
        }
        // Wait before reconnecting, but notice the session ending promptly.
        let _ = shared.life.rest(retry);
        retry = (retry * 2).min(MAX_RETRY);
    }
}

#[derive(Debug)]
pub(crate) struct WsAdapter;
impl ConnectionAdapter for WsAdapter {
    fn about(&self) -> orange_sdk::AdapterAbout {
        orange_sdk::AdapterAbout {
            label: "WebSocket".into(),
            purpose: "Keeps a socket open to a server to send and receive messages.".into(),
        }
    }
    fn texts(&self) -> Vec<orange_sdk::SettingText> {
        let mut texts = vec![orange_sdk::SettingText {
            required: true,
            ..crate::text_setting(
                "url",
                "Address",
                "The socket's address.",
                "wss://example.com/socket",
            )
        }];
        texts.extend(crate::auth_texts());
        texts
    }
    fn open(&self, settings: &Map<String, Json>) -> Result<Arc<dyn Session>, String> {
        let url = settings
            .get("url")
            .and_then(Json::as_str)
            .ok_or("`url` is required")?;
        if !url.starts_with("ws://") && !url.starts_with("wss://") {
            return Err("`url` must start with ws:// or wss://".into());
        }
        let shared = Arc::new(Shared {
            url: url.to_owned(),
            headers: crate::auth_headers(settings)?,
            outgoing: Mutex::new(VecDeque::new()),
            listeners: Listeners::new(),
            connected: AtomicBool::new(false),
            life: Lifecycle::new(),
        });
        // Connecting happens on the session's own thread, never here.
        let io = Arc::clone(&shared);
        thread::Builder::new()
            .name("orange-net-ws".into())
            .spawn(move || run(&io))
            .map_err(|error| error.to_string())?;
        Ok(Arc::new(WsSession(shared)))
    }
}

fn session(session: Option<&dyn Session>) -> Option<&Arc<Shared>> {
    session?
        .as_any()
        .downcast_ref::<WsSession>()
        .map(|session| &session.0)
}

/// Queues one text or binary message. `then` means queued, not delivered: the socket
/// may be reconnecting, and a WebSocket gives no delivery receipt either way.
#[derive(Debug)]
pub(crate) struct WsSend {
    pub binary: bool,
}
impl ServiceNode for WsSend {
    fn run(&self, call: &Call<'_>) -> Result<Vec<Value>, KernelFault> {
        let shared = session(call.session).ok_or(NO_SESSION)?;
        let message = match (self.binary, call.inputs) {
            (false, [Value::Str(text)]) => Outgoing::Text(text.as_str().to_owned()),
            (true, [Value::Bytes(data)]) => Outgoing::Binary(data.as_ref().clone()),
            _ => return Err(BAD_MESSAGE),
        };
        let mut outgoing = shared
            .outgoing
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if outgoing.len() >= MAX_QUEUED_MESSAGES {
            return Err(QUEUE_FULL);
        }
        outgoing.push_back(message);
        Ok(Vec::new())
    }
}

/// What a connection is doing now: whether its socket is open and how many messages
/// wait to be sent. Reading it never waits for the socket.
#[derive(Debug)]
pub(crate) struct WsStatus;
impl ServiceNode for WsStatus {
    fn run(&self, call: &Call<'_>) -> Result<Vec<Value>, KernelFault> {
        let shared = session(call.session).ok_or(NO_SESSION)?;
        let queued = shared
            .outgoing
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len();
        Ok(vec![
            Value::Bool(shared.connected.load(Ordering::Acquire)),
            Value::I64(i64::try_from(queued).unwrap_or(i64::MAX)),
        ])
    }
}

/// Fires its entry once per text message received on the connection, once per
/// binary message, or each time the socket opens or closes.
#[derive(Debug)]
pub(crate) struct WsMessage {
    pub listen: Listen,
}
impl ServiceSource for WsMessage {
    fn start(
        &self,
        session: Option<Arc<dyn Session>>,
        emitter: Emitter,
    ) -> Result<Box<dyn Send>, String> {
        let shared = crate::ws::session(session.as_deref())
            .ok_or("the entry needs a WebSocket connection")?;
        Ok(Listeners::listen(
            Arc::clone(shared),
            |shared| &shared.listeners,
            self.listen,
            emitter,
        ))
    }
}

pub(crate) fn ws_send_definition() -> String {
    Definition::node(crate::PACKAGE, "ws-send", "Send WebSocket message")
        .description("Puts a text message in the connection's send queue and continues: Queued means queued, not delivered. It fails when the queue is full.")
        .category("network", "Network")
        .uses(crate::WS_ADAPTER)
        .then_label("Queued")
        .input("text", "core/string@1")
        .build()
}

pub(crate) fn ws_status_definition() -> String {
    Definition::node(crate::PACKAGE, "ws-status", "WebSocket status")
        .description("Reads whether the connection's socket is open now and how many messages wait in its send queue.")
        .category("network", "Network")
        .uses(crate::WS_ADAPTER)
        .output("connected", "core/bool@1")
        .output("queued", "core/i64@1")
        .build()
}

pub(crate) fn ws_connected_definition() -> String {
    Definition::entry(crate::PACKAGE, "ws-connected", "On WebSocket connected")
        .description("Fires each time the connection's socket opens, with its URL.")
        .category("network", "Network")
        .uses(crate::WS_ADAPTER)
        .payload("core/string@1")
        .build()
}

pub(crate) fn ws_disconnected_definition() -> String {
    Definition::entry(crate::PACKAGE, "ws-disconnected", "On WebSocket disconnected")
        .description("Fires each time an open socket closes or fails, with why. The connection then reconnects by itself.")
        .category("network", "Network")
        .uses(crate::WS_ADAPTER)
        .payload("core/string@1")
        .build()
}

pub(crate) fn ws_message_definition() -> String {
    Definition::entry(crate::PACKAGE, "ws-message", "On WebSocket message")
        .category("network", "Network")
        .uses(crate::WS_ADAPTER)
        .payload("core/string@1")
        .build()
}

pub(crate) fn ws_send_bytes_definition() -> String {
    Definition::node(
        crate::PACKAGE,
        "ws-send-bytes",
        "Send WebSocket binary message",
    )
    .description("Puts a binary message in the connection's send queue and continues: Queued means queued, not delivered. It fails when the queue is full.")
    .category("network", "Network")
    .uses(crate::WS_ADAPTER)
    .then_label("Queued")
    .input("data", "core/bytes@1")
    .build()
}

pub(crate) fn ws_binary_definition() -> String {
    Definition::entry(crate::PACKAGE, "ws-binary", "On WebSocket binary message")
        .category("network", "Network")
        .uses(crate::WS_ADAPTER)
        .payload("core/bytes@1")
        .build()
}
