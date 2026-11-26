//! Downloads and update feeds: fetching over HTTPS, and signing and checking a feed.
use crate::{DEFAULT_TIMEOUT_MS, hex};
use std::net::IpAddr;
use std::time::Duration;
use ureq::Agent;
use ureq::config::Config;
use ureq::http::{Method, Request, Uri};
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{DefaultConnector, NextTimeout};

/// Opens a finite HTTP media response for an explicitly configured source.
/// Redirects are disabled and the response reader enforces the byte limit.
pub fn media_reader(url: &str, max_bytes: u64) -> Result<impl std::io::Read + Send + Sync, String> {
    let address = url.parse::<Uri>().map_err(|_| "invalid audio URL")?;
    if !matches!(address.scheme_str(), Some("http" | "https"))
        || address
            .authority()
            .is_none_or(|origin| origin.as_str().contains('@'))
    {
        return Err("an audio URL must be HTTP or HTTPS without embedded credentials".into());
    }
    let response = Agent::config_builder()
        .proxy(None)
        .max_redirects(0)
        .timeout_global(Some(Duration::from_secs(60)))
        .timeout_connect(Some(Duration::from_secs(5)))
        .timeout_recv_response(Some(Duration::from_secs(10)))
        .timeout_recv_body(Some(Duration::from_secs(60)))
        .build()
        .new_agent()
        .get(address)
        .call()
        .map_err(|_| "the audio server could not be reached")?;
    if response.status().as_u16() != 200 {
        return Err("the audio server did not return a complete file response".into());
    }
    if response
        .body()
        .content_length()
        .is_some_and(|size| size > max_bytes)
    {
        return Err("the audio file exceeds the download limit".into());
    }
    Ok(response
        .into_body()
        .into_with_config()
        .limit(max_bytes)
        .reader())
}

/// Downloads an `https://` URL, following redirects only to other `https://` URLs: how
/// an application reads a package's update feed or a release archive. Anything but
/// status 200, or a body over `max_bytes`, is an error. This is not a graph node and
/// needs no host.
pub fn download(url: &str, max_bytes: u64) -> Result<Vec<u8>, String> {
    if !url.starts_with("https://") {
        return Err("the URL must start with https://".into());
    }
    let agent = Agent::config_builder()
        .timeout_global(Some(Duration::from_millis(DEFAULT_TIMEOUT_MS)))
        .https_only(true)
        .http_status_as_error(false)
        .build()
        .new_agent();
    let mut response = agent
        .get(url)
        .call()
        .map_err(|_| "the download could not be completed".to_owned())?;
    let status = response.status().as_u16();
    if status != 200 {
        return Err(format!("the server answered with status {status}"));
    }
    response
        .body_mut()
        .with_config()
        .limit(max_bytes)
        .read_to_vec()
        .map_err(|_| "the download is too large or was cut short".to_owned())
}

/// Sends one HTTPS request and returns the status and body: what an application's
/// client for plugins holding the `http` grant does. Redirects are not followed, so
/// the request reaches only the host it names; a body over `max_bytes` is an error.
pub fn fetch(
    method: &str,
    url: &str,
    headers: &[(String, String)],
    body: &[u8],
    max_bytes: u64,
) -> Result<(u16, Vec<u8>), String> {
    fetch_response(method, url, headers, body, max_bytes, FetchRules::HTTPS)
        .map(|response| (response.status, response.body))
}

/// What one [`fetch_response`] may use and where it may connect.
#[derive(Debug, Clone, Copy)]
pub struct FetchRules {
    /// Whether a plain `http://` URL may be sent. The caller has checked that the
    /// request may reach its address in plain text, as the `http-local` plugin grant does.
    pub plain_http: bool,
    /// Which resolved addresses the request may connect to; `None` allows any. With a
    /// filter the request connects directly, never through a proxy from the
    /// environment, so the check holds for the address actually reached.
    pub connect_to: Option<fn(IpAddr) -> bool>,
}

impl FetchRules {
    /// `https://` only, to any address, through the environment's proxy if one is set.
    pub const HTTPS: Self = Self {
        plain_http: false,
        connect_to: None,
    };
}

/// Resolves host names as usual and keeps only the addresses a filter accepts, so a
/// name cannot point a checked request somewhere else.
#[derive(Debug)]
struct FilteredResolver(fn(IpAddr) -> bool);

impl Resolver for FilteredResolver {
    fn resolve(
        &self,
        uri: &Uri,
        config: &Config,
        timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let resolved = DefaultResolver::default().resolve(uri, config, timeout)?;
        let Self(accepts) = *self;
        let mut allowed = self.empty();
        for address in resolved.iter().filter(|address| accepts(address.ip())) {
            allowed.push(*address);
        }
        if allowed.is_empty() {
            return Err(ureq::Error::HostNotFound);
        }
        Ok(allowed)
    }
}

