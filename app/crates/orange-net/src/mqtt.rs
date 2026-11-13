//! MQTT 3.1.1 connections: publish to topics and be triggered by the messages of the
//! topics a connection subscribes to.
//!
//! A session owns one I/O thread, like a WebSocket session. It connects once a flow
//! using it has attached its triggers, subscribes to the connection's topic filters,
//! keeps the connection alive, reconnects with backoff and subscribes again. Nodes
//! never touch the socket: publishing queues a message and returns.
//!
//! Quality of service 0 and 1 are supported. With 1, a message stays queued until the
//! broker acknowledges it and is sent again, marked as a duplicate, after a reconnect;
//! messages received with 1 are acknowledged once they have been handed to the
//! listening entries. Each connection starts a clean session, so a broker keeps
//! nothing for it while it is away. MQTT 5, quality of service 2, wills and
//! persistent sessions are not provided.
use orange_sdk::{
    Call, ConnectionAdapter, Definition, Emitter, KernelFault, ListRepr, Listeners, ServiceNode,
    ServiceSource, Session, Value,
};
use serde_json::{Map, Value as Json};
use std::any::Any;
use std::collections::VecDeque;
use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use crate::fault;

/// Messages a session holds while its socket is busy or down. Publishing beyond it
/// fails.
pub const MAX_QUEUED_MESSAGES: usize = 256;
/// Largest packet accepted from a broker and largest message published.
pub const MAX_PACKET_BYTES: usize = 8 * 1024 * 1024;
/// Most topic filters one connection subscribes to.
pub const MAX_TOPICS: usize = 64;
const POLL: Duration = Duration::from_millis(10);
const FIRST_RETRY: Duration = Duration::from_millis(250);
const MAX_RETRY: Duration = Duration::from_secs(5);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

const NO_SESSION: KernelFault = fault(
    "org.orange.net/Connection",
    "org.orange.net/connection-kind",
    "The node needs an MQTT connection.",
);
const BAD_MESSAGE: KernelFault = fault(
    "org.orange.net/Request",
    "org.orange.net/mqtt-message-invalid",
    "The topic or message is not valid for publishing.",
);
const QUEUE_FULL: KernelFault = fault(
    "org.orange.net/Busy",
    "org.orange.net/send-queue-full",
    "The connection has too many messages waiting to be sent.",
);

// ---- packets -------------------------------------------------------------------

fn text(out: &mut Vec<u8>, value: &[u8]) {
    // Lengths are checked where values enter: a string is at most 65,535 bytes.
    let length = u16::try_from(value.len()).unwrap_or(u16::MAX);
    out.extend_from_slice(&length.to_be_bytes());
    out.extend_from_slice(value.get(..usize::from(length)).unwrap_or(value));
}

/// A whole packet: the first byte, the remaining length and the body.
fn packet(first: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![first];
    let mut length = body.len();
    loop {
        let mut byte = (length % 128) as u8;
        length /= 128;
        if length > 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if length == 0 {
            break;
        }
    }
    out.extend_from_slice(body);
    out
}

fn connect_packet(settings: &Settings) -> Vec<u8> {
    let mut body = Vec::new();
    text(&mut body, b"MQTT");
    body.push(4);
    let mut flags = 0x02;
    if settings.username.is_some() {
        flags |= 0x80;
    }
    if settings.password.is_some() {
        flags |= 0x40;
    }
    body.push(flags);
    body.extend_from_slice(&settings.keep_alive.to_be_bytes());
    text(&mut body, settings.client_id.as_bytes());
    if let Some(username) = &settings.username {
        text(&mut body, username.as_bytes());
    }
    if let Some(password) = &settings.password {
        text(&mut body, password.as_bytes());
    }
    packet(0x10, &body)
}

fn subscribe_packet(id: u16, filters: &[String], qos: u8) -> Vec<u8> {
    let mut body = id.to_be_bytes().to_vec();
    for filter in filters {
        text(&mut body, filter.as_bytes());
        body.push(qos);
    }
    packet(0x82, &body)
}

