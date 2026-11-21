use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;
use tungstenite::Message;
use tungstenite::handshake::server::{Request, Response};
use tungstenite::http::StatusCode;

const JSON: &[u8] = br#"{"source":"orange-network-lab","ok":true}"#;
const BYTES: &[u8] = &[0, 255, 128, 10];
const LINES: [&[u8]; 2] = [b"{\"sample\":1}\n", b"{\"sample\":2}\n"];
const BASIC: &str = "Basic bGFiOmxhYi1odHRwLW9ubHk=";
const BEARER: &str = "Bearer lab-stream-only";
const WS_BEARER: &str = "Bearer lab-websocket-only";
const API_KEY: &str = "lab-bytes-only";

fn http_reply(mut input: impl BufRead, mut output: impl Write) -> io::Result<()> {
    let mut line = String::new();
    input.read_line(&mut line)?;
    let route = line.split_whitespace().nth(1).unwrap_or("").to_owned();
    let mut auth = String::new();
    let mut key = String::new();
    loop {
        line.clear();
        if input.read_line(&mut line)? == 0 || line.trim_end().is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("authorization") {
                auth = value.trim().to_owned();
            } else if name.eq_ignore_ascii_case("x-api-key") {
                key = value.trim().to_owned();
            }
        }
    }

    let anonymous = auth.is_empty() && key.is_empty();
    let (status, media_type, body) = match route.as_str() {
        "/health" => ("200 OK", "text/plain", &b"ready"[..]),
        "/json" if anonymous => ("200 OK", "application/json", JSON),
        "/basic/json" if auth == BASIC => ("200 OK", "application/json", JSON),
        "/bytes" if anonymous => ("200 OK", "application/octet-stream", BYTES),
        "/key/bytes" if key == API_KEY => ("200 OK", "application/octet-stream", BYTES),
        "/stream" | "/bearer/stream" => {
            let authorized = if route == "/stream" {
                anonymous
            } else {
                auth == BEARER
            };
            if authorized {
                return stream(output);
            }
            (
                "401 Unauthorized",
                "text/plain",
                &b"credentials rejected"[..],
            )
        }
        "/json" | "/basic/json" | "/bytes" | "/key/bytes" => (
            "401 Unauthorized",
            "text/plain",
            &b"credentials rejected"[..],
        ),
        _ => (
            "404 Not Found",
            "text/plain",
            &b"fixture route not found"[..],
        ),
    };
    write!(
        output,
        "HTTP/1.1 {status}\r\nContent-Type: {media_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    output.write_all(body)
}

fn stream(mut output: impl Write) -> io::Result<()> {
    output.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n")?;
    for line in LINES {
        write!(output, "{:X}\r\n", line.len())?;
        output.write_all(line)?;
        output.write_all(b"\r\n")?;
        output.flush()?;
        thread::sleep(Duration::from_millis(125));
    }
    output.write_all(b"0\r\n\r\n")
}

fn ws_status(request: &Request) -> StatusCode {
    let authorization = request.headers().get("authorization");
    match request.uri().path() {
        "/echo" if authorization.is_none() => StatusCode::SWITCHING_PROTOCOLS,
        "/auth/echo" if authorization.is_some_and(|value| value == WS_BEARER) => {
            StatusCode::SWITCHING_PROTOCOLS
        }
        "/echo" | "/auth/echo" => StatusCode::UNAUTHORIZED,
        _ => StatusCode::NOT_FOUND,
    }
}

#[expect(
    clippy::result_large_err,
    reason = "tungstenite defines the handshake callback result"
)]
fn echo(stream: TcpStream) -> io::Result<()> {
    let mut socket = tungstenite::accept_hdr(stream, |request: &Request, response: Response| {
        let status = ws_status(request);
        if status == StatusCode::SWITCHING_PROTOCOLS {
            Ok(response)
        } else {
            Err(tungstenite::http::Response::builder()
                .status(status)
                .body(None)
                .unwrap())
        }
    })
    .map_err(io::Error::other)?;
    loop {
        match socket.read().map_err(io::Error::other)? {
            message @ (Message::Text(_) | Message::Binary(_)) => {
                socket.send(message).map_err(io::Error::other)?
            }
            Message::Close(_) => return socket.flush().map_err(io::Error::other),
            _ => socket.flush().map_err(io::Error::other)?,
        }
    }
}

fn listen(listener: TcpListener, serve: fn(TcpStream)) -> io::Result<()> {
    for stream in listener.incoming() {
        let stream = stream?;
        stream.set_read_timeout(Some(Duration::from_secs(30)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        thread::spawn(move || serve(stream));
    }
    Ok(())
}

fn main() -> io::Result<()> {
    if std::env::args().nth(1).as_deref() == Some("--check") {
        let mut http = TcpStream::connect("127.0.0.1:8080")?;
        http.write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n")?;
        let mut response = String::new();
        http.read_to_string(&mut response)?;
        if !response.starts_with("HTTP/1.1 200 ") {
            return Err(io::Error::other("HTTP fixture did not answer"));
        }
        tungstenite::connect("ws://127.0.0.1:8081/echo").map_err(io::Error::other)?;
        return Ok(());
    }
    let http = TcpListener::bind("0.0.0.0:8080")?;
    let ws = TcpListener::bind("0.0.0.0:8081")?;
    thread::spawn(move || {
        listen(ws, |stream| {
            let _ = echo(stream);
        })
    });
    listen(http, |stream| {
        let input = BufReader::new((&stream).take(8192));
        let _ = http_reply(input, &stream);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_routes_require_the_right_credentials_and_send_complete_chunks() {
        let request = |route: &str, headers: &str| {
            let input = format!("GET {route} HTTP/1.1\r\n{headers}\r\n");
            let mut result = Vec::new();
            http_reply(input.as_bytes(), &mut result).unwrap();
            result
        };
        assert!(request("/json", "").ends_with(JSON));
        assert!(request("/basic/json", "").starts_with(b"HTTP/1.1 401"));
        assert!(request("/basic/json", &format!("Authorization: {BASIC}\r\n")).ends_with(JSON));
        assert!(request("/key/bytes", "X-API-Key: wrong\r\n").starts_with(b"HTTP/1.1 401"));
        assert!(request("/key/bytes", &format!("X-API-Key: {API_KEY}\r\n")).ends_with(BYTES));
        assert!(request("/bearer/stream", "").starts_with(b"HTTP/1.1 401"));
        let streamed = request("/bearer/stream", &format!("Authorization: {BEARER}\r\n"));
        assert!(streamed.ends_with(b"D\r\n{\"sample\":1}\n\r\nD\r\n{\"sample\":2}\n\r\n0\r\n\r\n"));
    }

    #[test]
    fn websocket_routes_distinguish_anonymous_allowed_and_rejected() {
        let request = |path, auth: Option<&str>| {
            let mut builder = Request::builder().uri(path);
            if let Some(auth) = auth {
                builder = builder.header("Authorization", auth);
            }
            ws_status(&builder.body(()).unwrap())
        };
        assert_eq!(request("/echo", None), StatusCode::SWITCHING_PROTOCOLS);
        assert_eq!(
            request("/auth/echo", Some(WS_BEARER)),
            StatusCode::SWITCHING_PROTOCOLS
        );
        assert_eq!(
            request("/auth/echo", Some("Bearer wrong")),
            StatusCode::UNAUTHORIZED
        );
    }
}
