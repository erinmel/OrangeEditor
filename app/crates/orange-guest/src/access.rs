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
        self.folder()?.write(path, data).map_err(word)
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
            http: None,
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
}