fn publish_packet(message: &Outgoing, qos: u8, id: u16, duplicate: bool) -> Vec<u8> {
    let mut body = Vec::with_capacity(message.topic.len() + message.payload.len() + 4);
    text(&mut body, message.topic.as_bytes());
    if qos > 0 {
        body.extend_from_slice(&id.to_be_bytes());
    }
    body.extend_from_slice(&message.payload);
    let first =
        0x30 | (u8::from(duplicate && qos > 0) << 3) | (qos << 1) | u8::from(message.retain);
    packet(first, &body)
}

/// A packet's first byte, its body and its total length.
type Packet<'a> = (u8, &'a [u8], usize);

/// One complete packet at the start of `buffer`: its first byte, its body and its
/// total length. `Ok(None)` when more bytes are needed.
fn parse(buffer: &[u8]) -> Result<Option<Packet<'_>>, ()> {
    let Some(&first) = buffer.first() else {
        return Ok(None);
    };
    let (mut length, mut shift, mut at) = (0usize, 0u32, 1usize);
    loop {
        let Some(&byte) = buffer.get(at) else {
            return Ok(None);
        };
        at += 1;
        length |= usize::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            break;
        }
        shift += 7;
        if shift > 21 {
            return Err(());
        }
    }
    if length > MAX_PACKET_BYTES {
        return Err(());
    }
    match buffer.get(at..at + length) {
        Some(body) => Ok(Some((first, body, at + length))),
        None => Ok(None),
    }
}

/// A received PUBLISH: topic, payload, and the packet ID to acknowledge.
fn parse_publish(first: u8, body: &[u8]) -> Option<(&str, &[u8], Option<u16>)> {
    let qos = (first >> 1) & 3;
    let length = usize::from(u16::from_be_bytes([*body.first()?, *body.get(1)?]));
    let topic = std::str::from_utf8(body.get(2..2 + length)?).ok()?;
    let rest = body.get(2 + length..)?;
    match qos {
        0 => Some((topic, rest, None)),
        1 => {
            let id = u16::from_be_bytes([*rest.first()?, *rest.get(1)?]);
            Some((topic, rest.get(2..)?, Some(id)))
        }
        _ => None,
    }
}

// ---- connection ----------------------------------------------------------------

enum Stream {
    Plain(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}
impl Read for Stream {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.read(buffer),
            Self::Tls(stream) => stream.read(buffer),
        }
    }
}
impl Write for Stream {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.write(buffer),
            Self::Tls(stream) => stream.write(buffer),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Plain(stream) => stream.flush(),
            Self::Tls(stream) => stream.flush(),
        }
    }
}

fn open(settings: &Settings) -> Option<Stream> {
    use std::net::ToSocketAddrs;
    let address = (settings.host.as_str(), settings.port)
        .to_socket_addrs()
        .ok()?
        .next()?;
    let tcp = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT).ok()?;
    let _ = tcp.set_nodelay(true);
    // Short timeouts make reads and writes steps of the session loop.
    tcp.set_read_timeout(Some(POLL)).ok()?;
    tcp.set_write_timeout(Some(CONNECT_TIMEOUT)).ok()?;
    if !settings.tls {
        return Some(Stream::Plain(tcp));
    }
    let roots = match &settings.authorities {
        // A private broker is trusted through its own authority only.
        Some(authorities) => {
            let mut roots = rustls::RootCertStore::empty();
            for authority in authorities {
                roots.add(authority.clone()).ok()?;
            }
            roots
        }
        None => rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned()),
    };
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .ok()?
        .with_root_certificates(roots)
        .with_no_client_auth();
    let name = rustls::pki_types::ServerName::try_from(settings.host.clone()).ok()?;
    let connection = rustls::ClientConnection::new(Arc::new(config), name).ok()?;
    Some(Stream::Tls(Box::new(rustls::StreamOwned::new(
        connection, tcp,
    ))))
}

