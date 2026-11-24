//! What a granted plugin instance may reach outside itself: its own data directory,
//! HTTPS requests and, with `http-local`, HTTP to this computer and the local network.
//! The application supplies both; without them a granted import answers `unavailable`.
//!
//! Errors go to the guest as one of the fixed words below, never as operating-system
//! text, so a path or address outside the plugin's reach does not leak through them.
use orange_sandbox::{Folder, FsError};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// The file does not exist.
pub const NOT_FOUND: &str = "not-found";
/// The path is not a relative path of plain names, or it leaves the data directory.
pub const INVALID_PATH: &str = "invalid-path";
/// The content is over the instance's transfer limit.
pub const TOO_LARGE: &str = "too-large";
/// The application gave this capability nothing to reach.
pub const UNAVAILABLE: &str = "unavailable";
/// The request is not one this interface sends.
pub const INVALID_REQUEST: &str = "invalid-request";
/// The data directory would grow past what the application allows the package.
pub const QUOTA: &str = "quota-exceeded";
/// The address is not one the application lets packages reach.
pub const NOT_ALLOWED: &str = "not-allowed";
/// Any other failure of the operating system or the network.
pub const FAILED: &str = "failed";

pub use orange_sandbox::MAX_PATH_BYTES;
/// Most names one listing returns.
pub const MAX_ENTRIES: usize = 10_000;
/// Most headers of one request.
pub const MAX_HEADERS: usize = 64;
// The quota check and write must agree across guest instances pointing at the
// same package directory. No guest code runs under this guard.
static DATA_QUOTAS: Mutex<()> = Mutex::new(());

/// One HTTP request a plugin asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    /// `GET`, `HEAD`, `POST`, `PUT`, `PATCH` or `DELETE`.
    pub method: String,
    /// An `https://` URL, or an `http://` URL to a local address ([`is_local`]).
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// Largest response body the plugin can take.
    pub max_response_bytes: usize,
}

/// The answer to an [`HttpRequest`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    /// Headers in the order received. Together with the body they must fit the
    /// request's `max_response_bytes`.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Sends the requests of plugins that hold the `http` or `http-local` grant. The
/// application implements it with its own network client, which decides timeouts and
/// which hosts are reachable. The address was checked before the call; a client that
/// follows redirects must check each new address the same way, or not follow them. It is called on the thread that runs the plugin and may
/// block it.
pub trait HttpClient: Send + Sync + std::fmt::Debug {
    /// Sends the request. An error is reported to the plugin as `failed`.
    fn send(&self, request: &HttpRequest) -> Result<HttpResponse, String>;
}

/// What the instances of one package reach when they hold the matching grant.
#[derive(Debug, Clone, Default)]
pub struct GuestAccess {
    /// The package's own data directory, for the `files` grant. Created on first write.
    pub data_dir: Option<PathBuf>,
    /// The client for the `http` and `http-local` grants.
    pub http: Option<Arc<dyn HttpClient>>,
    /// Most bytes the data directory may hold; `None` is no limit.
    pub max_data_bytes: Option<u64>,
    /// The hosts `http` requests may go to, such as `api.example.org`; a name starting
    /// with `.` allows every host under that domain. `None` allows any host. Local
    /// addresses reached through `http-local` are not limited by it.
    pub http_hosts: Option<Vec<String>>,
}

/// The bytes of the files under a directory, or `None` when it holds more than
/// [`MAX_ENTRIES`] files and directories: a total that stopped counting would let a
/// package pass its quota by spreading data over many files, so too many to count is
/// treated as over it. Links are not followed, so a link cannot make the walk loop or
/// count what lies outside. A directory that cannot be read counts as empty.
fn used(root: &std::path::Path) -> Option<u64> {
    let mut total = 0u64;
    let mut seen = 0usize;
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory)
            .into_iter()
            .flatten()
            .flatten()
        {
            seen += 1;
            if seen > MAX_ENTRIES {
                return None;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file() {
                let size = entry.metadata().map_or(0, |metadata| metadata.len());
                total = total.saturating_add(size);
            }
        }
    }
    Some(total)
}

fn word(error: FsError) -> &'static str {
    match error {
        FsError::NotFound => NOT_FOUND,
        FsError::InvalidPath => INVALID_PATH,
        FsError::TooLarge => TOO_LARGE,
        _ => FAILED,
    }
}

