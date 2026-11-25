//! Bounded AMQP 0-9-1 framing and the protocol methods used by the adapter.

use super::{CONNECT_TIMEOUT, MAX_MESSAGE_BYTES, POLL, Settings};
use crate::transport::{self, Stream};
use std::io::{ErrorKind, Read, Write};
use std::time::{Duration, Instant};

const METHOD: u8 = 1;
const HEADER: u8 = 2;
const BODY: u8 = 3;
const HEARTBEAT: u8 = 8;
const END: u8 = 0xce;
const MIN_FRAME: usize = 4096;
const MAX_FRAME: usize = 128 * 1024;
const CHANNEL: u16 = 1;
const PROTOCOL: &[u8; 8] = b"AMQP\0\0\x09\x01";

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Failure {
    Transport,
    Protocol,
    Rejected,
    Unroutable,
    OversizedDelivery,
    Deadline,
}

type Result<T> = std::result::Result<T, Failure>;

#[derive(Debug)]
struct Frame {
    kind: u8,
    channel: u16,
    data: Vec<u8>,
}

fn take<'a>(bytes: &mut &'a [u8], count: usize) -> Result<&'a [u8]> {
    let (value, tail) = bytes.split_at_checked(count).ok_or(Failure::Protocol)?;
    *bytes = tail;
    Ok(value)
}

fn word<const N: usize>(bytes: &mut &[u8]) -> Result<[u8; N]> {
    take(bytes, N)?.try_into().map_err(|_| Failure::Protocol)
}

fn read_short<'a>(bytes: &mut &'a [u8]) -> Result<&'a str> {
    let [count] = word(bytes)?;
    std::str::from_utf8(take(bytes, usize::from(count))?).map_err(|_| Failure::Protocol)
}

fn read_long<'a>(bytes: &mut &'a [u8]) -> Result<&'a [u8]> {
    let count = u32::from_be_bytes(word(bytes)?) as usize;
    take(bytes, count)
}

fn short(out: &mut Vec<u8>, text: &str) -> Result<()> {
    out.push(u8::try_from(text.len()).map_err(|_| Failure::Protocol)?);
    out.extend_from_slice(text.as_bytes());
    Ok(())
}

fn long(out: &mut Vec<u8>, bytes: &[u8]) -> Result<()> {
    let size = u32::try_from(bytes.len()).map_err(|_| Failure::Protocol)?;
    out.extend_from_slice(&size.to_be_bytes());
    out.extend_from_slice(bytes);
    Ok(())
}

fn parse_frame(bytes: &[u8], frame_max: usize) -> Result<Option<(Frame, usize)>> {
    if bytes.len() < 7 {
        return Ok(None);
    }
    let mut fields = bytes;
    let [kind] = word(&mut fields)?;
    let channel = u16::from_be_bytes(word(&mut fields)?);
    let size = u32::from_be_bytes(word(&mut fields)?) as usize;
    if size > frame_max - 8 {
        return Err(Failure::Protocol);
    }
    let consumed = size + 8;
    if bytes.len() < consumed {
        return Ok(None);
    }
    let (data, end) = fields.split_at(size);
    if end.first() != Some(&END) {
        return Err(Failure::Protocol);
    }
    Ok(Some((
        Frame {
            kind,
            channel,
            data: data.to_vec(),
        },
        consumed,
    )))
}

struct Reader {
    bytes: Vec<u8>,
    offset: usize,
}
impl Reader {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            offset: 0,
        }
    }

    fn next(&mut self, frame_max: usize) -> Result<Option<Frame>> {
        let Some((frame, used)) = parse_frame(&self.bytes[self.offset..], frame_max)? else {
            if self.offset != 0 {
                let _ = self.bytes.drain(..self.offset);
                self.offset = 0;
            }
            return Ok(None);
        };
        self.offset += used;
        Ok(Some(frame))
    }
}