/// One message waiting to be published.
#[derive(Debug, Clone)]
struct Outgoing {
    topic: String,
    payload: Vec<u8>,
    retain: bool,
}

struct Settings {
    host: String,
    port: u16,
    tls: bool,
    /// The certificate authorities of `ca_file`, replacing the bundled ones.
    authorities: Option<Vec<rustls::pki_types::CertificateDer<'static>>>,
    client_id: String,
    username: Option<String>,
    password: Option<String>,
    keep_alive: u16,
    qos: u8,
    topics: Vec<String>,
}

struct Shared {
    settings: Settings,
    outgoing: Mutex<VecDeque<Outgoing>>,
    /// Each listener with whether it takes payloads as bytes instead of text.
    listeners: Listeners<bool>,
    started: AtomicBool,
    stop: AtomicBool,
}
/// The user name and password are left out.
impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shared")
            .field("host", &self.settings.host)
            .field("port", &self.settings.port)
            .field("topics", &self.settings.topics.len())
            .finish_non_exhaustive()
    }
}
impl Shared {
    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }
    /// Fires the listening entries. Text entries get `[topic, text]` and skip a
    /// payload that is not UTF-8; byte entries get the payload alone.
    fn deliver(&self, topic: &str, payload: &[u8]) {
        let as_text = std::str::from_utf8(payload).ok().map(|text| {
            let pair = [Arc::new(topic.to_owned()), Arc::new(text.to_owned())];
            Value::List(Arc::new(ListRepr::Str(Box::new(pair))))
        });
        let as_bytes = Value::Bytes(Arc::new(payload.to_vec()));
        self.listeners.emit(|bytes| {
            if *bytes {
                Some(as_bytes.clone())
            } else {
                as_text.clone()
            }
        });
    }
}

fn send(stream: &mut Stream, bytes: &[u8]) -> bool {
    stream
        .write_all(bytes)
        .and_then(|()| stream.flush())
        .is_ok()
}

