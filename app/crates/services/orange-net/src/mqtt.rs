//! MQTT 3.1.1 connections: publish to topics and be triggered by the messages of the
//! topics a connection subscribes to.
//!
//! A session owns one I/O thread, like a WebSocket session. It connects once a flow
//! using it has attached its triggers, subscribes to the connection's topic filters,
//! keeps the connection alive, reconnects with backoff and subscribes again. Nodes
//! never touch the socket: publishing queues a message and returns.
//!
//! Quality of service 0, 1 and 2 are supported. With 1 or 2, a message stays held until
//! the broker has finished its exchange and is sent again, marked as a duplicate and
//! with the same packet ID, after a reconnect; messages received with 1 are
//! acknowledged once they have been handed to the listening entries, and one received
//! with 2 is handed over once however often the broker repeats it. A connection starts
//! a clean session unless it is `persistent`, in which case the broker keeps its
//! subscriptions and the messages it missed while away. A `will` is a message the
//! broker publishes if the connection is lost without saying goodbye. MQTT 5 is not
//! provided.
use orange_sdk::{
    Arguments, Call, ConnectionAdapter, Definition, Emitter, KernelFault, Lifecycle, ListRepr,
    Listeners, ServiceNode, ServiceSource, Session, Value,
};
use serde_json::{Map, Value as Json};
use std::any::Any;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::{ErrorKind, Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use crate::{
    fault,
    transport::{self, Authorities, Stream},
};

/// Messages a session holds while its socket is busy or down. Publishing beyond it
/// fails.
pub(crate) const MAX_QUEUED_MESSAGES: usize = 256;
/// Largest packet accepted from a broker and largest message published.
pub(crate) const MAX_PACKET_BYTES: usize = 8 * 1024 * 1024;
/// Most topic filters one connection subscribes to.
pub(crate) const MAX_TOPICS: usize = 64;
const POLL: Duration = Duration::from_millis(10);
const FIRST_RETRY: Duration = Duration::from_millis(250);
const MAX_RETRY: Duration = Duration::from_secs(5);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Longest a closing session waits for its disconnect to be sent.
const GOODBYE: Duration = Duration::from_millis(500);

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
    let mut flags = if settings.persistent { 0 } else { 0x02 };
    if let Some(will) = &settings.will {
        flags |= 0x04 | (settings.qos << 3) | (u8::from(will.retain) << 5);
    }
    if settings.username.is_some() {
        flags |= 0x80;
    }
    if settings.password.is_some() {
        flags |= 0x40;
    }
    body.push(flags);
    body.extend_from_slice(&settings.keep_alive.to_be_bytes());
    text(&mut body, settings.client_id.as_bytes());
    if let Some(will) = &settings.will {
        text(&mut body, will.topic.as_bytes());
        text(&mut body, &will.payload);
    }
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

fn unsubscribe_packet(id: u16, filters: &[String]) -> Vec<u8> {
    let mut body = id.to_be_bytes().to_vec();
    for filter in filters {
        text(&mut body, filter.as_bytes());
    }
    packet(0xa2, &body)
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

/// A received PUBLISH: topic, payload, and with quality of service 1 or 2 the packet
/// ID to answer, with that quality.
type Publish<'a> = (&'a str, &'a [u8], Option<(u16, u8)>);

fn parse_publish(first: u8, body: &[u8]) -> Option<Publish<'_>> {
    let qos = (first >> 1) & 3;
    let length = usize::from(u16::from_be_bytes([*body.first()?, *body.get(1)?]));
    let topic = std::str::from_utf8(body.get(2..2 + length)?).ok()?;
    // A topic name the broker sends has no wildcards and is not empty [MQTT-3.3.2-2];
    // anything else is a protocol error, which closes the connection.
    if !is_topic(topic, false) {
        return None;
    }
    let rest = body.get(2 + length..)?;
    match qos {
        0 => Some((topic, rest, None)),
        1 | 2 => {
            let id = u16::from_be_bytes([*rest.first()?, *rest.get(1)?]);
            Some((topic, rest.get(2..)?, Some((id, qos))))
        }
        _ => None,
    }
}

/// A packet that carries only a packet ID: PUBACK, PUBREC, PUBREL and PUBCOMP.
fn answer(first: u8, id: u16) -> [u8; 4] {
    let [high, low] = id.to_be_bytes();
    [first, 2, high, low]
}

// ---- connection ----------------------------------------------------------------

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
    authorities: Option<Authorities>,
    client_id: String,
    username: Option<String>,
    password: Option<String>,
    keep_alive: u16,
    qos: u8,
    topics: Vec<String>,
    /// The broker keeps the session between connections.
    persistent: bool,
    /// Published by the broker if the connection is lost.
    will: Option<Outgoing>,
}
impl Drop for Settings {
    fn drop(&mut self) {
        use zeroize::Zeroize as _;
        self.password.zeroize();
    }
}

