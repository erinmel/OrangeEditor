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
/// Total payload bytes retained while the socket is busy or down.
const MAX_QUEUED_BYTES: usize = 16 * 1024 * 1024;
/// How long the I/O thread waits for incoming data before it looks at the outgoing
/// queue again; the longest a queued message waits on an idle socket.
const POLL: Duration = Duration::from_millis(10);
const WRITE_TIMEOUT: Duration = Duration::from_secs(2);
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
    "The connection's waiting messages exceed its count or byte limit.",
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
    /// Every text or binary message: a package trigger reads both.
    Messages,
}

/// One message waiting to be sent.
#[derive(Debug, Clone)]
enum Outgoing {
    Text(Arc<String>),
    Binary(Arc<Vec<u8>>),
}

impl Outgoing {
    fn len(&self) -> usize {
        match self {
            Self::Text(text) => text.len(),
            Self::Binary(bytes) => bytes.len(),
        }
    }
}

#[derive(Default)]
struct Outbox {
    messages: VecDeque<Outgoing>,
    bytes: usize,
}

impl Outbox {
    fn push(&mut self, message: Outgoing) -> Result<(), KernelFault> {
        let total = self
            .bytes
            .checked_add(message.len())
            .filter(|bytes| *bytes <= MAX_QUEUED_BYTES)
            .ok_or(QUEUE_FULL)?;
        if self.messages.len() >= MAX_QUEUED_MESSAGES {
            return Err(QUEUE_FULL);
        }
        self.messages.push_back(message);
        self.bytes = total;
        Ok(())
    }

    fn sent(&mut self) {
        if let Some(message) = self.messages.pop_front() {
            self.bytes -= message.len();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Read, Write};
    use tungstenite::protocol::Role;

    #[test]
    fn limits_cover_backlog_and_the_message_being_written() {
        let mut queue = Outbox::default();
        let text = Arc::new("é".to_owned());
        let binary = Arc::new(vec![0; MAX_QUEUED_BYTES - 2]);
        queue.push(Outgoing::Binary(binary)).unwrap();
        queue.push(Outgoing::Text(Arc::clone(&text))).unwrap();
        assert_eq!(queue.bytes, MAX_QUEUED_BYTES);
        assert!(queue.push(Outgoing::Text(Arc::clone(&text))).is_err());
        let in_flight = queue.messages.front().cloned().unwrap();
        assert!(queue.push(Outgoing::Text(Arc::clone(&text))).is_err());
        assert_eq!(in_flight.len(), MAX_QUEUED_BYTES - 2);
        queue.sent();
        assert_eq!(queue.bytes, 2);
        for _ in 1..MAX_QUEUED_MESSAGES {
            queue.push(Outgoing::Text(Arc::clone(&text))).unwrap();
        }
        assert!(queue.push(Outgoing::Text(text)).is_err());
        for _ in 0..MAX_QUEUED_MESSAGES {
            queue.sent();
        }
        assert_eq!(queue.bytes, 0);
    }

    #[derive(Default)]
    struct MemorySocket {
        input: Cursor<Vec<u8>>,
        output: Vec<u8>,
    }

    impl Read for MemorySocket {
        fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
            self.input.read(bytes)
        }
    }