/// Serves one established connection until it fails or the session ends. Messages
/// published with quality of service 1 and not yet acknowledged are put back at the
/// front of the queue, to be sent again as duplicates.
fn serve(shared: &Shared, stream: &mut Stream, resend: &mut VecDeque<Outgoing>) {
    let settings = &shared.settings;
    if !send(stream, &connect_packet(settings)) {
        return;
    }
    let mut buffer: Vec<u8> = Vec::new();
    let mut chunk = vec![0u8; 16 * 1024];
    let mut connected = false;
    let mut next_id: u16 = 1;
    // Sent with quality of service 1 and waiting for the broker's acknowledgement.
    let mut inflight: VecDeque<(u16, Outgoing)> = VecDeque::new();
    let opened = Instant::now();
    let mut last_sent = Instant::now();
    let mut last_heard = Instant::now();
    let keep_alive = Duration::from_secs(u64::from(settings.keep_alive.max(1)));
    let mut new_id = |inflight: &VecDeque<(u16, Outgoing)>| loop {
        let id = next_id;
        next_id = next_id.checked_add(1).unwrap_or(1);
        if !inflight.iter().any(|(used, _)| *used == id) {
            break id;
        }
    };
    let mut ok = true;
    while ok && !shared.stopped() {
        if connected {
            // Unacknowledged messages of the previous connection go first.
            while inflight.len() < 16 {
                let (message, duplicate) = match resend.pop_front() {
                    Some(message) => (message, true),
                    None => {
                        let next = shared
                            .outgoing
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .pop_front();
                        match next {
                            Some(message) => (message, false),
                            None => break,
                        }
                    }
                };
                let id = new_id(&inflight);
                if !send(
                    stream,
                    &publish_packet(&message, settings.qos, id, duplicate),
                ) {
                    resend.push_front(message);
                    ok = false;
                    break;
                }
                last_sent = Instant::now();
                if settings.qos > 0 {
                    inflight.push_back((id, message));
                }
            }
            if ok && last_sent.elapsed() >= keep_alive / 2 {
                ok = send(stream, &[0xc0, 0]);
                last_sent = Instant::now();
            }
            // A broker answers a ping; silence for longer than this means it is gone.
            if last_heard.elapsed() > keep_alive * 2 {
                ok = false;
            }
        } else if opened.elapsed() > CONNECT_TIMEOUT {
            ok = false;
        }
        match stream.read(&mut chunk) {
            Ok(0) => ok = false,
            Ok(count) => buffer.extend_from_slice(chunk.get(..count).unwrap_or_default()),
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(_) => ok = false,
        }
        let mut used = 0;
        while ok {
            let (first, body, length) = match parse(buffer.get(used..).unwrap_or_default()) {
                Ok(Some(found)) => found,
                Ok(None) => break,
                Err(()) => {
                    ok = false;
                    break;
                }
            };
            last_heard = Instant::now();
            match first >> 4 {
                // CONNACK: a return code other than 0 is a refusal.
                2 => {
                    if body.get(1) != Some(&0) {
                        ok = false;
                    } else {
                        connected = true;
                        if !settings.topics.is_empty() {
                            let id = new_id(&inflight);
                            ok = send(
                                stream,
                                &subscribe_packet(id, &settings.topics, settings.qos),
                            );
                            last_sent = Instant::now();
                        }
                    }
                }
                3 => match parse_publish(first, body) {
                    Some((topic, payload, id)) => {
                        shared.deliver(topic, payload);
                        if let Some(id) = id {
                            let [high, low] = id.to_be_bytes();
                            ok = send(stream, &[0x40, 2, high, low]);
                            last_sent = Instant::now();
                        }
                    }
                    None => ok = false,
                },
                // PUBACK
                4 => {
                    if let [high, low] = body {
                        let id = u16::from_be_bytes([*high, *low]);
                        inflight.retain(|(sent, _)| *sent != id);
                    }
                }
                // SUBACK, PINGRESP and anything else need no answer.
                _ => {}
            }
            used += length;
        }
        if used > 0 {
            let _ = buffer.drain(..used);
        }
    }
    if shared.stopped() {
        let _ = send(stream, &[0xe0, 0]);
    }
    // Oldest first, ahead of anything already waiting to be sent again.
    for (_, message) in inflight.into_iter().rev() {
        resend.push_front(message);
    }
}

fn run(shared: &Shared) {
    while !shared.started.load(Ordering::Acquire) && !shared.stopped() {
        thread::sleep(POLL);
    }
    let mut retry = FIRST_RETRY;
    let mut resend = VecDeque::new();
    while !shared.stopped() {
        if let Some(mut stream) = open(&shared.settings) {
            let before = Instant::now();
            serve(shared, &mut stream, &mut resend);
            // A connection that lasted is not a failing one.
            if before.elapsed() > MAX_RETRY {
                retry = FIRST_RETRY;
            }
        }
        let mut waited = Duration::ZERO;
        while waited < retry && !shared.stopped() {
            thread::sleep(POLL);
            waited += POLL;
        }
        retry = (retry * 2).min(MAX_RETRY);
    }
}

/// An MQTT connection. Dropping the last reference disconnects.
#[derive(Debug)]
struct MqttSession(Arc<Shared>);
impl Session for MqttSession {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn start(&self) {
        self.0.started.store(true, Ordering::Release);
    }
}
impl Drop for MqttSession {
    fn drop(&mut self) {
        self.0.stop.store(true, Ordering::Relaxed);
    }
}

/// A topic name or filter the protocol can carry.
fn is_topic(topic: &str, filter: bool) -> bool {
    !topic.is_empty()
        && topic.len() <= usize::from(u16::MAX)
        && !topic.contains('\0')
        && (filter || !topic.contains(['+', '#']))
}

