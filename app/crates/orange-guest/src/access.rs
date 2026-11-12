//! What a granted plugin instance may reach outside itself: its own data directory and
//! HTTPS requests. The application supplies both; without them a granted import
//! answers `unavailable`.
//!
//! Errors go to the guest as one of the fixed words below, never as operating-system
//! text, so a path or address outside the plugin's reach does not leak through them.
use std::path::{Path, PathBuf};
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

/// Longest path a plugin may name, in bytes.
pub const MAX_PATH_BYTES: usize = 512;
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

/// Names Windows gives to devices in every directory.
fn is_device_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.ends_with(|c: char| c.is_ascii_digit()))
}

/// The path inside `root` that `path` names: `/`-separated plain names, no `.` or `..`,
/// no drive, device or absolute form. `""` is the root itself when `root_allowed`.
fn inside(root: &Path, path: &str, root_allowed: bool) -> Result<PathBuf, &'static str> {
    if path.len() > MAX_PATH_BYTES || (path.is_empty() && !root_allowed) {
        return Err(INVALID_PATH);
    }
    let mut full = root.to_path_buf();
    for name in path.split('/').filter(|_| !path.is_empty()) {
        let plain = !name.is_empty()
            && name != "."
            && name != ".."
            && !name.ends_with(['.', ' '])
            && !name
                .chars()
                .any(|c| c.is_control() || "\\:*?\"<>|".contains(c))
            && !is_device_name(name);
        if !plain {
            return Err(INVALID_PATH);
        }
        full.push(name);
    }
    Ok(full)
}

/// Refuses a path that a link inside the data directory leads out of it.
fn contained(root: &Path, existing: &Path) -> Result<(), &'static str> {
    let root = root.canonicalize().map_err(|_| FAILED)?;
    let real = existing.canonicalize().map_err(|_| FAILED)?;
    if real.starts_with(root) {
        Ok(())
    } else {
        Err(INVALID_PATH)
    }
}

fn io(error: &std::io::Error) -> &'static str {
    if error.kind() == std::io::ErrorKind::NotFound {
        NOT_FOUND
    } else {
        FAILED
    }
}

impl GuestAccess {
    fn root(&self) -> Result<&Path, &'static str> {
        self.data_dir.as_deref().ok_or(UNAVAILABLE)
    }

    pub(crate) fn read(&self, path: &str, max_bytes: usize) -> Result<Vec<u8>, &'static str> {
        let root = self.root()?;
        let full = inside(root, path, false)?;
        let size = std::fs::metadata(&full).map_err(|e| io(&e))?.len();
        if usize::try_from(size).map_or(true, |size| size > max_bytes) {
            return Err(TOO_LARGE);
        }
        contained(root, &full)?;
        std::fs::read(&full).map_err(|e| io(&e))
    }

    pub(crate) fn write(&self, path: &str, data: &[u8]) -> Result<(), &'static str> {
        let root = self.root()?;
        let full = inside(root, path, false)?;
        let parent = full.parent().ok_or(INVALID_PATH)?;
        std::fs::create_dir_all(parent).map_err(|_| FAILED)?;
        contained(root, parent)?;
        // A link at the file's own name would be followed by the write.
        if full.is_symlink() {
            return Err(INVALID_PATH);
        }
        std::fs::write(&full, data).map_err(|_| FAILED)
    }

    pub(crate) fn list(&self, path: &str) -> Result<Vec<String>, &'static str> {
        let root = self.root()?;
        let full = inside(root, path, true)?;
        // Nothing was written yet: the data directory is empty, not missing.
        if path.is_empty() && !full.exists() {
            return Ok(Vec::new());
        }
        contained(root, &full)?;
        let mut names = Vec::new();
        for entry in std::fs::read_dir(&full).map_err(|e| io(&e))? {
            let entry = entry.map_err(|_| FAILED)?;
            let Ok(mut name) = entry.file_name().into_string() else {
                continue;
            };
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                name.push('/');
            }
            names.push(name);
        }
        names.sort();
        Ok(names)
    }

    pub(crate) fn remove(&self, path: &str) -> Result<(), &'static str> {
        let root = self.root()?;
        let full = inside(root, path, false)?;
        let parent = full.parent().ok_or(INVALID_PATH)?;
        if !parent.exists() {
            return Err(NOT_FOUND);
        }
        contained(root, parent)?;
        std::fs::remove_file(&full).map_err(|e| io(&e))
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
    fn only_plain_relative_paths_stay_inside_the_directory() {
        let root = Path::new("data");
        assert_eq!(
            inside(root, "a/b.txt", false),
            Ok(root.join("a").join("b.txt"))
        );
        assert_eq!(inside(root, "", true), Ok(root.to_path_buf()));
        for bad in [
            "",
            "..",
            "a/../b",
            "./a",
            "/etc/passwd",
            "a//b",
            "a/",
            "C:/Windows",
            "a\\b",
            "nul",
            "dir/COM1.txt",
            "trailing.",
            "space ",
            "a\0b",
        ] {
            assert_eq!(inside(root, bad, false), Err(INVALID_PATH), "{bad:?}");
        }
        assert_eq!(
            inside(root, &"a".repeat(MAX_PATH_BYTES + 1), false),
            Err(INVALID_PATH)
        );
    }

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