pub(super) struct Delivery {
    pub tag: u64,
    pub exchange: String,
    pub routing_key: String,
    pub redelivered: bool,
    pub body: Vec<u8>,
}

pub(super) struct Connection<S = Stream> {
    stream: S,
    reader: Reader,
    frame_max: usize,
    heartbeat: Duration,
    last_received: Instant,
    last_sent: Instant,
    publish_sequence: u64,
}

impl Connection<Stream> {
    pub(super) fn open(
        settings: &Settings,
        deadline: Instant,
        active: &impl Fn() -> bool,
    ) -> Result<Self> {
        let timeout = deadline
            .saturating_duration_since(Instant::now())
            .min(CONNECT_TIMEOUT);
        let stream = transport::open(
            &settings.host,
            settings.port,
            settings.tls,
            settings.authorities.as_ref(),
            POLL,
            timeout,
        )
        .ok_or(Failure::Transport)?;
        Self::handshake(stream, settings, deadline, active)
    }
}

impl<S: Read + Write> Connection<S> {
    fn handshake(
        stream: S,
        settings: &Settings,
        deadline: Instant,
        active: &impl Fn() -> bool,
    ) -> Result<Self> {
        let now = Instant::now();
        let mut conn = Self {
            stream,
            reader: Reader::new(),
            frame_max: MAX_FRAME,
            heartbeat: Duration::ZERO,
            last_received: now,
            last_sent: now,
            publish_sequence: 0,
        };
        conn.write(PROTOCOL)?;
        let start = conn.expect(0, 10, 10, deadline, active)?;
        let mut fields = start.as_slice();
        if word::<2>(&mut fields)? != [0, 9] {
            return Err(Failure::Protocol);
        }
        let _ = read_long(&mut fields)?; // server properties
        let sasl = read_long(&mut fields)?;
        let locales = read_long(&mut fields)?;
        if !sasl
            .split(|byte| *byte == b' ')
            .any(|mechanism| mechanism == b"PLAIN")
            || !locales
                .split(|byte| *byte == b' ')
                .any(|locale| locale == b"en_US")
        {
            return Err(Failure::Protocol);
        }
        let mut start_ok = vec![0; 4]; // empty client properties table
        short(&mut start_ok, "PLAIN")?;
        let response = format!("\0{}\0{}", settings.username, settings.password);
        long(&mut start_ok, response.as_bytes())?;
        short(&mut start_ok, "en_US")?;
        conn.method(0, 10, 11, &start_ok)?;

        let tune = conn.expect(0, 10, 30, deadline, active)?;
        let mut fields = tune.as_slice();
        let _ = word::<2>(&mut fields)?; // channel_max, at least one channel is required
        let offered = u32::from_be_bytes(word(&mut fields)?) as usize;
        if offered != 0 && offered < MIN_FRAME {
            return Err(Failure::Protocol);
        }
        conn.frame_max = if offered == 0 {
            MAX_FRAME
        } else {
            offered.min(MAX_FRAME)
        };
        let heartbeat = u16::from_be_bytes(word(&mut fields)?);
        let heartbeat = if heartbeat == 0 {
            settings.heartbeat
        } else {
            heartbeat.min(settings.heartbeat)
        };
        conn.heartbeat = Duration::from_secs(u64::from(heartbeat));
        let mut tune_ok = CHANNEL.to_be_bytes().to_vec();
        tune_ok.extend_from_slice(&(conn.frame_max as u32).to_be_bytes());
        tune_ok.extend_from_slice(&heartbeat.to_be_bytes());
        conn.method(0, 10, 31, &tune_ok)?;
        let mut open = Vec::new();
        short(&mut open, &settings.vhost)?;
        open.extend_from_slice(&[0, 0]); // reserved capabilities short string and insist bit
        conn.method(0, 10, 40, &open)?;
        let _ = conn.expect(0, 10, 41, deadline, active)?;
        conn.method(CHANNEL, 20, 10, &[0])?; // channel.open
        let _ = conn.expect(CHANNEL, 20, 11, deadline, active)?;
        Ok(conn)
    }

