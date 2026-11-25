//! The AMQP 0-9-1 adapter with publisher confirms and acknowledged deliveries.
//!
//! Publisher workers and consumers each own a connection and channel. Socket I/O
//! stays off the graph thread. Dropping a source guard ends that consumer without
//! detaching publish requests or consumers from other flows sharing a session.

mod wire;

use crate::transport::Authorities;
use crate::{PACKAGE, fault, secret_setting, text_setting};
use orange_sdk::{
    Arguments, Call, ConnectionAdapter, Definition, EmitError, Emitter, KernelFault, Lifecycle,
    RecordRepr, ServiceNode, ServiceSource, Session, SettingField, SettingText, Value,
};
use serde_json::{Map, Value as Json, json};
use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender, TrySendError};
use std::thread;
use std::time::{Duration, Instant};
use wire::{Connection, Delivery, Failure};

const MAX_MESSAGE_BYTES: usize = 8 * 1024 * 1024;
const MAX_QUEUED: usize = 16;
const MAX_BINDINGS: usize = 64;
const MAX_CONSUMERS: usize = 64;
const POLL: Duration = Duration::from_millis(50);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const PUBLISH_TIMEOUT: Duration = Duration::from_secs(15);
const FIRST_RETRY: Duration = Duration::from_millis(250);
const MAX_RETRY: Duration = Duration::from_secs(5);

const NO_SESSION: KernelFault = fault(
    "org.orange.net/Connection",
    "org.orange.net/connection-kind",
    "The node needs an AMQP connection.",
);
const BAD_MESSAGE: KernelFault = fault(
    "org.orange.net/Request",
    "org.orange.net/amqp-message-invalid",
    "The routing key or message is invalid or too large.",
);
const QUEUE_FULL: KernelFault = fault(
    "org.orange.net/Busy",
    "org.orange.net/send-queue-full",
    "The connection has too many messages waiting to be sent.",
);
const DEADLINE: KernelFault = fault(
    "org.orange.net/Timeout",
    "org.orange.net/amqp-confirm-timeout",
    "The AMQP publish did not receive a confirmation by its deadline.",
);
const CONNECTION_LOST: KernelFault = fault(
    "org.orange.net/Connection",
    "org.orange.net/amqp-confirm-connection",
    "The AMQP connection ended before the publish was confirmed.",
);
const REJECTED: KernelFault = fault(
    "org.orange.net/Request",
    "org.orange.net/amqp-publish-rejected",
    "The AMQP broker rejected the publish.",
);
const UNROUTABLE: KernelFault = fault(
    "org.orange.net/Request",
    "org.orange.net/amqp-publish-unroutable",
    "The AMQP exchange could not route the message to a queue.",
);
const CONSUME_REFUSED: KernelFault = fault(
    "org.orange.net/Connection",
    "org.orange.net/amqp-consume-refused",
    "The AMQP broker refused the consumer or sent an invalid delivery.",
);
const CONSUME_TOO_LARGE: KernelFault = fault(
    "org.orange.net/Request",
    "org.orange.net/amqp-delivery-too-large",
    "The AMQP delivery exceeded the maximum message size.",
);

const TEXT_TYPE: &str = "org.orange.net/amqp-text-delivery@1";
const BYTES_TYPE: &str = "org.orange.net/amqp-binary-delivery@1";
pub(crate) const TEXT_FIELDS: [(&str, &str); 4] = [
    ("exchange", "core/string@1"),
    ("routing_key", "core/string@1"),
    ("redelivered", "core/bool@1"),
    ("text", "core/string@1"),
];
pub(crate) const BYTES_FIELDS: [(&str, &str); 4] = [
    ("exchange", "core/string@1"),
    ("routing_key", "core/string@1"),
    ("redelivered", "core/bool@1"),
    ("data", "core/bytes@1"),
];