impl GuestAccess {
    /// The package's data directory as a sandbox, when the application gave one.
    fn folder(&self) -> Result<Folder, &'static str> {
        self.data_dir
            .as_ref()
            .map(|root| Folder::new(root, false))
            .ok_or(UNAVAILABLE)
    }

    pub(crate) fn read(&self, path: &str, max_bytes: usize) -> Result<Vec<u8>, &'static str> {
        self.folder()?.read(path, max_bytes as u64).map_err(word)
    }

    pub(crate) fn write(&self, path: &str, data: &[u8]) -> Result<(), &'static str> {
        let folder = self.folder()?;
        let _reservation = self
            .max_data_bytes
            .map(|_| DATA_QUOTAS.lock().map_err(|_| FAILED))
            .transpose()?;
        if let Some(limit) = self.max_data_bytes {
            // The file being replaced gives its bytes back.
            let replaced = folder.size(path).unwrap_or(0);
            let after = used(folder.root())
                .ok_or(QUOTA)?
                .saturating_sub(replaced)
                .saturating_add(data.len() as u64);
            if after > limit {
                return Err(QUOTA);
            }
        }
        folder.write(path, data).map_err(word)
    }

    pub(crate) fn list(&self, path: &str) -> Result<Vec<String>, &'static str> {
        self.folder()?.list(path, MAX_ENTRIES).map_err(word)
    }

    pub(crate) fn remove(&self, path: &str) -> Result<(), &'static str> {
        self.folder()?.remove(path).map_err(word)
    }

    /// Sends `request` when the grants reach its address: `internet` (the `http`
    /// grant) reaches any `https://` host [`http_hosts`](Self::http_hosts) allows, and
    /// `local` (the `http-local` grant) reaches local addresses ([`is_local`]) over
    /// `http://` or `https://`.
    pub(crate) fn send(
        &self,
        request: &HttpRequest,
        internet: bool,
        local: bool,
    ) -> Result<HttpResponse, &'static str> {
        let client = self.http.as_ref().ok_or(UNAVAILABLE)?;
        let method = matches!(
            request.method.as_str(),
            "GET" | "HEAD" | "POST" | "PUT" | "PATCH" | "DELETE"
        );
        let (secure, rest) = match request.url.strip_prefix("https://") {
            Some(rest) => (true, Some(rest)),
            None => (false, request.url.strip_prefix("http://")),
        };
        let authority = rest.map(|rest| rest.split(['/', '?', '#']).next().unwrap_or_default());
        let name = authority
            .filter(|authority| !authority.is_empty() && !authority.contains('@'))
            .and_then(host_name)
            .map(str::to_ascii_lowercase);
        let url = name.is_some() && request.url.chars().all(|c| c.is_ascii_graphic());
        let headers = request.headers.len() <= MAX_HEADERS
            && request.headers.iter().all(|(name, value)| {
                !name.is_empty()
                    && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    && !value.chars().any(char::is_control)
            });
        let Some(name) = name.filter(|_| method && url && headers) else {
            return Err(INVALID_REQUEST);
        };
        let listed = || {
            self.http_hosts.as_ref().is_none_or(|allowed| {
                allowed.iter().any(|entry| {
                    let entry = entry.to_ascii_lowercase();
                    match entry.strip_prefix('.') {
                        Some(domain) => name
                            .strip_suffix(domain)
                            .is_some_and(|rest| rest.ends_with('.')),
                        None => name == entry,
                    }
                })
            })
        };
        let reachable = (local && is_local(&name)) || (internet && secure && listed());
        if !reachable {
            return Err(NOT_ALLOWED);
        }
        let response = client.send(request).map_err(|_| FAILED)?;
        let header_bytes = response
            .headers
            .iter()
            .map(|(name, value)| name.len() + value.len())
            .fold(0usize, usize::saturating_add);
        if response.body.len().saturating_add(header_bytes) > request.max_response_bytes {
            return Err(TOO_LARGE);
        }
        Ok(response)
    }
}

/// The host name of a URL authority without its port, and without the brackets of an
/// IPv6 address. `None` when the authority is malformed.
fn host_name(authority: &str) -> Option<&str> {
    let name = match authority.strip_prefix('[') {
        Some(rest) => {
            let (inside, after) = rest.split_once(']')?;
            (after.is_empty() || after.starts_with(':')).then_some(inside)?
        }
        // A port is part of the address, not of the host's name.
        None => authority
            .rsplit_once(':')
            .map_or(authority, |(name, _)| name),
    };
    (!name.is_empty()).then_some(name)
}