    fn write(&mut self, bytes: &[u8]) -> Result<()> {
        self.stream
            .write_all(bytes)
            .and_then(|()| self.stream.flush())
            .map_err(|_| Failure::Transport)?;
        self.last_sent = Instant::now();
        Ok(())
    }

    fn frame(&mut self, kind: u8, channel: u16, bytes: &[u8]) -> Result<()> {
        if bytes.len() > self.frame_max - 8 {
            return Err(Failure::Protocol);
        }
        let mut frame = Vec::with_capacity(bytes.len() + 8);
        frame.push(kind);
        frame.extend_from_slice(&channel.to_be_bytes());
        frame.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        frame.extend_from_slice(bytes);
        frame.push(END);
        self.write(&frame)
    }

    fn method(&mut self, channel: u16, class: u16, id: u16, args: &[u8]) -> Result<()> {
        let mut bytes = Vec::with_capacity(args.len() + 4);
        bytes.extend_from_slice(&class.to_be_bytes());
        bytes.extend_from_slice(&id.to_be_bytes());
        bytes.extend_from_slice(args);
        self.frame(METHOD, channel, &bytes)
    }

    fn receive(&mut self, deadline: Instant, active: &impl Fn() -> bool) -> Result<Option<Frame>> {
        let mut chunk = [0; 16 * 1024];
        while active() && Instant::now() < deadline {
            if let Some(frame) = self.reader.next(self.frame_max)? {
                if frame.kind == HEARTBEAT && frame.channel == 0 && frame.data.is_empty() {
                    continue;
                }
                if frame.kind == METHOD {
                    match frame.method_key()? {
                        (0, 10, 50) => {
                            let _ = self.method(0, 10, 51, &[]);
                            return Err(Failure::Protocol);
                        }
                        (CHANNEL, 20, 40) => {
                            let _ = self.method(CHANNEL, 20, 41, &[]);
                            return Err(Failure::Protocol);
                        }
                        _ => {}
                    }
                }
                return Ok(Some(frame));
            }
            if !self.heartbeat.is_zero() {
                if self.last_received.elapsed() > self.heartbeat * 2 {
                    return Err(Failure::Transport);
                }
                if self.last_sent.elapsed() >= self.heartbeat / 2 {
                    self.frame(HEARTBEAT, 0, &[])?;
                }
            }
            match self.stream.read(&mut chunk) {
                Ok(0) => return Err(Failure::Transport),
                Ok(count) => {
                    self.reader.bytes.extend_from_slice(&chunk[..count]);
                    self.last_received = Instant::now();
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                    ) => {}
                Err(_) => return Err(Failure::Transport),
            }
        }
        Ok(None)
    }

    fn expect(
        &mut self,
        channel: u16,
        class: u16,
        id: u16,
        deadline: Instant,
        active: &impl Fn() -> bool,
    ) -> Result<Vec<u8>> {
        let frame = self.receive(deadline, active)?.ok_or(Failure::Deadline)?;
        if frame.method_key()? != (channel, class, id) {
            return Err(Failure::Protocol);
        }
        Ok(frame.data[4..].to_vec())
    }

    pub(super) fn confirms(&mut self, deadline: Instant, active: &impl Fn() -> bool) -> Result<()> {
        // RabbitMQ's confirm.select with no-wait false.
        self.method(CHANNEL, 85, 10, &[0])?;
        let _ = self.expect(CHANNEL, 85, 11, deadline, active)?;
        Ok(())
    }

    pub(super) fn idle(&mut self, active: &impl Fn() -> bool) -> Result<()> {
        if self.receive(Instant::now() + POLL, active)?.is_some() {
            return Err(Failure::Protocol);
        }
        Ok(())
    }