static CLIENTS: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub(crate) struct MqttAdapter;
impl ConnectionAdapter for MqttAdapter {
    fn open(&self, settings: &Map<String, Json>) -> Result<Arc<dyn Session>, String> {
        let url = settings
            .get("url")
            .and_then(Json::as_str)
            .ok_or("`url` is required")?;
        let (tls, rest) = match (url.strip_prefix("mqtt://"), url.strip_prefix("mqtts://")) {
            (Some(rest), _) => (false, rest),
            (_, Some(rest)) => (true, rest),
            _ => return Err("`url` must start with mqtt:// or mqtts://".into()),
        };
        let rest = rest.trim_end_matches('/');
        let (host, port) = match rest.rsplit_once(':') {
            Some((host, port)) => (
                host,
                port.parse()
                    .map_err(|_| "`url` has a port that is not valid")?,
            ),
            None => (rest, if tls { 8883 } else { 1883 }),
        };
        if host.is_empty() || host.contains(['/', '@', ' ']) {
            return Err("`url` must be mqtt://host or mqtt://host:port".into());
        }
        let short = |name: &str| -> Result<Option<String>, String> {
            match settings.get(name) {
                None | Some(Json::Null) => Ok(None),
                Some(Json::String(value))
                    if !value.is_empty() && value.len() <= usize::from(u16::MAX) =>
                {
                    Ok(Some(value.clone()))
                }
                Some(_) => Err(format!("`{name}` must be text")),
            }
        };
        let (username, password) = (short("username")?, short("password")?);
        if password.is_some() && username.is_none() {
            return Err("`password` needs `username`".into());
        }
        let client_id = match short("client_id")? {
            Some(id) => id,
            None => format!(
                "orange-{}-{}",
                std::process::id(),
                CLIENTS.fetch_add(1, Ordering::Relaxed)
            ),
        };
        let number = |name: &str, default: u64, range: std::ops::RangeInclusive<u64>| match settings
            .get(name)
        {
            None => Ok(default),
            Some(value) => value
                .as_u64()
                .filter(|number| range.contains(number))
                .ok_or_else(|| {
                    format!(
                        "`{name}` must be an integer from {} to {}",
                        range.start(),
                        range.end()
                    )
                }),
        };
        let keep_alive = number("keep_alive_s", 30, 5..=3600)?;
        let qos = number("qos", 0, 0..=1)?;
        let topics = match settings.get("topics") {
            None => Vec::new(),
            Some(Json::Array(items)) if items.len() <= MAX_TOPICS => items
                .iter()
                .map(|item| {
                    item.as_str()
                        .filter(|filter| is_topic(filter, true))
                        .map(str::to_owned)
                        .ok_or("`topics` must be a list of topic filters")
                })
                .collect::<Result<_, _>>()?,
            Some(_) => return Err("`topics` must be a list of topic filters".into()),
        };
        let authorities = match short("ca_file")? {
            None => None,
            Some(_) if !tls => return Err("`ca_file` needs an mqtts:// URL".into()),
            Some(path) => {
                use rustls::pki_types::pem::PemObject;
                let pem = std::fs::read(&path).map_err(|_| "`ca_file` could not be read")?;
                let found: Vec<_> = rustls::pki_types::CertificateDer::pem_slice_iter(&pem)
                    .collect::<Result<_, _>>()
                    .map_err(|_| "`ca_file` is not a PEM certificate file")?;
                if found.is_empty() {
                    return Err("`ca_file` holds no certificate".into());
                }
                Some(found)
            }
        };
        let shared = Arc::new(Shared {
            settings: Settings {
                host: host.to_owned(),
                port,
                tls,
                authorities,
                client_id,
                username,
                password,
                keep_alive: u16::try_from(keep_alive).unwrap_or(30),
                qos: u8::try_from(qos).unwrap_or(0),
                topics,
            },
            outgoing: Mutex::new(VecDeque::new()),
            listeners: Listeners::new(),
            started: AtomicBool::new(false),
            stop: AtomicBool::new(false),
        });
        let io = Arc::clone(&shared);
        thread::Builder::new()
            .name("orange-net-mqtt".into())
            .spawn(move || run(&io))
            .map_err(|error| error.to_string())?;
        Ok(Arc::new(MqttSession(shared)))
    }
}