struct Settings {
    host: String,
    port: u16,
    tls: bool,
    authorities: Option<Authorities>,
    vhost: String,
    username: String,
    password: String,
    exchange: String,
    queue: String,
    routing_key: String,
    binding_keys: Vec<String>,
    prefetch: u16,
    heartbeat: u16,
}
impl Drop for Settings {
    fn drop(&mut self) {
        use zeroize::Zeroize as _;
        self.password.zeroize();
    }
}

fn short_text(text: &str) -> bool {
    text.len() <= usize::from(u8::MAX) && !text.chars().any(char::is_control)
}

fn decode_path(path: &str) -> Option<String> {
    let hex = |byte: u8| char::from(byte).to_digit(16).map(|nibble| nibble as u8);
    let mut decoded = Vec::new();
    let mut bytes = path.bytes();
    while let Some(byte) = bytes.next() {
        decoded.push(if byte == b'%' {
            (hex(bytes.next()?)? << 4) | hex(bytes.next()?)?
        } else {
            byte
        });
    }
    String::from_utf8(decoded)
        .ok()
        .filter(|value| short_text(value))
}

fn authority(authority: &str, default_port: u16) -> Option<(String, u16)> {
    if authority.contains(['@', '?', '#', '\\'])
        || authority
            .chars()
            .any(|ch| ch.is_whitespace() || ch.is_control())
    {
        return None;
    }
    let (host, port) = if let Some(ipv6) = authority.strip_prefix('[') {
        let (host, tail) = ipv6.split_once(']')?;
        let _ = host.parse::<std::net::Ipv6Addr>().ok()?;
        let port = if tail.is_empty() {
            default_port
        } else {
            tail.strip_prefix(':')?.parse().ok()?
        };
        (host, port)
    } else if let Some((host, port)) = authority.split_once(':') {
        (host, port.parse().ok()?)
    } else {
        (authority, default_port)
    };
    (!host.is_empty() && port != 0).then(|| (host.to_owned(), port))
}

fn string<'a>(settings: &'a Map<String, Json>, key: &str) -> Result<Option<&'a str>, String> {
    match settings.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::String(value)) => Ok(Some(value)),
        Some(_) => Err(format!("`{key}` must be text")),
    }
}

fn short_setting(settings: &Map<String, Json>, key: &str, default: &str) -> Result<String, String> {
    let value = string(settings, key)?.unwrap_or(default);
    if !short_text(value) {
        return Err(format!("`{key}` is not valid AMQP text"));
    }
    Ok(value.to_owned())
}

fn number(
    settings: &Map<String, Json>,
    key: &str,
    default: u16,
    range: std::ops::RangeInclusive<u16>,
) -> Result<u16, String> {
    match settings.get(key) {
        None => Ok(default),
        Some(value) => value
            .as_u64()
            .and_then(|n| u16::try_from(n).ok())
            .filter(|n| range.contains(n))
            .ok_or_else(|| {
                format!(
                    "`{key}` must be an integer from {} to {}",
                    range.start(),
                    range.end()
                )
            }),
    }
}

