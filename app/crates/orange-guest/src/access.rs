//! What a granted plugin instance may reach outside itself: its own data directory and
//! HTTPS requests. The application supplies both; without them a granted import
//! answers `unavailable`.
//!
//! Errors go to the guest as one of the fixed words below, never as operating-system
//! text, so a path or address outside the plugin's reach does not leak through them.
use orange_sandbox::{Folder, FsError};
use std::path::PathBuf;
use std::sync::Arc;

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

/// One HTTPS request a plugin asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    /// `GET`, `HEAD`, `POST`, `PUT`, `PATCH` or `DELETE`.
    pub method: String,
    /// An `https://` URL.
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
    pub body: Vec<u8>,
}

/// Sends the HTTPS requests of plugins that hold the `http` grant. The application
/// implements it with its own network client, which decides timeouts, redirects and
/// which hosts are reachable. It is called on the thread that runs the plugin and may
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
    /// The client for the `http` grant.
    pub http: Option<Arc<dyn HttpClient>>,
    /// Most bytes the data directory may hold; `None` is no limit.
    pub max_data_bytes: Option<u64>,
    /// The hosts requests may go to, such as `api.example.org`; a name starting with
    /// `.` allows every host under that domain. `None` allows any host.
    pub http_hosts: Option<Vec<String>>,
}

/// The bytes of the files under a directory. A directory that cannot be read counts
/// as empty; the walk stops at [`MAX_ENTRIES`] files.
fn used(root: &std::path::Path) -> u64 {
    let mut total = 0u64;
    let mut seen = 0usize;
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory)
            .into_iter()
            .flatten()
            .flatten()
        {
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.is_dir() {
                pending.push(entry.path());
            } else {
                total = total.saturating_add(metadata.len());
                seen += 1;
                if seen >= MAX_ENTRIES {
                    return total;
                }
            }
        }
    }
    total
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
        if let Some(limit) = self.max_data_bytes {
            // The file being replaced gives its bytes back.
            let replaced = folder.size(path).unwrap_or(0);
            let after = used(folder.root())
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

    pub(crate) fn send(&self, request: &HttpRequest) -> Result<HttpResponse, &'static str> {
        let client = self.http.as_ref().ok_or(UNAVAILABLE)?;
        let method = matches!(
            request.method.as_str(),
            "GET" | "HEAD" | "POST" | "PUT" | "PATCH" | "DELETE"
        );
        let host = request
            .url
            .strip_prefix("https://")
            .map(|rest| rest.split(['/', '?', '#']).next().unwrap_or_default());
        let url = host.is_some_and(|host| !host.is_empty() && !host.contains('@'))
            && request.url.chars().all(|c| c.is_ascii_graphic());
        let headers = request.headers.len() <= MAX_HEADERS
            && request.headers.iter().all(|(name, value)| {
                !name.is_empty()
                    && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    && !value.chars().any(char::is_control)
            });
        if !(method && url && headers) {
            return Err(INVALID_REQUEST);
        }
        if let (Some(allowed), Some(host)) = (&self.http_hosts, host) {
            // A port is part of the address, not of the host's name.
            let name = host.rsplit_once(':').map_or(host, |(name, _)| name);
            let name = name.to_ascii_lowercase();
            let reachable = allowed.iter().any(|entry| {
                let entry = entry.to_ascii_lowercase();
                match entry.strip_prefix('.') {
                    Some(domain) => name
                        .strip_suffix(domain)
                        .is_some_and(|rest| rest.ends_with('.')),
                    None => name == entry,
                }
            });
            if !reachable {
                return Err(NOT_ALLOWED);
            }
        }
        let response = client.send(request).map_err(|_| FAILED)?;
        if response.body.len() > request.max_response_bytes {
            return Err(TOO_LARGE);
        }
        Ok(response)
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
        assert_eq!(nothing.send(&request), Err(UNAVAILABLE));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[derive(Debug)]
    struct Echo;
    impl HttpClient for Echo {
        fn send(&self, request: &HttpRequest) -> Result<HttpResponse, String> {
            Ok(HttpResponse {
                status: 200,
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

        let send = |url: &str| {
            access
                .send(&HttpRequest {
                    method: "GET".into(),
                    url: url.into(),
                    headers: Vec::new(),
                    body: Vec::new(),
                    max_response_bytes: 256,
                })
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
}