/// The answer to [`fetch_response`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchResponse {
    pub status: u16,
    /// Headers in the order received. A value that is not UTF-8 is converted lossily.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// [`fetch_response`] with its answer as (status, headers, body), the shape an
/// application hands the plugin engine as its transport. The engine has checked the
/// URL's scheme for the grant, so `http://` is sent, and the request connects only to
/// addresses `connect_to` accepts.
#[expect(clippy::type_complexity, reason = "the transport's plain shape")]
pub fn fetch_parts(
    method: &str,
    url: &str,
    headers: &[(String, String)],
    body: &[u8],
    max_bytes: u64,
    connect_to: fn(IpAddr) -> bool,
) -> Result<(u16, Vec<(String, String)>, Vec<u8>), String> {
    let rules = FetchRules {
        plain_http: true,
        connect_to: Some(connect_to),
    };
    fetch_response(method, url, headers, body, max_bytes, rules)
        .map(|response| (response.status, response.headers, response.body))
}

/// Sends one request like [`fetch`] and also returns the response headers, within
/// `rules`. Redirects are never followed, so the address check holds for the whole
/// request. Headers and body together must fit `max_bytes`.
pub fn fetch_response(
    method: &str,
    url: &str,
    headers: &[(String, String)],
    body: &[u8],
    max_bytes: u64,
    rules: FetchRules,
) -> Result<FetchResponse, String> {
    let secure = url.starts_with("https://");
    if !secure && !(rules.plain_http && url.starts_with("http://")) {
        return Err("the URL must start with https://".into());
    }
    let config = Agent::config_builder()
        .timeout_global(Some(Duration::from_millis(DEFAULT_TIMEOUT_MS)))
        .https_only(secure)
        .max_redirects(0)
        .http_status_as_error(false);
    let agent = match rules.connect_to {
        Some(filter) => Agent::with_parts(
            config.proxy(None).build(),
            DefaultConnector::default(),
            FilteredResolver(filter),
        ),
        None => config.build().new_agent(),
    };
    let method = Method::from_bytes(method.as_bytes()).map_err(|_| "the method is not valid")?;
    let mut request = Request::builder().method(method).uri(url);
    for (name, value) in headers {
        request = request.header(name.as_str(), value.as_str());
    }
    let request = request
        .body(body.to_vec())
        .map_err(|_| "the request is not valid".to_owned())?;
    let mut response = agent
        .run(request)
        .map_err(|_| "the request could not be completed".to_owned())?;
    let status = response.status().as_u16();
    let mut header_bytes = 0u64;
    let headers: Vec<(String, String)> = response
        .headers()
        .iter()
        .map(|(name, value)| {
            let pair = (
                name.as_str().to_owned(),
                String::from_utf8_lossy(value.as_bytes()).into_owned(),
            );
            header_bytes = header_bytes.saturating_add((pair.0.len() + pair.1.len()) as u64);
            pair
        })
        .collect();
    let Some(left) = max_bytes.checked_sub(header_bytes) else {
        return Err("the response is too large or was cut short".to_owned());
    };
    let body = response
        .body_mut()
        .with_config()
        .limit(left)
        .read_to_vec()
        .map_err(|_| "the response is too large or was cut short".to_owned())?;
    Ok(FetchResponse {
        status,
        headers,
        body,
    })
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    let digits = text.as_bytes();
    if !digits.len().is_multiple_of(2) {
        return None;
    }
    digits
        .chunks(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).ok()?;
            u8::from_str_radix(text, 16).ok()
        })
        .collect()
}

/// Checks that `signature` is the publisher's Ed25519 signature over exactly `feed`.
/// `key` is a package's `update.key` (`ed25519:<64 hex>`); `signature` is the content of
/// the feed's `.sig` document, 128 hexadecimal digits, surrounding white space ignored.
pub fn verify_feed(feed: &[u8], signature: &[u8], key: &str) -> Result<(), String> {
    let refused = || "the update feed is not signed by the package's key".to_owned();
    let key = key
        .strip_prefix("ed25519:")
        .and_then(unhex)
        .filter(|key| key.len() == 32)
        .ok_or("the package's update key is not an Ed25519 public key")?;
    let signature = std::str::from_utf8(signature)
        .ok()
        .and_then(|text| unhex(text.trim()))
        .filter(|signature| signature.len() == 64)
        .ok_or_else(refused)?;
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, key)
        .verify(feed, &signature)
        .map_err(|_| refused())
}