fn session(session: Option<&dyn Session>) -> Option<&Arc<Shared>> {
    session?
        .as_any()
        .downcast_ref::<MqttSession>()
        .map(|session| &session.0)
}

/// Queues one message. `then` means queued, not delivered.
#[derive(Debug)]
pub(crate) struct MqttPublish {
    pub binary: bool,
}
impl ServiceNode for MqttPublish {
    fn run(&self, call: &Call<'_>) -> Result<Vec<Value>, KernelFault> {
        let shared = session(call.session).ok_or(NO_SESSION)?;
        let (topic, payload, retain) = match (self.binary, call.inputs) {
            (false, [Value::Str(topic), Value::Str(text), Value::Bool(retain)]) => {
                (topic, text.as_bytes().to_vec(), *retain)
            }
            (true, [Value::Str(topic), Value::Bytes(data), Value::Bool(retain)]) => {
                (topic, data.as_ref().clone(), *retain)
            }
            _ => return Err(BAD_MESSAGE),
        };
        if !is_topic(topic, false) || topic.len() + payload.len() + 16 > MAX_PACKET_BYTES {
            return Err(BAD_MESSAGE);
        }
        let mut outgoing = shared
            .outgoing
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if outgoing.len() >= MAX_QUEUED_MESSAGES {
            return Err(QUEUE_FULL);
        }
        outgoing.push_back(Outgoing {
            topic: topic.as_str().to_owned(),
            payload,
            retain,
        });
        Ok(Vec::new())
    }
}

/// Fires its entry once per message received on the connection's topics.
#[derive(Debug)]
pub(crate) struct MqttMessage {
    pub binary: bool,
}
impl ServiceSource for MqttMessage {
    fn start(
        &self,
        session: Option<Arc<dyn Session>>,
        emitter: Emitter,
    ) -> Result<Box<dyn Send>, String> {
        let shared =
            crate::mqtt::session(session.as_deref()).ok_or("the entry needs an MQTT connection")?;
        Ok(Listeners::listen(
            Arc::clone(shared),
            |shared| &shared.listeners,
            self.binary,
            emitter,
        ))
    }
}

pub(crate) fn publish_definition() -> String {
    Definition::node(crate::PACKAGE, "mqtt-publish", "Publish MQTT message")
        .category("network", "Network")
        .connection()
        .input("topic", "core/string@1")
        .input("text", "core/string@1")
        .input("retain", "core/bool@1")
        .build()
}

pub(crate) fn publish_bytes_definition() -> String {
    Definition::node(
        crate::PACKAGE,
        "mqtt-publish-bytes",
        "Publish MQTT binary message",
    )
    .category("network", "Network")
    .connection()
    .input("topic", "core/string@1")
    .input("data", "core/bytes@1")
    .input("retain", "core/bool@1")
    .build()
}

pub(crate) fn message_definition() -> String {
    Definition::entry(crate::PACKAGE, "mqtt-message", "On MQTT message")
        .category("network", "Network")
        .connection()
        .payload("core/list@1<core/string@1>")
        .build()
}