    pub(super) fn publish(
        &mut self,
        exchange: &str,
        routing_key: &str,
        body: &[u8],
        deadline: Instant,
        active: &impl Fn() -> bool,
    ) -> Result<()> {
        let mut args = Vec::new();
        args.extend_from_slice(&[0, 0]); // reserved ticket
        short(&mut args, exchange)?;
        short(&mut args, routing_key)?;
        args.push(1); // mandatory: unroutable messages return before the confirm
        self.method(CHANNEL, 60, 40, &args)?;
        let mut header = vec![0, 60, 0, 0]; // class, weight
        header.extend_from_slice(&(body.len() as u64).to_be_bytes());
        header.extend_from_slice(&[0, 0]); // no Basic properties
        self.frame(HEADER, CHANNEL, &header)?;
        for chunk in body.chunks(self.frame_max - 8) {
            if !active() || Instant::now() >= deadline {
                return Err(Failure::Deadline);
            }
            self.frame(BODY, CHANNEL, chunk)?;
        }
        self.publish_sequence = self
            .publish_sequence
            .checked_add(1)
            .ok_or(Failure::Protocol)?;
        let frame = self.receive(deadline, active)?.ok_or(Failure::Deadline)?;
        match frame.method_key()? {
            (CHANNEL, 60, 50) => {
                // The caller closes this channel. A pending confirm must not be
                // mistaken for a confirm on the next publish.
                Err(Failure::Unroutable)
            }
            (CHANNEL, 60, id @ (80 | 120)) => {
                let mut fields = &frame.data[4..];
                let tag = u64::from_be_bytes(word(&mut fields)?);
                let [bits] = word(&mut fields)?;
                if tag != self.publish_sequence && !(tag == 0 && bits & 1 != 0) {
                    return Err(Failure::Protocol);
                }
                if id == 80 {
                    Ok(())
                } else {
                    Err(Failure::Rejected)
                }
            }
            _ => Err(Failure::Protocol),
        }
    }

    pub(super) fn consume(
        &mut self,
        settings: &Settings,
        queue: &str,
        deadline: Instant,
        active: &impl Fn() -> bool,
    ) -> Result<()> {
        for routing_key in &settings.binding_keys {
            let mut bind = vec![0, 0]; // reserved ticket
            short(&mut bind, queue)?;
            short(&mut bind, &settings.exchange)?;
            short(&mut bind, routing_key)?;
            bind.extend_from_slice(&[0; 5]); // no-wait false, empty arguments table
            self.method(CHANNEL, 50, 20, &bind)?;
            let _ = self.expect(CHANNEL, 50, 21, deadline, active)?;
        }
        let mut qos = vec![0; 4]; // prefetch size
        qos.extend_from_slice(&settings.prefetch.to_be_bytes());
        qos.push(0); // per consumer
        self.method(CHANNEL, 60, 10, &qos)?;
        let _ = self.expect(CHANNEL, 60, 11, deadline, active)?;
        let mut consume = vec![0, 0];
        short(&mut consume, queue)?;
        consume.extend_from_slice(&[0; 6]); // server consumer tag, flags and arguments table
        self.method(CHANNEL, 60, 20, &consume)?;
        let _ = self.expect(CHANNEL, 60, 21, deadline, active)?;
        Ok(())
    }

    pub(super) fn delivery(&mut self, active: &impl Fn() -> bool) -> Result<Option<Delivery>> {
        let Some(frame) = self.receive(Instant::now() + POLL, active)? else {
            return Ok(None);
        };
        if frame.method_key()? != (CHANNEL, 60, 60) {
            return Err(Failure::Protocol);
        }
        let mut fields = &frame.data[4..];
        let _ = read_short(&mut fields)?; // consumer tag
        let tag = u64::from_be_bytes(word(&mut fields)?);
        let [bits] = word(&mut fields)?;
        let exchange = read_short(&mut fields)?.to_owned();
        let routing_key = read_short(&mut fields)?.to_owned();
        let body = match self.read_content(active) {
            Err(Failure::OversizedDelivery) => {
                let _ = self.reject(tag, false);
                return Err(Failure::OversizedDelivery);
            }
            result => result?,
        };
        Ok(Some(Delivery {
            tag,
            exchange,
            routing_key,
            redelivered: bits & 1 != 0,
            body,
        }))
    }