/// Makes a new feed-signing key. The first value is the private key, as text to keep
/// in a credential store and nowhere else; the second is the public key in the form a
/// manifest's `update.key` takes, `ed25519:<64 hex>`.
pub fn new_feed_key() -> Result<(String, String), String> {
    use ring::signature::KeyPair;
    let failed = || "the key could not be generated".to_owned();
    let private = ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
        .map_err(|_| failed())?;
    let pair =
        ring::signature::Ed25519KeyPair::from_pkcs8(private.as_ref()).map_err(|_| failed())?;
    Ok((
        hex(private.as_ref()),
        format!("ed25519:{}", hex(pair.public_key().as_ref())),
    ))
}

/// Signs the exact bytes of a feed with a private key from [`new_feed_key`]. Returns
/// the content of the feed's `.sig` document and the public key that verifies it.
pub fn sign_feed(feed: &[u8], private_key: &str) -> Result<(String, String), String> {
    use ring::signature::KeyPair;
    let pair = unhex(private_key.trim())
        .and_then(|bytes| ring::signature::Ed25519KeyPair::from_pkcs8(&bytes).ok())
        .ok_or("the private key is not a feed-signing key")?;
    Ok((
        hex(pair.sign(feed).as_ref()),
        format!("ed25519:{}", hex(pair.public_key().as_ref())),
    ))
}

/// The lowercase hexadecimal SHA-256 of `bytes` as `sha256:<64 hex>`: the form a feed's
/// `content_digest` has for a native plugin library.
pub fn sha256(bytes: &[u8]) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, bytes);
    format!("sha256:{}", hex(digest.as_ref()))
}

#[cfg(test)]
mod tests {
    use super::{fetch_parts, hex, sha256, verify_feed};
    use ring::signature::{Ed25519KeyPair, KeyPair};
    use std::io::{Read, Write};

    #[test]
    fn a_checked_request_connects_only_to_resolved_addresses_its_filter_accepts() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 1024];
            let _ = stream.read(&mut request).unwrap();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .unwrap();
        });
        let get = |url: &str, connect_to: fn(std::net::IpAddr) -> bool| {
            fetch_parts("GET", url, &[], &[], 1024, connect_to)
                .map(|(status, _, body)| (status, body))
        };
        let public = |address: std::net::IpAddr| !address.is_loopback();
        // A name resolving to this computer is refused before any connection is made.
        for url in [
            format!("http://127.0.0.1:{port}/"),
            format!("http://localhost:{port}/"),
        ] {
            assert!(get(&url, public).is_err(), "{url}");
        }
        let local = |address: std::net::IpAddr| address.is_loopback();
        assert_eq!(
            get(&format!("http://127.0.0.1:{port}/"), local),
            Ok((200, b"ok".to_vec()))
        );
        server.join().unwrap();
    }

    #[test]
    fn a_feed_is_accepted_only_with_its_publishers_signature() {
        let publisher = Ed25519KeyPair::from_seed_unchecked(&[7; 32]).unwrap();
        let other = Ed25519KeyPair::from_seed_unchecked(&[8; 32]).unwrap();
        let key = format!("ed25519:{}", hex(publisher.public_key().as_ref()));
        let feed = br#"{"format": "orange.update-feed", "format_version": 1, "releases": []}"#;
        let signature = format!("{}\n", hex(publisher.sign(feed).as_ref()));

        assert_eq!(verify_feed(feed, signature.as_bytes(), &key), Ok(()));
        // A changed feed, another signer, a damaged signature and a bad key are refused.
        assert!(verify_feed(b"{}", signature.as_bytes(), &key).is_err());
        let forged = hex(other.sign(feed).as_ref());
        assert!(verify_feed(feed, forged.as_bytes(), &key).is_err());
        assert!(verify_feed(feed, b"00", &key).is_err());
        assert!(verify_feed(feed, b"not hex", &key).is_err());
        assert!(verify_feed(feed, signature.as_bytes(), "ed25519:00").is_err());
    }

    #[test]
    fn a_generated_key_signs_a_feed_that_its_public_key_verifies() {
        let (private, public) = super::new_feed_key().unwrap();
        let (other_private, other_public) = super::new_feed_key().unwrap();
        assert_ne!(public, other_public);
        let feed = br#"{"format": "orange.update-feed", "format_version": 1, "releases": []}"#;
        let (signature, signer) = super::sign_feed(feed, &private).unwrap();
        assert_eq!(signer, public);
        assert_eq!(signature.len(), 128);
        assert_eq!(verify_feed(feed, signature.as_bytes(), &public), Ok(()));
        assert!(verify_feed(feed, signature.as_bytes(), &other_public).is_err());
        let (forged, _) = super::sign_feed(feed, &other_private).unwrap();
        assert!(verify_feed(feed, forged.as_bytes(), &public).is_err());
        // The public key is not a private key, and neither is arbitrary text.
        assert!(super::sign_feed(feed, &public).is_err());
        assert!(super::sign_feed(feed, "00ff").is_err());
    }

    #[test]
    fn the_digest_of_a_file_has_the_feed_form() {
        assert_eq!(
            sha256(b"abc"),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