pub(crate) fn binary_definition() -> String {
    Definition::entry(crate::PACKAGE, "mqtt-binary", "On MQTT binary message")
        .category("network", "Network")
        .connection()
        .payload("core/bytes@1")
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        Settings {
            host: "localhost".into(),
            port: 1883,
            tls: false,
            authorities: None,
            client_id: "c".into(),
            username: Some("u".into()),
            password: Some("p".into()),
            keep_alive: 60,
            qos: 1,
            topics: vec!["a/b".into()],
        }
    }

    #[test]
    fn packets_have_the_bytes_the_protocol_specifies() {
        // CONNECT: protocol name and level 4, clean session with user name and
        // password, keep-alive 60, then client ID, user name and password.
        assert_eq!(
            connect_packet(&settings()),
            [
                0x10, 19, 0, 4, b'M', b'Q', b'T', b'T', 4, 0xc2, 0, 60, 0, 1, b'c', 0, 1, b'u', 0,
                1, b'p'
            ]
        );
        assert_eq!(
            subscribe_packet(10, &["a/b".into()], 1),
            [0x82, 8, 0, 10, 0, 3, b'a', b'/', b'b', 1]
        );
        let message = Outgoing {
            topic: "a/b".into(),
            payload: b"hi".to_vec(),
            retain: true,
        };
        assert_eq!(
            publish_packet(&message, 0, 7, true),
            [0x31, 7, 0, 3, b'a', b'/', b'b', b'h', b'i']
        );
        assert_eq!(
            publish_packet(&message, 1, 7, true),
            [0x3b, 9, 0, 3, b'a', b'/', b'b', 0, 7, b'h', b'i']
        );
        // The remaining length uses seven bits per byte: 321 is 0xC1 0x02.
        let long = packet(0x30, &[0; 321]);
        assert_eq!(&long[..3], [0x30, 0xc1, 0x02]);
    }

    #[test]
    fn received_bytes_are_split_into_whole_packets() {
        let message = Outgoing {
            topic: "t".into(),
            payload: vec![9; 300],
            retain: false,
        };
        let bytes = publish_packet(&message, 1, 513, false);
        // Every proper prefix is "not yet"; the whole is one packet.
        for end in 0..bytes.len() {
            assert_eq!(parse(&bytes[..end]), Ok(None), "{end}");
        }
        let (first, body, length) = parse(&bytes).unwrap().unwrap();
        assert_eq!(length, bytes.len());
        assert_eq!(
            parse_publish(first, body),
            Some(("t", &[9u8; 300][..], Some(513)))
        );
        // Quality of service 2 and an oversized or malformed length are refused.
        assert_eq!(parse_publish(0x34, body), None);
        assert_eq!(parse(&[0x30, 0xff, 0xff, 0xff, 0xff, 0x01]), Err(()));
        assert_eq!(parse(&[0x30, 0x80, 0x80, 0x80, 0x08]), Err(()));
    }

    #[test]
    fn settings_are_checked_when_a_document_opens() {
        let open = |json: Json| MqttAdapter.open(json.as_object().unwrap()).map(|_| ());
        let json = |text: &str| serde_json::from_str::<Json>(text).unwrap();
        assert!(
            open(json(
                r#"{"url": "mqtt://127.0.0.1:9", "topics": ["a/+/b", "c/#"]}"#
            ))
            .is_ok()
        );
        assert!(open(json(r#"{"url": "mqtts://broker.example"}"#)).is_ok());
        for bad in [
            r#"{}"#,
            r#"{"url": "http://broker"}"#,
            r#"{"url": "mqtt://"}"#,
            r#"{"url": "mqtt://host:notaport"}"#,
            r#"{"url": "mqtt://host", "qos": 2}"#,
            r#"{"url": "mqtt://host", "password": "p"}"#,
            r#"{"url": "mqtt://host", "topics": [""]}"#,
            r#"{"url": "mqtt://host", "topics": "a"}"#,
            r#"{"url": "mqtt://host", "keep_alive_s": 1}"#,
            r#"{"url": "mqtt://host", "ca_file": "ca.pem"}"#,
            r#"{"url": "mqtts://host", "ca_file": "no-such-file.pem"}"#,
        ] {
            assert!(open(json(bad)).is_err(), "{bad}");
        }
        assert!(is_topic("a/b", false) && !is_topic("a/#", false) && is_topic("a/#", true));
    }
}