    impl Write for MemorySocket {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.output.write(bytes)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_peer_close_is_answered_before_the_socket_is_released() {
        let mut peer = WebSocket::from_raw_socket(MemorySocket::default(), Role::Server, None);
        peer.send(Message::Close(None)).unwrap();
        let wire = MemorySocket {
            input: Cursor::new(std::mem::take(&mut peer.get_mut().output)),
            output: Vec::new(),
        };
        let mut socket = WebSocket::from_raw_socket(wire, Role::Client, None);
        let shared = Shared {
            url: "ws://example.invalid".into(),
            headers: Vec::new(),
            outgoing: Mutex::new(Outbox::default()),
            listeners: Listeners::new(),
            connected: AtomicBool::new(true),
            life: Lifecycle::new(),
        };
        assert_eq!(serve(&shared, &mut socket), CLOSED);
        peer.get_mut().input = Cursor::new(std::mem::take(&mut socket.get_mut().output));
        assert_eq!(peer.read().unwrap(), Message::Close(None));
    }
}

struct Shared {
    url: String,
    /// Headers of the opening handshake; they can be credentials.
    headers: Vec<(String, String)>,
    outgoing: Mutex<Outbox>,
    /// Each listener with what it fires for.
    listeners: Listeners<Listen>,
    /// Whether the socket is open now.
    connected: AtomicBool,
    /// Started once a flow has attached its triggers; the socket is not opened before.
    life: Lifecycle,
}
impl Drop for Shared {
    fn drop(&mut self) {
        use zeroize::Zeroize as _;
        for (_, value) in &mut self.headers {
            value.zeroize();
        }
    }
}
impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shared")
            .field("endpoint", &crate::EndpointDebug(&self.url))
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
        let message = matches!(what, Listen::Text | Listen::Binary);
        self.listeners.emit(|listens| {
            (*listens == what || (message && *listens == Listen::Messages)).then(|| payload.clone())
        });
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
    /// Queues a message on this socket for a package node, through the same bounded
    /// queue as Send: the socket may be reconnecting, and queued is not delivered.
    fn send_message(&self, message: orange_sdk::Message) -> Result<(), orange_sdk::MessageError> {
        use orange_sdk::{Message as Outbound, MessageError};
        let message = match message {
            Outbound::Text(text) => Outgoing::Text(Arc::new(text.to_string())),
            Outbound::Binary(data) => Outgoing::Binary(Arc::new(data.to_vec())),
        };
        if message.len() > MAX_QUEUED_BYTES {
            return Err(MessageError::TooLarge);
        }
        self.0
            .outgoing
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message)
            .map_err(|_| MessageError::Full)
    }
    /// Hands every incoming message to `emitter` until the guard is dropped, through
    /// the same listener list as the Message and Binary triggers.
    fn listen_messages(&self, emitter: Emitter) -> Result<Box<dyn Send>, orange_sdk::MessageError> {
        Ok(Listeners::listen(
            Arc::clone(&self.0),
            |shared| &shared.listeners,
            Listen::Messages,
            emitter,
        ))
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
    let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
}

/// Why a served socket stopped, as the disconnected entry's payload says it.
const CLOSED: &str = "closed by the server";
const FAILED: &str = "connection lost";

/// Serves one established socket until it fails, closes or the session ends, and says
/// why it stopped.
fn serve(
    shared: &Shared,
    socket: &mut WebSocket<impl std::io::Read + std::io::Write>,
) -> &'static str {
    loop {
        if shared.stopped() {
            let _ = socket.close(None);
            let _ = socket.flush();
            return "connection closed";
        }
        // Give incoming messages and stop requests a turn even under steady writes.
        for _ in 0..16 {
            let next = shared
                .outgoing
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .messages
                .front()
                .cloned();
            let Some(next) = next else { break };
            let message = match next {
                Outgoing::Text(text) => Message::text(text.as_ref().clone()),
                Outgoing::Binary(data) => Message::binary(data.as_ref().clone()),
            };
            if socket.send(message).is_err() {
                // Keep the message for the next socket. WebSocket has no delivery
                // receipt, so a failure after a partial write can lead to a retry.
                return FAILED;
            }
            shared
                .outgoing
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .sent();
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
            Ok(Message::Close(_)) => {
                let _ = socket.flush();
                return CLOSED;
            }
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
            set_poll(&socket);
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
    fn open(&self, settings: &Map<String, Json>) -> Result<Arc<dyn Session>, String> {
        let url = settings
            .get("url")
            .and_then(Json::as_str)
            .ok_or("`url` is required")?;
        crate::connection_uri(url, ["ws", "wss"])
            .ok_or("`url` must be an absolute WebSocket address without user info or fragment")?;
        let shared = Arc::new(Shared {
            url: url.to_owned(),
            headers: crate::auth_headers(settings)?,
            outgoing: Mutex::new(Outbox::default()),
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
            (false, [Value::Str(text)]) => Outgoing::Text(Arc::clone(text)),
            (true, [Value::Bytes(data)]) => Outgoing::Binary(Arc::clone(data)),
            _ => return Err(BAD_MESSAGE),
        };
        let mut outgoing = shared
            .outgoing
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        outgoing.push(message)?;
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
            .messages
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