/// What a session and its broker still owe each other, kept across reconnects.
#[derive(Default)]
struct Flight {
    /// Published with quality of service 1 or 2 and not finished: the packet ID, the
    /// message, and whether the broker has confirmed receiving it (only with 2, where
    /// the release is then still to be completed).
    sent: VecDeque<(u16, Outgoing, bool)>,
    /// Packet IDs of messages received with quality of service 2 and already handed
    /// to the entries, until the broker releases them.
    received: VecDeque<u16>,
    subscribed: BTreeSet<String>,
    last_id: u16,
}
impl Flight {
    fn new_id(&mut self) -> u16 {
        loop {
            self.last_id = self.last_id.checked_add(1).unwrap_or(1);
            if !self.sent.iter().any(|(used, ..)| *used == self.last_id) {
                break self.last_id;
            }
        }
    }

    fn sync_topics(
        &mut self,
        shared: &Shared,
        stream: &mut Stream,
        reassert: bool,
    ) -> Option<bool> {
        let desired = shared.topics();
        let add: Vec<_> = if reassert {
            desired.iter().cloned().collect()
        } else {
            desired.difference(&self.subscribed).cloned().collect()
        };
        let remove: Vec<_> = self.subscribed.difference(&desired).cloned().collect();
        if !remove.is_empty() {
            if !send(stream, &unsubscribe_packet(self.new_id(), &remove)) {
                return None;
            }
            self.subscribed.retain(|filter| desired.contains(filter));
        }
        if !add.is_empty() {
            if !send(
                stream,
                &subscribe_packet(self.new_id(), &add, shared.settings.qos),
            ) {
                return None;
            }
            self.subscribed.extend(add.iter().cloned());
        }
        Some(!add.is_empty() || !remove.is_empty())
    }
}
/// Most received packet IDs remembered; a broker sends far fewer at once.
const MAX_RECEIVED: usize = 1024;

struct Shared {
    settings: Settings,
    outgoing: Mutex<VecDeque<Outgoing>>,
    listeners: Listeners<Listener>,
    topics: Mutex<Topics>,
    /// Started once a flow has attached its triggers; stopped when the session ends.
    life: Lifecycle,
    /// Stopped when the I/O thread has ended.
    io: Lifecycle,
}

struct Listener {
    binary: bool,
    topic: Option<String>,
}

struct Topics {
    filters: BTreeMap<String, usize>,
    generation: u64,
}