    fn read_content(&mut self, active: &impl Fn() -> bool) -> Result<Vec<u8>> {
        let deadline = Instant::now() + CONNECT_TIMEOUT;
        let header = self.receive(deadline, active)?.ok_or(Failure::Deadline)?;
        let mut fields = header.data.as_slice();
        if header.channel != CHANNEL
            || header.kind != HEADER
            || word::<4>(&mut fields)? != [0, 60, 0, 0]
        {
            return Err(Failure::Protocol);
        }
        let size = usize::try_from(u64::from_be_bytes(word(&mut fields)?))
            .ok()
            .filter(|size| *size <= MAX_MESSAGE_BYTES)
            .ok_or(Failure::OversizedDelivery)?;
        let _ = word::<2>(&mut fields)?; // Basic property flags; unused properties stay opaque
        let mut body = Vec::new();
        while body.len() < size {
            let frame = self.receive(deadline, active)?.ok_or(Failure::Deadline)?;
            if frame.channel != CHANNEL
                || frame.kind != BODY
                || frame.data.is_empty()
                || frame.data.len() > size - body.len()
            {
                return Err(Failure::Protocol);
            }
            body.extend_from_slice(&frame.data);
        }
        Ok(body)
    }

    pub(super) fn acknowledge(&mut self, tag: u64) -> Result<()> {
        let mut args = tag.to_be_bytes().to_vec();
        args.push(0); // multiple false
        self.method(CHANNEL, 60, 80, &args)
    }

    pub(super) fn reject(&mut self, tag: u64, requeue: bool) -> Result<()> {
        let mut args = tag.to_be_bytes().to_vec();
        args.push(u8::from(requeue));
        self.method(CHANNEL, 60, 90, &args)
    }
}

