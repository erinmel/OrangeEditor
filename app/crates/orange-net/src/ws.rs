//! WebSocket connections: one socket per session, shared by every node that names it.
//!
//! A session owns one I/O thread. It connects once a flow using it has attached its
//! triggers, reconnects with backoff, writes queued
//! outgoing messages and fans each received text message out to the entry nodes that
//! listen. Nodes never touch the socket: sending queues a message and returns, and a
//! listener is fired through its emitter, so a slow or paused flow cannot stall the
//! socket and the socket cannot stall a flow.
use orange_sdk::{
    Call, ConnectionAdapter, EmitError, Emitter, KernelFault, ServiceNode, ServiceSource, Session,
    Value,
};
use serde_json::{Map, Value as Json};
use std::any::Any;
use std::collections::VecDeque;
use std::io::ErrorKind;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
    "The message must be text.",
);
const QUEUE_FULL: KernelFault = fault(
    "org.orange.net/Busy",
    "org.orange.net/send-queue-full",
    "The connection has too many messages waiting to be sent.",
);

#[derive(Debug)]
struct Shared {
    url: String,
    outgoing: Mutex<VecDeque<String>>,
    listeners: Mutex<Vec<(u64, Emitter)>>,
    next_listener: AtomicU64,
    /// Set once a flow has attached its triggers; the socket is not opened before.
    started: AtomicBool,
    stop: AtomicBool,
}
impl Shared {
    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }
    /// Fires every listening entry. A flow that has ended is forgotten; one whose
    /// mailbox is full misses this message, as its bounded mailbox requires.
    fn deliver(&self, text: &str) {
        let payload = Value::Str(Arc::new(text.to_owned()));
        self.listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|(_, emitter)| emitter.emit(Some(payload.clone())) != Err(EmitError::Closed));
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
            if let Err(error) = socket.send(Message::text(text.clone())) {
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
            Ok(Message::Text(text)) => shared.deliver(text.as_str()),
            Ok(Message::Close(_)) => return,
            Ok(_) => {}
            Err(Error::Io(error))
                if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(_) => return,
        }
    }
}

fn run(shared: &Shared) {
    // A server may speak first. Connecting only once a flow has attached its triggers
    // means its first messages have somewhere to go.
    while !shared.started.load(Ordering::Acquire) && !shared.stopped() {
        thread::sleep(POLL);
    }
    let mut retry = FIRST_RETRY;
    while !shared.stopped() {
        if let Ok((mut socket, _)) = tungstenite::connect(shared.url.as_str()) {
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
            outgoing: Mutex::new(VecDeque::new()),
            listeners: Mutex::new(Vec::new()),
            next_listener: AtomicU64::new(0),
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

/// Queues one text message. `then` means queued, not delivered: the socket may be
/// reconnecting, and a WebSocket gives no delivery receipt either way.
#[derive(Debug)]
pub(crate) struct WsSend;
impl ServiceNode for WsSend {
    fn run(&self, call: &Call<'_>) -> Result<Vec<Value>, KernelFault> {
        let shared = session(call.session).ok_or(NO_SESSION)?;
        let [Value::Str(text)] = call.inputs else {
            return Err(BAD_MESSAGE);
        };
        let mut outgoing = shared
            .outgoing
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if outgoing.len() >= MAX_QUEUED_MESSAGES {
            return Err(QUEUE_FULL);
        }
        outgoing.push_back(text.as_str().to_owned());
        Ok(Vec::new())
    }
}

/// Removes its listener when the flow that owns the entry closes.
struct Listening(Arc<Shared>, u64);
impl Drop for Listening {
    fn drop(&mut self) {
        self.0
            .listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|(id, _)| *id != self.1);
    }
}

/// Fires its entry once per text message received on the connection.
#[derive(Debug)]
pub(crate) struct WsMessage;
impl ServiceSource for WsMessage {
    fn start(
        &self,
        session: Option<Arc<dyn Session>>,
        emitter: Emitter,
    ) -> Result<Box<dyn Send>, String> {
        let shared = crate::ws::session(session.as_deref())
            .ok_or("the entry needs a WebSocket connection")?;
        let id = shared.next_listener.fetch_add(1, Ordering::Relaxed);
        shared
            .listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((id, emitter));
        Ok(Box::new(Listening(Arc::clone(shared), id)))
    }
}

pub(crate) const WS_SEND_DEFINITION: &str = r#"{
  "definition_id": "org.orange.net/ws-send",
  "label": {"key": "org.orange.net/ws-send-label", "fallback": "Send WebSocket message"},
  "editor": {"category_id": "org.orange.net/category-network",
    "category": {"key": "org.orange.net/category-network", "fallback": "Network"}},
  "definition_version": 1,
  "config_version": 1,
  "kind": "async",
  "suspends": "always",
  "determinism": "external",
  "connection": true,
  "ports": [
    {"port_id": "text", "direction": "input", "kind": "data", "type": "core/string@1"}
  ]
}"#;

pub(crate) const WS_MESSAGE_DEFINITION: &str = r#"{
  "definition_id": "org.orange.net/ws-message",
  "label": {"key": "org.orange.net/ws-message-label", "fallback": "On WebSocket message"},
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