impl Settings {
    fn parse(settings: &Map<String, Json>) -> Result<Self, String> {
        let url = string(settings, "url")?.ok_or("`url` is required")?;
        let (tls, address) = if let Some(rest) = url.strip_prefix("amqps://") {
            (true, rest)
        } else if let Some(rest) = url.strip_prefix("amqp://") {
            (false, rest)
        } else {
            return Err("`url` must start with amqp:// or amqps://".into());
        };
        let (server, path) = address.split_once('/').unwrap_or((address, ""));
        let (host, port) = authority(server, if tls { 5671 } else { 5672 })
            .ok_or("`url` needs a valid host and port without credentials")?;
        if path.contains(['/', '?', '#']) {
            return Err("`url` needs a single percent-encoded virtual host segment".into());
        }
        let path = if path.is_empty() {
            "/".to_owned()
        } else {
            decode_path(path).ok_or("`url` has an invalid virtual host")?
        };
        let vhost = short_setting(settings, "vhost", &path)?;
        if vhost.is_empty() {
            return Err("`vhost` cannot be empty".into());
        }
        let (username, password) =
            match (string(settings, "username")?, string(settings, "password")?) {
                (None, None) => ("guest", "guest"),
                (Some(username), Some(password))
                    if !username.is_empty()
                        && short_text(username)
                        && password.len() <= 2048
                        && !password.contains('\0') =>
                {
                    (username, password)
                }
                _ => return Err("give a valid `username` and `password` together".into()),
            };
        let authorities = match string(settings, "ca_file")? {
            None | Some("") => None,
            Some(path) if tls => Some(crate::transport::load_authorities(path)?),
            Some(_) => return Err("`ca_file` needs an amqps:// URL".into()),
        };
        let exchange = short_setting(settings, "exchange", "")?;
        let binding_keys = match settings.get("binding_keys") {
            None => Vec::new(),
            Some(Json::Array(keys)) if keys.len() <= MAX_BINDINGS => keys
                .iter()
                .map(|key| {
                    key.as_str()
                        .filter(|key| short_text(key))
                        .map(str::to_owned)
                        .ok_or("`binding_keys` must contain valid routing keys")
                })
                .collect::<Result<Vec<_>, _>>()?,
            Some(_) => {
                return Err("`binding_keys` must be a list of at most 64 routing keys".into());
            }
        };
        if !binding_keys.is_empty() && exchange.is_empty() {
            return Err("`binding_keys` needs a named exchange".into());
        }

        Ok(Self {
            host,
            port,
            tls,
            authorities,
            vhost,
            username: username.to_owned(),
            password: password.to_owned(),
            exchange,
            queue: short_setting(settings, "queue", "")?,
            routing_key: short_setting(settings, "routing_key", "")?,
            binding_keys,
            prefetch: number(settings, "prefetch", 16, 1..=256)?,
            heartbeat: number(settings, "heartbeat_s", 30, 5..=300)?,
        })
    }
}

#[derive(Debug)]
pub(crate) struct AmqpAdapter;
impl ConnectionAdapter for AmqpAdapter {
    fn open(&self, settings: &Map<String, Json>) -> Result<Arc<dyn Session>, String> {
        let settings = Arc::new(Settings::parse(settings)?);
        let life = Arc::new(Lifecycle::new());
        let (requests, receive) = mpsc::sync_channel(MAX_QUEUED);
        let (worker_settings, worker_life) = (Arc::clone(&settings), Arc::clone(&life));
        thread::Builder::new()
            .name("orange-net-amqp-publish".into())
            .spawn(move || publisher(&worker_settings, &worker_life, &receive))
            .map_err(|_| "the AMQP publisher worker could not start")?;
        Ok(Arc::new(AmqpSession {
            settings,
            requests,
            life,
            consumers: Arc::new(AtomicUsize::new(0)),
        }))
    }

    fn about(&self) -> orange_sdk::AdapterAbout {
        orange_sdk::AdapterAbout {
            label: "AMQP 0-9-1 broker".into(),
            purpose: "Publishes confirmed messages and receives deliveries from a queue.".into(),
        }
    }

    fn texts(&self) -> Vec<SettingText> {
        vec![
            SettingText {
                required: true,
                ..text_setting(
                    "url",
                    "Broker",
                    "Use an amqp:// or amqps:// endpoint.",
                    "amqps://broker.example.com",
                )
            },
            text_setting(
                "vhost",
                "Virtual host",
                "Overrides the URL's virtual host, which defaults to /.",
                "/",
            ),
            text_setting(
                "username",
                "User name",
                "The user name the broker knows.",
                "",
            ),
            secret_setting("password", "Password", "The broker password."),
            text_setting(
                "exchange",
                "Exchange",
                "The broker's default direct exchange when left empty.",
                "",
            ),
            text_setting(
                "queue",
                "Queue",
                "The queue used when the trigger has no queue input.",
                "",
            ),
            text_setting(
                "routing_key",
                "Routing key",
                "The routing key used when the publish node has no routing key input.",
                "",
            ),
            SettingText {
                list: true,
                ..text_setting(
                    "binding_keys",
                    "Binding keys",
                    "Optional keys for binding the queue to the named exchange, one per line.",
                    "",
                )
            },
            text_setting(
                "ca_file",
                "Certificate authority file",
                "A PEM file of the authorities to trust for TLS.",
                "",
            ),
        ]
    }