impl Frame {
    fn method_key(&self) -> Result<(u16, u16, u16)> {
        if self.kind != METHOD {
            return Err(Failure::Protocol);
        }
        let mut fields = self.data.as_slice();
        let class = u16::from_be_bytes(word(&mut fields)?);
        let id = u16::from_be_bytes(word(&mut fields)?);
        Ok((self.channel, class, id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Cursor;

    #[test]
    fn frames_are_incremental_and_enforce_negotiated_limit_and_end_byte() {
        let bytes = [1, 0, 1, 0, 0, 0, 4, 0, 60, 0, 80, END];
        for length in 0..bytes.len() {
            assert!(parse_frame(&bytes[..length], MIN_FRAME).unwrap().is_none());
        }
        let (frame, used) = parse_frame(&bytes, MIN_FRAME).unwrap().unwrap();
        assert_eq!(frame.method_key(), Ok((1, 60, 80)));
        assert_eq!(used, bytes.len());
        assert_eq!(
            parse_frame(&[1, 0, 1, 0, 0, 0x10, 0], MIN_FRAME).unwrap_err(),
            Failure::Protocol
        );
        let mut malformed = bytes;
        malformed[bytes.len() - 1] = 0;
        assert_eq!(
            parse_frame(&malformed, MIN_FRAME).unwrap_err(),
            Failure::Protocol
        );
    }

    #[derive(Default)]
    struct Transcript {
        broker: Cursor<Vec<u8>>,
        client: Vec<u8>,
    }

    impl Read for Transcript {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let limit = buf.len().min(11); // split frame headers and bodies
            self.broker.read(&mut buf[..limit])
        }
    }

    impl Write for Transcript {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.client.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Transcript {
        fn frame(&mut self, kind: u8, channel: u16, data: &[u8]) {
            let bytes = self.broker.get_mut();
            bytes.push(kind);
            bytes.extend_from_slice(&channel.to_be_bytes());
            bytes.extend_from_slice(&(data.len() as u32).to_be_bytes());
            bytes.extend_from_slice(data);
            bytes.push(END);
        }

        fn method(&mut self, channel: u16, class: u16, id: u16, args: &[u8]) {
            let mut data = class.to_be_bytes().to_vec();
            data.extend_from_slice(&id.to_be_bytes());
            data.extend_from_slice(args);
            self.frame(METHOD, channel, &data);
        }

        fn handshake() -> Self {
            let mut transport = Self::default();
            let mut start = vec![0, 9, 0, 0, 0, 0]; // version and empty properties
            start.extend_from_slice(b"\0\0\0\x05PLAIN\0\0\0\x05en_US");
            transport.method(0, 10, 10, &start);
            transport.method(0, 10, 30, &[0, 8, 0, 0, 16, 0, 0, 30]);
            transport.method(0, 10, 41, &[0]);
            transport.method(1, 20, 11, &[0; 4]);
            transport
        }

        fn written(&self) -> Vec<Frame> {
            assert_eq!(&self.client[..8], PROTOCOL);
            let mut bytes = &self.client[8..];
            let mut frames = Vec::new();
            while !bytes.is_empty() {
                let (frame, used) = parse_frame(bytes, MIN_FRAME).unwrap().unwrap();
                bytes = &bytes[used..];
                frames.push(frame);
            }
            frames
        }
    }

    fn settings() -> Settings {
        Settings::parse(
            json!({"url": "amqp://localhost", "queue": "test"})
                .as_object()
                .unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn negotiated_sizes_limit_publish_frames_and_confirm_tags_match() {
        let mut transport = Transcript::handshake();
        transport.method(1, 85, 11, &[]);
        transport.method(1, 60, 80, &[0, 0, 0, 0, 0, 0, 0, 1, 0]);
        transport.method(1, 60, 120, &[0, 0, 0, 0, 0, 0, 0, 2, 1]);
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut connection =
            Connection::handshake(transport, &settings(), deadline, &|| true).unwrap();
        assert_eq!(connection.frame_max, MIN_FRAME);
        assert_eq!(connection.heartbeat, Duration::from_secs(30));
        connection.confirms(deadline, &|| true).unwrap();
        assert_eq!(
            connection.publish("", "test", &vec![7u8; 9000], deadline, &|| true),
            Ok(())
        );
        assert_eq!(
            connection.publish("", "test", &[], deadline, &|| true),
            Err(Failure::Rejected)
        );
        let output = connection.stream.written();
        assert_eq!(output[4].method_key(), Ok((1, 85, 10)));
        assert_eq!(output[5].method_key(), Ok((1, 60, 40)));
        assert_eq!(
            &output[5].data[4..],
            &[0, 0, 0, 4, b't', b'e', b's', b't', 1]
        );
        assert_eq!(output[6].kind, HEADER);
        assert_eq!(&output[6].data[4..12], &9000u64.to_be_bytes());
        assert_eq!(
            output[7..10]
                .iter()
                .map(|frame| frame.data.len())
                .collect::<Vec<_>>(),
            [4088, 4088, 824]
        );
    }

    #[test]
    fn deliveries_keep_routing_data_and_use_manual_disposition() {
        let mut transport = Transcript::handshake();
        transport.method(1, 60, 11, &[]);
        transport.method(1, 60, 21, b"\x03tag");
        let mut deliver = b"\x03tag".to_vec();
        deliver.extend_from_slice(&17u64.to_be_bytes());
        deliver.extend_from_slice(b"\x01\x02ex\x04test");
        transport.method(1, 60, 60, &deliver);
        let mut header = vec![0, 60, 0, 0];
        header.extend_from_slice(&6u64.to_be_bytes());
        header.extend_from_slice(&[0, 0]);
        transport.frame(HEADER, 1, &header);
        transport.frame(BODY, 1, &[0, 255, 2]);
        transport.frame(BODY, 1, &[3, 4, 5]);
        let settings = settings();
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut connection =
            Connection::handshake(transport, &settings, deadline, &|| true).unwrap();
        connection
            .consume(&settings, "test", deadline, &|| true)
            .unwrap();
        let message = connection.delivery(&|| true).unwrap().unwrap();
        assert_eq!(message.tag, 17);
        assert_eq!(message.exchange, "ex");
        assert_eq!(message.routing_key, "test");
        assert!(message.redelivered);
        assert_eq!(message.body, [0, 255, 2, 3, 4, 5]);
        connection.acknowledge(message.tag).unwrap();
        connection.reject(18, true).unwrap();
        let output = connection.stream.written();
        assert_eq!(output[4].method_key(), Ok((1, 60, 10)));
        assert_eq!(&output[4].data[4..], &[0, 0, 0, 0, 0, 16, 0]);
        assert_eq!(output[5].method_key(), Ok((1, 60, 20)));
        assert_eq!(&output[5].data[6..], b"\x04test\0\0\0\0\0\0");
        assert_eq!(output[6].method_key(), Ok((1, 60, 80)));
        assert_eq!(&output[6].data[11..], &[17, 0]);
        assert_eq!(output[7].method_key(), Ok((1, 60, 90)));
        assert_eq!(&output[7].data[11..], &[18, 1]);
    }

    /// Broker bytes are untrusted: a corrupted transcript ends in a typed failure or a
    /// bounded delivery, never a panic.
    #[test]
    fn corrupted_broker_transcripts_fail_without_panicking() {
        let mut valid = Transcript::handshake();
        valid.method(1, 60, 11, &[]);
        valid.method(1, 60, 21, b"\x03tag");
        let mut deliver = b"\x03tag".to_vec();
        deliver.extend_from_slice(&17u64.to_be_bytes());
        deliver.extend_from_slice(b"\x01\x02ex\x04test");
        valid.method(1, 60, 60, &deliver);
        let mut header = vec![0, 60, 0, 0];
        header.extend_from_slice(&6u64.to_be_bytes());
        header.extend_from_slice(&[0, 0]);
        valid.frame(HEADER, 1, &header);
        valid.frame(BODY, 1, &[0, 255, 2, 3, 4, 5]);
        let valid = valid.broker.into_inner();
        let settings = settings();
        let mut state = 0x9E37_79B9_7F4A_7C15_u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            usize::try_from(state % (1 << 31)).unwrap()
        };
        for _ in 0..4000 {
            let mut bytes = valid.clone();
            for _ in 0..=next() % 3 {
                let at = next() % bytes.len();
                match next() % 4 {
                    0 => bytes.truncate(at),
                    1 => bytes.insert(at, u8::try_from(next() % 256).unwrap()),
                    // Lengths and sizes: make a field huge.
                    2 => bytes[at] = 0xff,
                    _ => bytes[at] ^= u8::try_from(next() % 255 + 1).unwrap(),
                }
                if bytes.is_empty() {
                    break;
                }
            }
            let transport = Transcript {
                broker: Cursor::new(bytes),
                client: Vec::new(),
            };
            let deadline = Instant::now() + Duration::from_secs(1);
            let Ok(mut connection) =
                Connection::handshake(transport, &settings, deadline, &|| true)
            else {
                continue;
            };
            if connection
                .consume(&settings, "test", deadline, &|| true)
                .is_err()
            {
                continue;
            }
            if let Ok(Some(message)) = connection.delivery(&|| true) {
                assert!(message.body.len() <= MAX_MESSAGE_BYTES);
            }
        }
    }
}