struct TopicGuard {
    shared: Arc<Shared>,
    topic: String,
}
impl Drop for TopicGuard {
    fn drop(&mut self) {
        let mut topics = self
            .shared
            .topics
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(count) = topics.filters.get_mut(&self.topic) {
            *count -= 1;
            if *count == 0 {
                let _ = topics.filters.remove(&self.topic);
                topics.generation = topics.generation.wrapping_add(1);
            }
        }
    }
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
        self.life.stopped()
    }

    fn topics(&self) -> BTreeSet<String> {
        let topics = self.topics.lock().unwrap_or_else(PoisonError::into_inner);
        self.settings
            .topics
            .iter()
            .chain(topics.filters.keys())
            .cloned()
            .collect()
    }

    fn topic_generation(&self) -> u64 {
        self.topics
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .generation
    }
    /// Fires the listening entries. Text entries get `[topic, text]` and skip a
    /// payload that is not UTF-8; byte entries get the payload alone.
    fn deliver(&self, topic: &str, payload: &[u8]) {
        let as_text = std::str::from_utf8(payload).ok().map(|text| {
            let pair = [Arc::new(topic.to_owned()), Arc::new(text.to_owned())];
            Value::List(Arc::new(ListRepr::Str(Box::new(pair))))
        });
        let as_bytes = Value::Bytes(Arc::new(payload.to_vec()));
        self.listeners.emit(|listener| {
            let subscribed = match &listener.topic {
                Some(filter) => matches_filter(filter, topic),
                None => self
                    .settings
                    .topics
                    .iter()
                    .any(|filter| matches_filter(filter, topic)),
            };
            if !subscribed {
                None
            } else if listener.binary {
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

/// Serves one established connection until it fails or the session ends. What is
/// not finished stays in `flight` for the next connection.
fn serve(shared: &Shared, stream: &mut Stream, flight: &mut Flight) {
    let settings = &shared.settings;
    if !send(stream, &connect_packet(settings)) {
        return;
    }
    let mut buffer: Vec<u8> = Vec::new();
    let mut chunk = vec![0u8; 16 * 1024];
    let mut connected = false;
    let opened = Instant::now();
    let mut last_sent = Instant::now();
    let mut last_heard = Instant::now();
    let mut generation = 0;
    let keep_alive = Duration::from_secs(u64::from(settings.keep_alive.max(1)));
    let mut ok = true;
    while ok && !shared.stopped() {
        if connected {
            let next_generation = shared.topic_generation();
            if generation != next_generation {
                match flight.sync_topics(shared, stream, false) {
                    Some(true) => last_sent = Instant::now(),
                    Some(false) => {}
                    None => break,
                }
                generation = next_generation;
            }
            while flight.sent.len() < 16 {
                let mut outgoing = shared
                    .outgoing
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                let Some(message) = outgoing.pop_front() else {
                    break;
                };
                let id = flight.new_id();
                if !send(stream, &publish_packet(&message, settings.qos, id, false)) {
                    outgoing.push_front(message);
                    ok = false;
                    break;
                }
                last_sent = Instant::now();
                if settings.qos > 0 {
                    flight.sent.push_back((id, message, false));
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
            let id = match body {
                [high, low] => Some(u16::from_be_bytes([*high, *low])),
                _ => None,
            };
            match (first >> 4, id) {
                // CONNACK: a return code other than 0 is a refusal.
                (2, _) => {
                    if body.get(1) != Some(&0) {
                        ok = false;
                    } else {
                        connected = true;
                        if body.first().is_none_or(|flags| flags & 1 == 0) {
                            // The broker holds nothing of an earlier connection: an
                            // exchange it had confirmed cannot be completed any more.
                            flight.received.clear();
                            flight.sent.retain(|(_, _, confirmed)| !confirmed);
                            flight.subscribed.clear();
                        }
                        // Subscribing comes first, so a message that returns to
                        // this connection finds its subscription.
                        generation = shared.topic_generation();
                        ok = flight.sync_topics(shared, stream, true).is_some();
                        // Then what the last connection left unfinished.
                        for (id, message, confirmed) in &flight.sent {
                            ok = ok
                                && if *confirmed {
                                    send(stream, &answer(0x62, *id))
                                } else {
                                    send(stream, &publish_packet(message, settings.qos, *id, true))
                                };
                        }
                        last_sent = Instant::now();
                    }
                }
                (3, _) => match parse_publish(first, body) {
                    Some((topic, payload, None)) => shared.deliver(topic, payload),
                    Some((topic, payload, Some((id, 1)))) => {
                        shared.deliver(topic, payload);
                        ok = send(stream, &answer(0x40, id));
                        last_sent = Instant::now();
                    }
                    Some((topic, payload, Some((id, _)))) => {
                        // A repeat of a message already handed over is only confirmed.
                        if !flight.received.contains(&id) {
                            shared.deliver(topic, payload);
                            if flight.received.len() >= MAX_RECEIVED {
                                let _ = flight.received.pop_front();
                            }
                            flight.received.push_back(id);
                        }
                        ok = send(stream, &answer(0x50, id));
                        last_sent = Instant::now();
                    }
                    None => ok = false,
                },
                // PUBACK and PUBCOMP finish a message this side published.
                (4 | 7, Some(id)) => flight.sent.retain(|(sent, ..)| *sent != id),
                // PUBREC: the broker has the message; release it.
                (5, Some(id)) => {
                    if let Some(entry) = flight.sent.iter_mut().find(|(sent, ..)| *sent == id) {
                        entry.2 = true;
                    }
                    ok = send(stream, &answer(0x62, id));
                    last_sent = Instant::now();
                }
                // PUBREL: the broker will not repeat the message again.
                (6, Some(id)) => {
                    flight.received.retain(|received| *received != id);
                    ok = send(stream, &answer(0x70, id));
                    last_sent = Instant::now();
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
}

fn run(shared: &Shared) {
    if !shared.life.wait_started() {
        shared.io.stop();
        return;
    }
    let mut retry = FIRST_RETRY;
    let mut flight = Flight::default();
    while !shared.stopped() {
        if let Some(mut stream) = transport::open(
            &shared.settings.host,
            shared.settings.port,
            shared.settings.tls,
            shared.settings.authorities.as_ref(),
            POLL,
            CONNECT_TIMEOUT,
        ) {
            let before = Instant::now();
            serve(shared, &mut stream, &mut flight);
            // A connection that lasted is not a failing one.
            if before.elapsed() > MAX_RETRY {
                retry = FIRST_RETRY;
            }
        }
        let _ = shared.life.rest(retry);
        retry = (retry * 2).min(MAX_RETRY);
    }
    shared.io.stop();
}

/// An MQTT connection. Dropping the last reference disconnects.
#[derive(Debug)]
struct MqttSession(Arc<Shared>);
impl Session for MqttSession {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn start(&self) {
        self.0.life.start();
    }
}
impl Drop for MqttSession {
    fn drop(&mut self) {
        self.0.life.stop();
        // Give the I/O thread a moment to say goodbye: a broker that only sees the
        // connection close publishes the will, as if the client had been lost.
        let _ = self.0.io.rest(GOODBYE);
    }
}

/// A topic name or filter the protocol can carry.
fn is_topic(topic: &str, filter: bool) -> bool {
    if topic.is_empty() || topic.len() > usize::from(u16::MAX) || topic.contains('\0') {
        return false;
    }
    if !filter {
        return !topic.contains(['+', '#']);
    }
    let mut levels = topic.split('/').peekable();
    while let Some(level) = levels.next() {
        if (level.contains('#') && (level != "#" || levels.peek().is_some()))
            || (level.contains('+') && level != "+")
        {
            return false;
        }
    }
    true
}

fn matches_filter(filter: &str, topic: &str) -> bool {
    if topic.starts_with('$') && !filter.starts_with('$') {
        return false;
    }
    let mut levels = topic.split('/');
    for wanted in filter.split('/') {
        if wanted == "#" {
            return true;
        }
        let Some(actual) = levels.next() else {
            return false;
        };
        if wanted != "+" && wanted != actual {
            return false;
        }
    }
    levels.next().is_none()
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
        let given_id = short("client_id")?;
        let flag = |name: &str, from: &Map<String, Json>| match from.get(name) {
            None => Ok(false),
            Some(Json::Bool(value)) => Ok(*value),
            Some(_) => Err(format!("`{name}` must be true or false")),
        };
        let persistent = flag("persistent", settings)?;
        if persistent && given_id.is_none() {
            // A broker finds a kept session by the client's ID.
            return Err("`persistent` needs `client_id`".into());
        }
        let will = match settings.get("will") {
            None => None,
            Some(Json::Object(will)) => {
                let topic = will
                    .get("topic")
                    .and_then(Json::as_str)
                    .filter(|topic| is_topic(topic, false))
                    .ok_or("`will.topic` must be a topic")?;
                let message = match will.get("message") {
                    None => "",
                    Some(Json::String(text)) if text.len() <= usize::from(u16::MAX) => text,
                    Some(_) => return Err("`will.message` must be text".into()),
                };
                Some(Outgoing {
                    topic: topic.to_owned(),
                    payload: message.as_bytes().to_vec(),
                    retain: flag("retain", will)?,
                })
            }
            Some(_) => return Err("`will` must be an object with `topic` and `message`".into()),
        };
        let client_id = given_id.unwrap_or_else(|| {
            format!(
                "orange-{}-{}",
                std::process::id(),
                CLIENTS.fetch_add(1, Ordering::Relaxed)
            )
        });
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
        let qos = number("qos", 0, 0..=2)?;
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
            Some(path) => Some(transport::load_authorities(&path)?),
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
                persistent,
                will,
            },
            outgoing: Mutex::new(VecDeque::new()),
            listeners: Listeners::new(),
            topics: Mutex::new(Topics {
                filters: BTreeMap::new(),
                generation: 0,
            }),
            life: Lifecycle::new(),
            io: Lifecycle::new(),
        });
        let io = Arc::clone(&shared);
        thread::Builder::new()
            .name("orange-net-mqtt".into())
            .spawn(move || run(&io))
            .map_err(|error| error.to_string())?;
        Ok(Arc::new(MqttSession(shared)))
    }
    fn about(&self) -> orange_sdk::AdapterAbout {
        orange_sdk::AdapterAbout {
            label: "MQTT broker".into(),
            purpose: "Publishes and receives messages on the topics of an MQTT broker.".into(),
        }
    }
    fn texts(&self) -> Vec<orange_sdk::SettingText> {
        use crate::{secret_setting, text_setting};
        vec![
            orange_sdk::SettingText {
                required: true,
                ..text_setting(
                    "url",
                    "Broker",
                    "mqtt://host, or mqtts://host for TLS, with an optional :port.",
                    "mqtts://broker.example.com",
                )
            },
            text_setting(
                "username",
                "User name",
                "The user name the broker knows.",
                "",
            ),
            secret_setting("password", "Password", "The user's password."),
            text_setting(
                "client_id",
                "Client ID",
                "How the broker knows this client; made up when left empty.",
                "",
            ),
            orange_sdk::SettingText {
                list: true,
                ..text_setting(
                    "topics",
                    "Topics",
                    "The topic filters to subscribe to, one per line.",
                    "sensors/#",
                )
            },
            text_setting(
                "ca_file",
                "Certificate authority file",
                "A PEM file of the authorities to trust, for a broker with a private one.",
                "",
            ),
        ]
    }
    fn toggles(&self) -> Vec<orange_sdk::SettingToggle> {
        vec![orange_sdk::SettingToggle {
            setting: "persistent".into(),
            label: "Keep the session".into(),
            help: "The broker keeps subscriptions and messages while this client is away; needs a client ID.".into(),
            default: false,
        }]
    }
    fn fields(&self) -> Vec<orange_sdk::SettingField> {
        vec![
            orange_sdk::SettingField {
                setting: "qos".into(),
                label: "Quality of service".into(),
                help: "0 sends at most once, 1 at least once, and 2 uses the four-step delivery exchange.".into(),
                unit: String::new(),
                min: 0.0,
                max: 2.0,
                default: Some(0.0),
                whole: true,
            },
            orange_sdk::SettingField {
                setting: "keep_alive_s".into(),
                label: "Keep alive".into(),
                help: "How often the broker hears from this client when nothing is sent.".into(),
                unit: "s".into(),
                min: 5.0,
                max: 3600.0,
                default: Some(30.0),
                whole: true,
            },
        ]
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
        self.start_with_arguments(session, &Map::new(), &Arguments::default(), emitter)
    }

    fn start_with_arguments(
        &self,
        session: Option<Arc<dyn Session>>,
        _settings: &Map<String, Json>,
        arguments: &Arguments,
        emitter: Emitter,
    ) -> Result<Box<dyn Send>, String> {
        let shared =
            crate::mqtt::session(session.as_deref()).ok_or("the entry needs an MQTT connection")?;
        let topic = match arguments.get("topic") {
            None => None,
            Some(Value::Str(topic)) if topic.is_empty() => None,
            Some(Value::Str(topic)) if is_topic(topic, true) => Some(topic.as_str().to_owned()),
            _ => return Err("the entry needs a valid MQTT topic filter".into()),
        };
        let mut topics = shared.topics.lock().unwrap_or_else(PoisonError::into_inner);
        let guard = if let Some(topic) = &topic {
            let configured = &shared.settings.topics;
            let distinct = configured.iter().collect::<BTreeSet<_>>();
            let additional = topics
                .filters
                .keys()
                .filter(|filter| !distinct.contains(filter))
                .count();
            if !distinct.contains(topic)
                && !topics.filters.contains_key(topic)
                && distinct.len() + additional >= MAX_TOPICS
            {
                return Err(
                    "the connection already has its maximum number of topic filters".into(),
                );
            }
            let count = topics.filters.entry(topic.clone()).or_default();
            *count += 1;
            if *count == 1 {
                topics.generation = topics.generation.wrapping_add(1);
            }
            Some(TopicGuard {
                shared: Arc::clone(shared),
                topic: topic.clone(),
            })
        } else {
            None
        };
        let listener = Listeners::listen(
            Arc::clone(shared),
            |shared| &shared.listeners,
            Listener {
                binary: self.binary,
                topic,
            },
            emitter,
        );
        Ok(Box::new((listener, guard)))
    }
}

pub(crate) fn publish_definition() -> String {
    Definition::node(crate::PACKAGE, "mqtt-publish", "Publish MQTT message")
        .category("network", "Network")
        .uses(crate::MQTT_ADAPTER)
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
    .uses(crate::MQTT_ADAPTER)
    .input("topic", "core/string@1")
    .input("data", "core/bytes@1")
    .input("retain", "core/bool@1")
    .build()
}

pub(crate) fn topic_definition(binary: bool) -> String {
    let (name, label, payload) = if binary {
        ("mqtt-binary", "On MQTT binary message", "core/bytes@1")
    } else {
        (
            "mqtt-message",
            "On MQTT message",
            "core/list@1<core/string@1>",
        )
    };
    Definition::entry(crate::PACKAGE, name, label)
        .version(2)
        .category("network", "Network")
        .uses(crate::MQTT_ADAPTER)
        .input_default(
            "topic",
            "core/string@1",
            serde_json::json!({
                "type": "core/string@1", "value": ""
            }),
        )
        .payload(payload)
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
            persistent: false,
            will: None,
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
        // A kept session with a retained will: no clean-session flag, the will flag
        // with its quality of service and retain, and the will after the client ID.
        let mut with_will = settings();
        with_will.persistent = true;
        with_will.will = Some(Outgoing {
            topic: "w".into(),
            payload: b"x".to_vec(),
            retain: true,
        });
        assert_eq!(
            connect_packet(&with_will),
            [
                0x10, 25, 0, 4, b'M', b'Q', b'T', b'T', 4, 0xec, 0, 60, 0, 1, b'c', 0, 1, b'w', 0,
                1, b'x', 0, 1, b'u', 0, 1, b'p'
            ]
        );
        assert_eq!(answer(0x62, 513), [0x62, 2, 2, 1]);
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
            Some(("t", &[9u8; 300][..], Some((513, 1))))
        );
        assert_eq!(
            parse_publish(0x34, body),
            Some(("t", &[9u8; 300][..], Some((513, 2))))
        );
        // A quality of service that does not exist and an oversized or malformed
        // length are refused.
        assert_eq!(parse_publish(0x36, body), None);
        assert_eq!(parse(&[0x30, 0xff, 0xff, 0xff, 0xff, 0x01]), Err(()));
        assert_eq!(parse(&[0x30, 0x80, 0x80, 0x80, 0x08]), Err(()));
    }

    #[test]
    fn a_received_topic_with_wildcards_or_no_name_is_a_protocol_error() {
        for topic in ["", "a/+", "#", "a/#", "nul\0l"] {
            let mut body = Vec::new();
            text(&mut body, topic.as_bytes());
            body.extend_from_slice(b"payload");
            assert_eq!(parse_publish(0x30, &body), None, "{topic:?}");
        }
        let mut body = Vec::new();
        text(&mut body, b"$SYS/a b");
        assert!(parse_publish(0x30, &body).is_some());
    }

    /// Broker bytes are untrusted: random and corrupted input splits into packets with
    /// consistent lengths or is refused, never a panic.
    #[test]
    fn corrupted_broker_bytes_are_parsed_or_refused() {
        let message = Outgoing {
            topic: "a/b".into(),
            payload: vec![7; 200],
            retain: false,
        };
        let valid = publish_packet(&message, 2, 9, false);
        let mut state = 0x2545_F491_4F6C_DD1D_u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            usize::try_from(state % (1 << 31)).unwrap()
        };
        for _ in 0..20_000 {
            let mut bytes = valid.clone();
            for _ in 0..=next() % 4 {
                let at = next() % bytes.len().max(1);
                let byte = u8::try_from(next() % 256).unwrap();
                match next() % 3 {
                    0 if !bytes.is_empty() => bytes[at] = byte,
                    1 => bytes.insert(at.min(bytes.len()), byte),
                    _ => bytes.truncate(at),
                }
            }
            if let Ok(Some((first, body, length))) = parse(&bytes) {
                assert!(length <= bytes.len() && body.len() < length);
                if let Some((topic, payload, _)) = parse_publish(first, body) {
                    assert!(is_topic(topic, false));
                    assert!(payload.len() < body.len());
                }
            }
        }
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
        assert!(
            open(json(
                r#"{"url": "mqtt://127.0.0.1:9", "qos": 2, "persistent": true, "client_id": "c",
                    "will": {"topic": "state/c", "message": "gone", "retain": true}}"#
            ))
            .is_ok()
        );
        for bad in [
            r#"{}"#,
            r#"{"url": "http://broker"}"#,
            r#"{"url": "mqtt://"}"#,
            r#"{"url": "mqtt://host:notaport"}"#,
            r#"{"url": "mqtt://host", "qos": 3}"#,
            r#"{"url": "mqtt://host", "persistent": true}"#,
            r#"{"url": "mqtt://host", "persistent": "yes", "client_id": "c"}"#,
            r#"{"url": "mqtt://host", "will": "gone"}"#,
            r#"{"url": "mqtt://host", "will": {"topic": "a/#"}}"#,
            r#"{"url": "mqtt://host", "will": {"topic": "a", "message": 1}}"#,
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
        assert_eq!(MqttAdapter.fields()[0].max, 2.0);
    }

    #[test]
    fn wired_topic_filters_are_routed_and_removed_with_their_guard() {
        let session = MqttAdapter
            .open(
                serde_json::json!({"url": "mqtt://localhost", "topics": ["default/#"]})
                    .as_object()
                    .unwrap(),
            )
            .unwrap();
        let shared = super::session(Some(session.as_ref())).unwrap();
        let payloads = Arc::new(Mutex::new(Vec::new()));
        let collect = |payloads: &Arc<Mutex<Vec<Value>>>| {
            let payloads = Arc::clone(payloads);
            Emitter::new(move |payload| {
                payloads.lock().unwrap().extend(payload);
                Ok(())
            })
        };
        let source = MqttMessage { binary: true };
        let default = source
            .start(Some(Arc::clone(&session)), collect(&payloads))
            .unwrap();
        let listen = |filter: &str| {
            source.start_with_arguments(
                Some(Arc::clone(&session)),
                &Map::new(),
                &Arguments::new(vec![(
                    "topic".into(),
                    Value::Str(Arc::new(filter.to_owned())),
                )]),
                collect(&payloads),
            )
        };
        let guard = listen("rooms/+/temp").unwrap();
        assert!(listen("rooms/#/temp").is_err());
        assert_eq!(
            shared.topics(),
            BTreeSet::from(["default/#".into(), "rooms/+/temp".into()])
        );
        assert!(matches_filter("rooms/#", "rooms"));
        assert!(!matches_filter("#", "$SYS/broker"));
        shared.deliver("rooms/bedroom/temp", &[1]);
        shared.deliver("rooms/bedroom/humidity", &[2]);
        shared.deliver("default/info", &[3]);
        drop(guard);
        let _new = listen("rooms/+/humidity").unwrap();
        shared.deliver("rooms/bedroom/temp", &[4]);
        shared.deliver("rooms/bedroom/humidity", &[5]);
        assert_eq!(
            *payloads.lock().unwrap(),
            [1, 3, 5].map(|byte| Value::Bytes(Arc::new(vec![byte]))),
        );
        assert_eq!(
            shared.topics(),
            BTreeSet::from(["default/#".into(), "rooms/+/humidity".into()])
        );
        drop(default);
    }
}