    fn fields(&self) -> Vec<SettingField> {
        vec![
            SettingField {
                setting: "prefetch".into(),
                label: "Prefetch".into(),
                help: "Maximum unacknowledged deliveries per consumer.".into(),
                unit: String::new(),
                min: 1.0,
                max: 256.0,
                default: Some(16.0),
                whole: true,
            },
            SettingField {
                setting: "heartbeat_s".into(),
                label: "Heartbeat".into(),
                help: "Maximum heartbeat interval offered to the broker.".into(),
                unit: "s".into(),
                min: 5.0,
                max: 300.0,
                default: Some(30.0),
                whole: true,
            },
        ]
    }
}

struct AmqpSession {
    settings: Arc<Settings>,
    requests: SyncSender<Publish>,
    life: Arc<Lifecycle>,
    consumers: Arc<AtomicUsize>,
}

impl std::fmt::Debug for AmqpSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AmqpSession").finish_non_exhaustive()
    }
}

impl Session for AmqpSession {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn start(&self) {
        self.life.start();
    }
}

impl Drop for AmqpSession {
    fn drop(&mut self) {
        self.life.stop();
    }
}

fn session(session: Option<&dyn Session>) -> Option<&AmqpSession> {
    session?.as_any().downcast_ref()
}

struct Publish {
    key: String,
    payload: Vec<u8>,
    deadline: Instant,
    active: Arc<AtomicBool>,
    result: Sender<Result<(), KernelFault>>,
}

impl Publish {
    fn active(&self) -> bool {
        self.active.load(Ordering::Acquire) && Instant::now() < self.deadline
    }
}