/// Whether a host name is this computer or the local network, as the `http-local`
/// grant reaches: `localhost` and names under it, `.local` names, and loopback,
/// private and link-local IPv4 and IPv6 addresses. Other names are not resolved here,
/// so a public name that happens to point at a local address is not local.
pub fn is_local(name: &str) -> bool {
    use std::net::IpAddr;
    let name = name.trim_end_matches('.').to_ascii_lowercase();
    if name == "localhost" || name.ends_with(".localhost") || name.ends_with(".local") {
        return true;
    }
    match name.parse::<IpAddr>() {
        Ok(IpAddr::V4(address)) => {
            address.is_loopback() || address.is_private() || address.is_link_local()
        }
        Ok(IpAddr::V6(address)) => {
            address.is_loopback()
                || address.is_unique_local()
                || address.is_unicast_link_local()
                || address
                    .to_ipv4_mapped()
                    .is_some_and(|v4| v4.is_loopback() || v4.is_private() || v4.is_link_local())
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_are_listed_limited_and_removed_inside_the_directory() {
        let root = std::env::temp_dir().join(format!("orange-guest-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let access = GuestAccess {
            data_dir: Some(root.clone()),
            ..GuestAccess::default()
        };
        // Before the first write the directory is empty, not an error.
        assert_eq!(access.list(""), Ok(vec![]));
        assert_eq!(access.read("a.txt", 16), Err(NOT_FOUND));
        access.write("b/c.bin", &[1, 2, 3]).unwrap();
        access.write("a.txt", b"hello").unwrap();
        assert_eq!(
            access.list(""),
            Ok(vec!["a.txt".to_owned(), "b/".to_owned()])
        );
        assert_eq!(access.list("b"), Ok(vec!["c.bin".to_owned()]));
        assert_eq!(access.read("b/c.bin", 3), Ok(vec![1, 2, 3]));
        assert_eq!(access.read("b/c.bin", 2), Err(TOO_LARGE));
        assert_eq!(access.remove("a.txt"), Ok(()));
        assert_eq!(access.remove("a.txt"), Err(NOT_FOUND));
        assert_eq!(access.remove("../a.txt"), Err(INVALID_PATH));
        assert_eq!(access.list("../"), Err(INVALID_PATH));

        let nothing = GuestAccess::default();
        assert_eq!(nothing.read("a.txt", 16), Err(UNAVAILABLE));
        assert_eq!(nothing.write("a.txt", b""), Err(UNAVAILABLE));
        let request = HttpRequest {
            method: "GET".into(),
            url: "https://example.org/".into(),
            headers: Vec::new(),
            body: Vec::new(),
            max_response_bytes: 16,
        };
        assert_eq!(nothing.send(&request, true, true), Err(UNAVAILABLE));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[derive(Debug)]
    struct Echo;
    impl HttpClient for Echo {
        fn send(&self, request: &HttpRequest) -> Result<HttpResponse, String> {
            Ok(HttpResponse {
                status: 200,
                headers: vec![("content-type".into(), "text/plain".into())],
                body: request.url.as_bytes().to_vec(),
            })
        }
    }

    #[test]
    fn a_data_quota_and_a_host_list_limit_what_a_package_reaches() {
        let root = std::env::temp_dir().join(format!("orange-guest-quota-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let access = GuestAccess {
            data_dir: Some(root.clone()),
            http: Some(Arc::new(Echo)),
            max_data_bytes: Some(10),
            http_hosts: Some(vec!["api.example.org".into(), ".trusted.example".into()]),
        };
        access.write("a.bin", &[0; 6]).unwrap();
        assert_eq!(access.write("sub/b.bin", &[0; 5]), Err(QUOTA));
        access.write("sub/b.bin", &[0; 4]).unwrap();
        // Replacing a file counts only what it adds; shrinking one frees room.
        assert_eq!(access.write("a.bin", &[0; 7]), Err(QUOTA));
        access.write("a.bin", &[0; 2]).unwrap();
        access.write("c.bin", &[0; 4]).unwrap();
        assert_eq!(access.write("d.bin", &[0; 1]), Err(QUOTA));
        // Spreading data over more files than are counted does not get past the quota.
        let many = root.join("many");
        std::fs::create_dir_all(&many).unwrap();
        for n in 0..=MAX_ENTRIES {
            std::fs::write(many.join(n.to_string()), b"").unwrap();
        }
        access.remove("c.bin").unwrap();
        assert_eq!(access.write("e.bin", &[0; 1]), Err(QUOTA));
        std::fs::remove_dir_all(&many).unwrap();
        access.write("e.bin", &[0; 1]).unwrap();

        let send = |url: &str| {
            access
                .send(
                    &HttpRequest {
                        method: "GET".into(),
                        url: url.into(),
                        headers: Vec::new(),
                        body: Vec::new(),
                        max_response_bytes: 256,
                    },
                    true,
                    false,
                )
                .map(|response| response.status)
        };
        assert_eq!(send("https://api.example.org/v1"), Ok(200));
        assert_eq!(send("https://API.example.org:8443/v1"), Ok(200));
        assert_eq!(send("https://a.b.trusted.example/"), Ok(200));
        for refused in [
            "https://example.org/",
            "https://evil-api.example.org/",
            "https://trusted.example/",
            "https://nottrusted.example/",
            "https://api.example.org.evil.test/",
        ] {
            assert_eq!(send(refused), Err(NOT_ALLOWED), "{refused}");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn http_local_reaches_only_local_addresses_and_https_needs_the_http_grant() {
        let access = GuestAccess {
            http: Some(Arc::new(Echo)),
            http_hosts: Some(vec!["api.example.org".into()]),
            ..GuestAccess::default()
        };
        let send = |url: &str, internet: bool, local: bool| {
            let request = HttpRequest {
                method: "GET".into(),
                url: url.into(),
                headers: Vec::new(),
                body: Vec::new(),
                max_response_bytes: 256,
            };
            access
                .send(&request, internet, local)
                .map(|response| response.status)
        };
        let local = [
            "http://127.0.0.1:8880/v1/audio",
            "http://localhost/",
            "http://LOCALHOST.:3000/",
            "http://studio.local/",
            "http://192.168.1.20/",
            "http://10.0.0.7/",
            "http://172.16.4.1/",
            "http://169.254.1.1/",
            "http://[::1]:8080/",
            "http://[fd00::1]/",
            "http://[fe80::1]/",
            "https://127.0.0.1/",
        ];
        for url in local {
            assert_eq!(send(url, false, true), Ok(200), "{url}");
            // `http` alone never reaches plain HTTP, and not a local address that the
            // host list leaves out.
            assert_eq!(send(url, true, false), Err(NOT_ALLOWED), "{url}");
        }
        for public in [
            "http://example.org/",
            "http://8.8.8.8/",
            "http://172.32.0.1/",
            "http://[2001:db8::1]/",
            "http://localhost.example.org/",
            "https://api.example.org/",
        ] {
            assert_eq!(send(public, false, true), Err(NOT_ALLOWED), "{public}");
        }
        assert_eq!(send("https://api.example.org/", true, false), Ok(200));
        assert_eq!(
            send("http://api.example.org/", true, true),
            Err(NOT_ALLOWED)
        );
        for malformed in [
            "ftp://127.0.0.1/",
            "http://[::1/",
            "http://user@127.0.0.1/",
            "http:///x",
        ] {
            assert_eq!(
                send(malformed, true, true),
                Err(INVALID_REQUEST),
                "{malformed}"
            );
        }
    }

    #[test]
    fn response_headers_count_toward_the_transfer_limit() {
        let access = GuestAccess {
            http: Some(Arc::new(Echo)),
            ..GuestAccess::default()
        };
        let url = "https://example.org/";
        let request = |max_response_bytes| HttpRequest {
            method: "GET".into(),
            url: url.into(),
            headers: Vec::new(),
            body: Vec::new(),
            max_response_bytes,
        };
        // Echo answers the URL with a 22-byte `content-type: text/plain` header.
        let needed = url.len() + "content-type".len() + "text/plain".len();
        let response = access.send(&request(needed), true, false).unwrap();
        assert_eq!(
            response.headers,
            vec![("content-type".to_owned(), "text/plain".to_owned())]
        );
        assert_eq!(
            access.send(&request(needed - 1), true, false),
            Err(TOO_LARGE)
        );
    }
}
