//! Shared blocking TCP and rustls transport for the broker protocols.

use std::io::{ErrorKind, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub(crate) type Authorities = Vec<rustls::pki_types::CertificateDer<'static>>;

pub(crate) enum Stream {
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

pub(crate) fn open(
    host: &str,
    port: u16,
    tls: bool,
    authorities: Option<&Authorities>,
    poll: Duration,
    timeout: Duration,
) -> Option<Stream> {
    let deadline = Instant::now() + timeout;
    if timeout.is_zero() {
        return None;
    }
    let addresses = (host, port).to_socket_addrs().ok()?;
    let mut tcp = addresses
        .filter(|_| Instant::now() < deadline)
        .find_map(|address| {
            TcpStream::connect_timeout(&address, deadline.saturating_duration_since(Instant::now()))
                .ok()
        })?;
    let _ = tcp.set_nodelay(true);
    tcp.set_read_timeout(Some(poll)).ok()?;
    tcp.set_write_timeout(Some(timeout)).ok()?;
    if !tls {
        return Some(Stream::Plain(tcp));
    }
    let roots = match authorities {
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
    let name = rustls::pki_types::ServerName::try_from(host.to_owned()).ok()?;
    let mut connection = rustls::ClientConnection::new(Arc::new(config), name).ok()?;
    // Finish the verified TLS handshake using the connection deadline. The shorter
    // packet polling timeout is a resumable wait, not a handshake timeout.
    while connection.is_handshaking() {
        if Instant::now() >= deadline {
            return None;
        }
        match connection.complete_io(&mut tcp) {
            Ok((0, 0)) if connection.is_handshaking() => return None,
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                ) => {}
            Err(_) => return None,
        }
    }
    Some(Stream::Tls(Box::new(rustls::StreamOwned::new(
        connection, tcp,
    ))))
}

pub(crate) fn load_authorities(path: &str) -> Result<Authorities, &'static str> {
    use rustls::pki_types::pem::PemObject;
    let pem = std::fs::read(path).map_err(|_| "`ca_file` could not be read")?;
    let found: Authorities = rustls::pki_types::CertificateDer::pem_slice_iter(&pem)
        .collect::<Result<_, _>>()
        .map_err(|_| "`ca_file` is not a PEM certificate file")?;
    if found.is_empty() {
        return Err("`ca_file` holds no certificate");
    }
    Ok(found)
}