struct CancelPublish(Arc<AtomicBool>);
impl Drop for CancelPublish {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn publish_fault(error: Failure) -> KernelFault {
    match error {
        Failure::Transport | Failure::Protocol => CONNECTION_LOST,
        Failure::Rejected => REJECTED,
        Failure::Unroutable => UNROUTABLE,
        Failure::OversizedDelivery => BAD_MESSAGE,
        Failure::Deadline => DEADLINE,
    }
}

fn publisher(settings: &Settings, life: &Lifecycle, receive: &Receiver<Publish>) {
    if !life.wait_started() {
        return;
    }
    let mut connection: Option<Connection> = None;
    let mut backoff = FIRST_RETRY;
    while !life.stopped() {
        let publish = match receive.recv_timeout(POLL) {
            Ok(publish) => publish,
            Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {
                if let Some(conn) = &mut connection
                    && conn.idle(&|| !life.stopped()).is_err()
                {
                    connection = None;
                }
                continue;
            }
        };
        let active = || publish.active() && !life.stopped();
        let result = loop {
            if !active() {
                break Err(DEADLINE);
            }
            if let Some(conn) = &mut connection {
                let result = conn.publish(
                    &settings.exchange,
                    &publish.key,
                    &publish.payload,
                    publish.deadline,
                    &active,
                );
                if let Err(error) = result {
                    connection = None;
                    break Err(publish_fault(error));
                }
                break Ok(());
            }
            let deadline = (Instant::now() + CONNECT_TIMEOUT).min(publish.deadline);
            let connected = Connection::open(settings, deadline, &active).and_then(|mut conn| {
                conn.confirms(deadline, &active)?;
                Ok(conn)
            });
            match connected {
                Ok(conn) => {
                    connection = Some(conn);
                    backoff = FIRST_RETRY;
                }
                Err(Failure::Transport | Failure::Deadline) => {
                    let wait =
                        backoff.min(publish.deadline.saturating_duration_since(Instant::now()));
                    let _ = life.rest(wait);
                    backoff = (backoff * 2).min(MAX_RETRY);
                }
                Err(error) => break Err(publish_fault(error)),
            }
        };
        let _ = publish.result.send(result);
    }
}

#[derive(Debug)]
pub(crate) struct AmqpPublish {
    pub binary: bool,
}
impl ServiceNode for AmqpPublish {
    fn run(&self, call: &Call<'_>) -> Result<Vec<Value>, KernelFault> {
        let session = session(call.session).ok_or(NO_SESSION)?;
        let (key, payload): (&str, &[u8]) = match (self.binary, call.inputs) {
            (true, [Value::Str(key), Value::Bytes(body)]) => (key, body),
            (false, [Value::Str(key), Value::Str(body)]) => (key, body.as_bytes()),
            _ => return Err(BAD_MESSAGE),
        };
        let key = if key.is_empty() {
            &session.settings.routing_key
        } else {
            key
        };
        if !short_text(key) || payload.len() > MAX_MESSAGE_BYTES {
            return Err(BAD_MESSAGE);
        }
        let (result, receive) = mpsc::channel();
        let deadline = Instant::now() + PUBLISH_TIMEOUT;
        let active = CancelPublish(Arc::new(AtomicBool::new(true)));
        session
            .requests
            .try_send(Publish {
                key: key.to_owned(),
                payload: payload.to_vec(),
                deadline,
                active: Arc::clone(&active.0),
                result,
            })
            .map_err(|error| match error {
                TrySendError::Full(_) => QUEUE_FULL,
                TrySendError::Disconnected(_) => CONNECTION_LOST,
            })?;
        while !call.lease.is_cancelled() && Instant::now() < deadline {
            match receive.recv_timeout(POLL) {
                Ok(result) => return result.map(|()| Vec::new()),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Err(CONNECTION_LOST),
            }
        }
        Err(DEADLINE)
    }
}

enum ConsumerResult {
    Reconnect,
    Closed,
}

fn consume(
    settings: &Settings,
    queue: &str,
    binary: bool,
    emitter: &Emitter,
    life: &Lifecycle,
) -> Result<ConsumerResult, Failure> {
    let active = || !life.stopped();
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    let mut conn = Connection::open(settings, deadline, &active)?;
    conn.consume(settings, queue, deadline, &active)?;
    while active() {
        let Some(delivery) = conn.delivery(&active)? else {
            continue;
        };
        let tag = delivery.tag;
        if !active() {
            let _ = conn.reject(tag, true);
            return Ok(ConsumerResult::Closed);
        }
        let Some(payload) = delivery_value(delivery, binary) else {
            // A text consumer rejects a non-UTF-8 body without draining the queue
            // into a permanent requeue loop. The broker may dead-letter it.
            conn.reject(tag, false)?;
            continue;
        };
        match emitter.emit(Some(payload)) {
            Ok(()) => conn.acknowledge(tag)?,
            Err(EmitError::Busy) => {
                conn.reject(tag, true)?;
                return Ok(ConsumerResult::Reconnect);
            }
            Err(EmitError::Closed) => {
                let _ = conn.reject(tag, true);
                return Ok(ConsumerResult::Closed);
            }
        }
    }
    Ok(ConsumerResult::Closed)
}

fn delivery_value(delivery: Delivery, binary: bool) -> Option<Value> {
    let (ty, body) = if binary {
        (BYTES_TYPE, Value::Bytes(Arc::new(delivery.body)))
    } else {
        (
            TEXT_TYPE,
            Value::Str(Arc::new(String::from_utf8(delivery.body).ok()?)),
        )
    };
    Some(Value::Record(Arc::new(RecordRepr {
        ty: ty.parse().ok()?,
        fields: Box::new([
            Some(Value::Str(Arc::new(delivery.exchange))),
            Some(Value::Str(Arc::new(delivery.routing_key))),
            Some(Value::Bool(delivery.redelivered)),
            Some(body),
        ]),
    })))
}

fn consumer(settings: &Settings, queue: &str, binary: bool, emitter: &Emitter, life: &Lifecycle) {
    let mut backoff = FIRST_RETRY;
    while !life.stopped() {
        let opened = Instant::now();
        match consume(settings, queue, binary, emitter, life) {
            Ok(ConsumerResult::Closed) => return,
            Err(Failure::Protocol | Failure::Rejected | Failure::Unroutable) => {
                let _ = emitter.fault(CONSUME_REFUSED);
                return;
            }
            Err(Failure::OversizedDelivery) => {
                let _ = emitter.fault(CONSUME_TOO_LARGE);
                return;
            }
            Ok(ConsumerResult::Reconnect) | Err(Failure::Deadline | Failure::Transport) => {}
        }
        if opened.elapsed() >= MAX_RETRY {
            backoff = FIRST_RETRY;
        }
        let _ = life.rest(backoff);
        backoff = (backoff * 2).min(MAX_RETRY);
    }
}

struct ConsumerGuard(Arc<Lifecycle>);
impl Drop for ConsumerGuard {
    fn drop(&mut self) {
        self.0.stop();
    }
}

struct ConsumerPermit(Arc<AtomicUsize>);
impl ConsumerPermit {
    fn acquire(count: &Arc<AtomicUsize>) -> Result<Self, String> {
        count
            .try_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                (current < MAX_CONSUMERS).then_some(current + 1)
            })
            .map_err(|_| "the AMQP connection already has its maximum number of consumers")?;
        Ok(Self(Arc::clone(count)))
    }
}
impl Drop for ConsumerPermit {
    fn drop(&mut self) {
        let _ = self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Debug)]
