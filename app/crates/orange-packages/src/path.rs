//! Archive paths: the one portable spelling every manifest reference and archive entry uses
//! (04 "Package format and SDK"). The rules are format rules, identical for every reader, so
//! a path that is valid here unpacks to the same distinct file on Windows, macOS and Linux.

use std::fmt;
use std::str::FromStr;

use orange_model::error::Rejected;
use thiserror::Error;

/// Longest archive path, in bytes.
pub const MAX_PATH_BYTES: usize = 240;
/// Most `/`-separated components in an archive path.
pub const MAX_PATH_DEPTH: usize = 16;

/// Why an archive path is not valid. The path itself is held as [`Rejected`] text.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("invalid archive path {path}: {reason}")]
pub struct PathError {
    /// The path as written.
    pub path: Rejected,
    /// The rule it breaks.
    pub reason: PathRule,
}

/// The archive path rule a path breaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum PathRule {
    /// Empty, or longer than [`MAX_PATH_BYTES`].
    #[error("length")]
    Length,
    /// More than [`MAX_PATH_DEPTH`] components.
    #[error("depth")]
    Depth,
    /// An empty component: a leading, trailing or doubled `/`.
    #[error("empty component")]
    EmptyComponent,
    /// A `.` or `..` component.
    #[error("dot component")]
    DotComponent,
    /// A byte outside `[A-Za-z0-9._-]` and the `/` separator, such as `\`, `:` or a space.
    #[error("character")]
    Character,
    /// A component ending in `.`, which Windows strips.
    #[error("trailing dot")]
    TrailingDot,
    /// A Windows device name such as `con` or `com1`, with or without an extension.
    #[error("reserved name")]
    Reserved,
}

/// A validated archive path: relative, `/`-separated, ASCII `[A-Za-z0-9._-]` components
/// without `.`, `..`, trailing dots or Windows device names. Two paths that differ only in
/// ASCII case have the same [`normalized`](Self::normalized) key and cannot both appear in
/// one archive.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArchivePath(String);

impl ArchivePath {
    /// The path as written.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The key two paths collide on: the ASCII-lowercased path.
    pub fn normalized(&self) -> String {
        self.0.to_ascii_lowercase()
    }

    /// The `/`-separated components.
    pub fn components(&self) -> impl Iterator<Item = &str> {
        self.0.split('/')
    }

    /// Whether the last component ends with `.<extension>`, compared ignoring ASCII case.
    pub fn has_extension(&self, extension: &str) -> bool {
        self.0
            .rsplit_once('.')
            .is_some_and(|(stem, ext)| !stem.ends_with('/') && ext.eq_ignore_ascii_case(extension))
    }
}

impl FromStr for ArchivePath {
    type Err = PathError;

    fn from_str(text: &str) -> Result<Self, PathError> {
        check(text)
            .map(|()| Self(text.to_owned()))
            .map_err(|reason| PathError {
                path: text.into(),
                reason,
            })
    }
}

impl fmt::Display for ArchivePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Shows the length only: an archive path is package-supplied text (13 §2).
impl fmt::Debug for ArchivePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ArchivePath({})", Rejected::from(self.0.as_str()))
    }
}

fn check(text: &str) -> Result<(), PathRule> {
    if text.is_empty() || text.len() > MAX_PATH_BYTES {
        return Err(PathRule::Length);
    }
    if !text
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/'))
    {
        return Err(PathRule::Character);
    }
    let mut depth = 0;
    for component in text.split('/') {
        depth += 1;
        if component.is_empty() {
            return Err(PathRule::EmptyComponent);
        }
        if component == "." || component == ".." {
            return Err(PathRule::DotComponent);
        }
        if component.ends_with('.') {
            return Err(PathRule::TrailingDot);
        }
        if is_device_name(component) {
            return Err(PathRule::Reserved);
        }
    }
    if depth > MAX_PATH_DEPTH {
        return Err(PathRule::Depth);
    }
    Ok(())
}

/// Windows device names, matched on the part before the first `.` ignoring case.
fn is_device_name(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or_default();
    let upper = stem.to_ascii_uppercase();
    match upper.as_bytes() {
        b"CON" | b"PRN" | b"AUX" | b"NUL" | b"CONIN$" | b"CONOUT$" => true,
        [b'C', b'O', b'M', digit] | [b'L', b'P', b'T', digit] => digit.is_ascii_digit(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(text: &str) -> PathRule {
        text.parse::<ArchivePath>().unwrap_err().reason
    }

    #[test]
    fn accepts_portable_relative_paths() {
        for text in [
            "manifest.json",
            "locales/en-US.ftl",
            "bin/x86_64/worker",
            ".hidden/a_b",
        ] {
            assert_eq!(text.parse::<ArchivePath>().unwrap().as_str(), text);
        }
    }

    #[test]
    fn rejects_traversal_and_platform_aliases() {
        assert_eq!(rule(""), PathRule::Length);
        assert_eq!(rule(&"a".repeat(MAX_PATH_BYTES + 1)), PathRule::Length);
        assert_eq!(rule(&["a"; MAX_PATH_DEPTH + 1].join("/")), PathRule::Depth);
        assert_eq!(rule("/etc/passwd"), PathRule::EmptyComponent);
        assert_eq!(rule("a//b"), PathRule::EmptyComponent);
        assert_eq!(rule("a/"), PathRule::EmptyComponent);
        assert_eq!(rule("../a"), PathRule::DotComponent);
        assert_eq!(rule("a/./b"), PathRule::DotComponent);
        assert_eq!(rule("a\\..\\b"), PathRule::Character);
        assert_eq!(rule("C:/a"), PathRule::Character);
        assert_eq!(rule("a:stream"), PathRule::Character);
        assert_eq!(rule("a b"), PathRule::Character);
        assert_eq!(rule("caf\u{e9}"), PathRule::Character);
        assert_eq!(rule("a."), PathRule::TrailingDot);
        assert_eq!(rule("dir./a"), PathRule::TrailingDot);
        assert_eq!(rule("nul"), PathRule::Reserved);
        assert_eq!(rule("x/Com1.txt"), PathRule::Reserved);
        assert_eq!(rule("lpt9"), PathRule::Reserved);
    }

    #[test]
    fn case_variants_share_a_normalized_key() {
        let upper: ArchivePath = "Locales/EN.ftl".parse().unwrap();
        let lower: ArchivePath = "locales/en.ftl".parse().unwrap();
        assert_ne!(upper, lower);
        assert_eq!(upper.normalized(), lower.normalized());
        assert!(upper.has_extension("ftl"));
        assert!(
            !"a/.ftl"
                .parse::<ArchivePath>()
                .unwrap()
                .has_extension("ftl")
        );
    }
}
