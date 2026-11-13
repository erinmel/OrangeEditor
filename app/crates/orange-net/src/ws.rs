//! WebSocket connections: one socket per session, shared by every node that names it.
//!
//! A session owns one I/O thread. It connects once a flow using it has attached its
//! triggers, reconnects with backoff, writes queued
//! outgoing messages and fans each received text message out to the entry nodes that
//! listen. Nodes never touch the socket: sending queues a message and returns, and a
//! listener is fired through its emitter, so a slow or paused flow cannot stall the
//! socket and the socket cannot stall a flow.
use orange_sdk::{
    Call, ConnectionAdapter, Definition, Emitter, KernelFault, Listeners, ServiceNode,
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
    /// Each listener with whether it takes binary messages instead of text ones.
    listeners: Listeners<bool>,
    /// Set once a flow has attached its triggers; the socket is not opened before.
    started: AtomicBool,
    stop: AtomicBool,
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
        self.stop.load(Ordering::Relaxed)
    }
    /// Fires every listening entry. A flow that has ended is forgotten; one whose
    /// mailbox is full misses this message, as its bounded mailbox requires.
    fn deliver(&self, binary: bool, payload: &Value) {
        self.listeners
            .emit(|takes_binary| (*takes_binary == binary).then(|| payload.clone()));
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
        self.0.started.store(true, Ordering::Release);
    }
}
impl Drop for WsSession {
    fn drop(&mut self) {
        self.0.stop.store(true, Ordering::Relaxed);
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

/// Serves one established socket until it fails, closes or the session ends.
fn serve(shared: &Shared, socket: &mut WebSocket<MaybeTlsStream<TcpStream>>) {
    set_poll(socket);
    loop {
        if shared.stopped() {
            let _ = socket.close(None);
            let _ = socket.flush();
            return;
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
                    return;
                }
            }
        }
        match socket.read() {
            Ok(Message::Text(text)) => {
                shared.deliver(false, &Value::Str(Arc::new(text.as_str().to_owned())));
            }
            Ok(Message::Binary(data)) => {
                shared.deliver(true, &Value::Bytes(Arc::new(data.to_vec())));
            }
            Ok(Message::Close(_)) => return,
            Ok(_) => {}
            Err(Error::Io(error))
                if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(_) => return,
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
    while !shared.started.load(Ordering::Acquire) && !shared.stopped() {
        thread::sleep(POLL);
    }
    let mut retry = FIRST_RETRY;
    while !shared.stopped() {
        if let Some(request) = handshake(shared)
            && let Ok((mut socket, _)) = tungstenite::connect(request)
        {
            retry = FIRST_RETRY;
            serve(shared, &mut socket);
        }
        // Wait before reconnecting, but notice the session ending promptly.
        let mut waited = Duration::ZERO;
        while waited < retry && !shared.stopped() {
            thread::sleep(POLL);
            waited += POLL;
        }
        retry = (retry * 2).min(MAX_RETRY);
    }
}

#[derive(Debug)]
pub(crate) struct WsAdapter;
impl ConnectionAdapter for WsAdapter {
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
            started: AtomicBool::new(false),
            stop: AtomicBool::new(false),
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

/// Fires its entry once per text message received on the connection, or once per
/// binary message.
#[derive(Debug)]
pub(crate) struct WsMessage {
    pub binary: bool,
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
            self.binary,
            emitter,
        ))
    }
}

pub(crate) fn ws_send_definition() -> String {
    Definition::node(crate::PACKAGE, "ws-send", "Send WebSocket message")
        .category("network", "Network")
        .connection()
        .input("text", "core/string@1")
        .build()
}

pub(crate) fn ws_message_definition() -> String {
    Definition::entry(crate::PACKAGE, "ws-message", "On WebSocket message")
        .category("network", "Network")
        .connection()
        .payload("core/string@1")
        .build()
}

pub(crate) fn ws_send_bytes_definition() -> String {
    Definition::node(
        crate::PACKAGE,
        "ws-send-bytes",
        "Send WebSocket binary message",
    )
    .category("network", "Network")
    .connection()
    .input("data", "core/bytes@1")
    .build()
}

pub(crate) fn ws_binary_definition() -> String {
    Definition::entry(crate::PACKAGE, "ws-binary", "On WebSocket binary message")
        .category("network", "Network")
        .connection()
        .payload("core/bytes@1")
        .build()
}