pub(crate) struct AmqpConsumer {
    pub binary: bool,
}
impl ServiceSource for AmqpConsumer {
    fn start_with_arguments(
        &self,
        session_ref: Option<Arc<dyn Session>>,
        _settings: &Map<String, Json>,
        arguments: &Arguments,
        emitter: Emitter,
    ) -> Result<Box<dyn Send>, String> {
        let session =
            session(session_ref.as_deref()).ok_or("the entry needs an AMQP connection")?;
        let settings = Arc::clone(&session.settings);
        let queue: &str = match arguments.get("queue") {
            Some(Value::Str(queue)) if !queue.is_empty() => queue,
            None | Some(Value::Str(_)) => &settings.queue,
            _ => return Err("the entry needs a queue name".into()),
        };
        if queue.is_empty() || !short_text(queue) {
            return Err("the entry needs a valid queue name".into());
        }
        let queue = queue.to_owned();
        let permit = ConsumerPermit::acquire(&session.consumers)?;
        let life = Arc::new(Lifecycle::new());
        let guard = ConsumerGuard(Arc::clone(&life));
        let binary = self.binary;
        thread::Builder::new()
            .name("orange-net-amqp-consume".into())
            .spawn(move || {
                let _permit = permit;
                consumer(&settings, &queue, binary, &emitter, &life);
            })
            .map_err(|_| "the AMQP consumer worker could not start")?;
        Ok(Box::new(guard))
    }
}

fn empty_text() -> Json {
    json!({"type": "core/string@1", "value": ""})
}

pub(crate) fn publish_definition(binary: bool) -> String {
    let (name, label, port, ty) = if binary {
        (
            "amqp-publish-bytes",
            "Publish AMQP binary message",
            "data",
            "core/bytes@1",
        )
    } else {
        (
            "amqp-publish",
            "Publish AMQP message",
            "text",
            "core/string@1",
        )
    };
    Definition::node(PACKAGE, name, label)
        .category("network", "Network")
        .uses(crate::AMQP_ADAPTER)
        .then_label("Confirmed")
        .input_default("routing_key", "core/string@1", empty_text())
        .input(port, ty)
        .build()
}

pub(crate) fn consumer_definition(binary: bool) -> String {
    let (name, label, ty) = if binary {
        ("amqp-binary", "On AMQP binary message", BYTES_TYPE)
    } else {
        ("amqp-message", "On AMQP message", TEXT_TYPE)
    };
    Definition::entry(PACKAGE, name, label)
        .category("network", "Network")
        .uses(crate::AMQP_ADAPTER)
        .input_default("queue", "core/string@1", empty_text())
        .payload(ty)
        .build()
}
