//! Update feeds: the document a package's `update.feed` URL serves, announcing the
//! versions its publisher has released.
//!
//! ```json
//! {"format": "orange.update-feed", "format_version": 1,
//!  "releases": [{"id": "org.example.device", "version": "0.2.0", "channel": "stable",
//!    "archive": "https://example.org/device-0.2.0.orangeplug",
//!    "content_digest": "sha256:<64 hex>"}]}
//! ```
//!
//! A release may carry `"target"`, a compiler target such as
//! `x86_64-pc-windows-msvc`: it then announces a native plugin library for that target
//! instead of an archive, and `content_digest` is the SHA-256 of the library file.
//!
//! This module only reads a feed a host has already fetched and answers which release,
//! if any, is newer than a package. It contacts nothing and checks no signature. What
//! makes a downloaded file the announced one is its content digest, which the host
//! compares before installing. What makes the announcement trustworthy is the
//! `https://` origin the package itself named and, when the package names a signing
//! key (`update.key`), an Ed25519 signature over the exact feed bytes, served as 128
//! hexadecimal digits at the feed URL followed by `.sig`; the application verifies it
//! before it reads the feed.

use std::cmp::Ordering;

use serde::Deserialize;

use crate::about::is_https_url;
use crate::manifest::{PackageKey, PackageManifest, PackageVersion};

/// `format` of an update feed.
pub const FEED_FORMAT: &str = "orange.update-feed";
/// Largest feed that is read.
pub const MAX_FEED_BYTES: usize = 1024 * 1024;
/// Most releases one feed lists.
pub const MAX_FEED_RELEASES: usize = 4096;

/// Why a feed was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum FeedError {
    /// The feed is over [`MAX_FEED_BYTES`].
    #[error("the update feed is larger than {MAX_FEED_BYTES} bytes")]
    TooLarge,
    /// Not JSON, not this format and version, or an unknown field.
    #[error("the update feed is not a version 1 `{FEED_FORMAT}` document")]
    Format,
    /// The feed is over [`MAX_FEED_RELEASES`].
    #[error("the update feed lists more than {MAX_FEED_RELEASES} releases")]
    TooManyReleases,
    /// One release has a field that is not valid.
    #[error("release {index} of the update feed has an invalid `{field}`")]
    Release {
        /// Position of the release in the feed.
        index: usize,
        /// The field that was refused.
        field: &'static str,
    },
}

/// One announced package version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    key: PackageKey,
    channel: String,
    archive: String,
    content_digest: String,
    target: Option<String>,
}
impl Release {
    /// The compiler target of a native plugin library release; `None` for an archive.
    pub fn target(&self) -> Option<&str> {
        self.target.as_deref()
    }
    /// The package and version announced.
    pub fn key(&self) -> &PackageKey {
        &self.key
    }
    /// The channel the release belongs to, such as `stable`.
    pub fn channel(&self) -> &str {
        &self.channel
    }
    /// The `https://` URL of the archive.
    pub fn archive(&self) -> &str {
        &self.archive
    }
    /// The content digest the archive must have, `sha256:<64 hex>`.
    pub fn content_digest(&self) -> &str {
        &self.content_digest
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireFeed {
    format: String,
    format_version: u32,
    releases: Vec<WireRelease>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireRelease {
    id: String,
    version: String,
    #[serde(default)]
    channel: Option<String>,
    archive: String,
    content_digest: String,
    #[serde(default)]
    target: Option<String>,
}

fn is_target(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 64
        && text.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_' | b'.')
        })
}

fn is_channel(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 32
        && text
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn is_digest(text: &str) -> bool {
    text.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

/// A parsed update feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateFeed {
    releases: Vec<Release>,
}
impl UpdateFeed {
    /// Reads a feed. Every release is checked; one bad release refuses the feed.
    pub fn from_json(bytes: &[u8]) -> Result<Self, FeedError> {
        if bytes.len() > MAX_FEED_BYTES {
            return Err(FeedError::TooLarge);
        }
        let wire: WireFeed = serde_json::from_slice(bytes).map_err(|_| FeedError::Format)?;
        if wire.format != FEED_FORMAT || wire.format_version != 1 {
            return Err(FeedError::Format);
        }
        if wire.releases.len() > MAX_FEED_RELEASES {
            return Err(FeedError::TooManyReleases);
        }
        let mut releases = Vec::with_capacity(wire.releases.len());
        for (index, release) in wire.releases.into_iter().enumerate() {
            let bad = |field| FeedError::Release { index, field };
            let key = PackageKey {
                id: release.id.parse().map_err(|_| bad("id"))?,
                version: release.version.parse().map_err(|_| bad("version"))?,
            };
            let channel = release.channel.unwrap_or_else(|| "stable".to_owned());
            if !is_channel(&channel) {
                return Err(bad("channel"));
            }
            if !is_https_url(&release.archive) {
                return Err(bad("archive"));
            }
            if !is_digest(&release.content_digest) {
                return Err(bad("content_digest"));
            }
            if release
                .target
                .as_deref()
                .is_some_and(|target| !is_target(target))
            {
                return Err(bad("target"));
            }
            releases.push(Release {
                key,
                channel,
                archive: release.archive,
                content_digest: release.content_digest,
                target: release.target,
            });
        }
        Ok(Self { releases })
    }

    /// Every release the feed lists, in feed order.
    pub fn releases(&self) -> &[Release] {
        &self.releases
    }

    /// The newest release of the installed package's own channel that is newer than
    /// it, or `None` when it is up to date. A package without an `update` section is on
    /// `stable`.
    pub fn update_for(&self, installed: &PackageManifest) -> Option<&Release> {
        let channel = installed
            .update()
            .map_or("stable", |update| update.channel());
        self.newer(installed.key(), channel, None)
    }

    /// The newest release of `channel` that is newer than package `current` and is for
    /// `target`: `None` selects archives, a compiler target selects native plugin
    /// libraries built for it.
    pub fn newer(
        &self,
        current: &PackageKey,
        channel: &str,
        target: Option<&str>,
    ) -> Option<&Release> {
        self.releases
            .iter()
            .filter(|release| {
                release.key.id == current.id
                    && release.channel == channel
                    && release.target.as_deref() == target
            })
            .filter(|release| {
                precedence(&release.key.version, &current.version) == Ordering::Greater
            })
            .max_by(|a, b| precedence(&a.key.version, &b.key.version))
    }
}

fn numeric(a: &str, b: &str) -> Ordering {
    // Neither has a leading zero, so the longer number is the larger.
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}

fn is_number(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

fn parts(version: &PackageVersion) -> (&str, Option<&str>) {
    match version.as_str().split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (version.as_str(), None),
    }
}

/// Semantic Versioning precedence of two package versions: numeric fields compare as
/// numbers, and a pre-release comes before the version it leads to.
pub fn precedence(a: &PackageVersion, b: &PackageVersion) -> Ordering {
    let ((core_a, pre_a), (core_b, pre_b)) = (parts(a), parts(b));
    for (x, y) in core_a.split('.').zip(core_b.split('.')) {
        let order = numeric(x, y);
        if order != Ordering::Equal {
            return order;
        }
    }
    let (pre_a, pre_b) = match (pre_a, pre_b) {
        (None, None) => return Ordering::Equal,
        (None, Some(_)) => return Ordering::Greater,
        (Some(_), None) => return Ordering::Less,
        (Some(pre_a), Some(pre_b)) => (pre_a, pre_b),
    };
    let (mut left, mut right) = (pre_a.split('.'), pre_b.split('.'));
    loop {
        let order = match (left.next(), right.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) => match (is_number(x), is_number(y)) {
                (true, true) => numeric(x.trim_start_matches('0'), y.trim_start_matches('0')),
                (true, false) => Ordering::Less,
                (false, true) => Ordering::Greater,
                (false, false) => x.cmp(y),
            },
        };
        if order != Ordering::Equal {
            return order;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value as Json, json};

    fn version(text: &str) -> PackageVersion {
        text.parse().unwrap()
    }
    const HEX: &str = "2222222222222222222222222222222222222222222222222222222222222222";

    fn manifest(version: &str, channel: Option<&str>) -> PackageManifest {
        let mut manifest = json!({"format": "orange.plugin", "format_version": 1,
            "id": "org.example.device", "version": version, "publisher": "org.example",
            "host_api": {"major": 1, "min_minor": 0, "max_minor": 0},
            "contributions": {},
            "integrity": {"algorithm": "sha256", "content_digest": HEX}});
        if let Some(channel) = channel {
            manifest["update"] =
                json!({"feed": "https://example.org/feed.json", "channel": channel});
        }
        PackageManifest::from_json(manifest.to_string().as_bytes()).unwrap()
    }
    fn feed(releases: &[(&str, &str, &str)]) -> UpdateFeed {
        let releases: Vec<Json> = releases
            .iter()
            .map(|(id, version, channel)| {
                json!({"id": id, "version": version, "channel": channel,
                    "archive": format!("https://example.org/{id}-{version}.orangeplug"),
                    "content_digest": format!("sha256:{HEX}")})
            })
            .collect();
        let text = json!({"format": FEED_FORMAT, "format_version": 1, "releases": releases});
        UpdateFeed::from_json(text.to_string().as_bytes()).unwrap()
    }

    #[test]
    fn versions_compare_as_numbers_and_prereleases_come_first() {
        let ordered = [
            "0.9.0",
            "0.10.0",
            "1.0.0-alpha",
            "1.0.0-alpha.1",
            "1.0.0-alpha.beta",
            "1.0.0-beta.2",
            "1.0.0-beta.11",
            "1.0.0-rc.1",
            "1.0.0",
            "1.0.1",
            "2.0.0",
        ];
        for pair in ordered.windows(2) {
            let (a, b) = (version(pair[0]), version(pair[1]));
            assert_eq!(
                precedence(&a, &b),
                Ordering::Less,
                "{} {}",
                pair[0],
                pair[1]
            );
            assert_eq!(precedence(&b, &a), Ordering::Greater);
        }
        assert_eq!(
            precedence(&version("1.2.3"), &version("1.2.3")),
            Ordering::Equal
        );
    }

    #[test]
    fn the_newest_release_of_the_packages_channel_is_offered() {
        let feed = feed(&[
            ("org.example.device", "0.1.0", "stable"),
            ("org.example.device", "0.2.0", "stable"),
            ("org.example.device", "0.10.0", "stable"),
            ("org.example.device", "0.11.0", "beta"),
            ("org.example.other", "9.0.0", "stable"),
        ]);
        let offered = feed.update_for(&manifest("0.2.0", None)).unwrap();
        assert_eq!(offered.key().version.as_str(), "0.10.0");
        assert_eq!(offered.content_digest(), format!("sha256:{HEX}"));
        assert!(offered.archive().ends_with("device-0.10.0.orangeplug"));

        let beta = manifest("0.2.0", Some("beta"));
        assert_eq!(
            feed.update_for(&beta).unwrap().key().version.as_str(),
            "0.11.0"
        );
        assert_eq!(feed.update_for(&manifest("0.10.0", None)), None);
    }

    #[test]
    fn a_malformed_feed_is_refused_with_the_field_named() {
        let release = |name: &str, value: Json| {
            let mut release = json!({"id": "org.example.device", "version": "0.2.0",
                "archive": "https://example.org/a.orangeplug",
                "content_digest": format!("sha256:{HEX}")});
            release[name] = value;
            let text = json!({"format": FEED_FORMAT, "format_version": 1,
                "releases": [release]});
            UpdateFeed::from_json(text.to_string().as_bytes())
        };
        let field = |result: Result<UpdateFeed, FeedError>| match result {
            Err(FeedError::Release { index: 0, field }) => field,
            other => panic!("{other:?}"),
        };
        let stable = release("channel", "stable".into()).unwrap();
        assert_eq!(stable.releases()[0].channel(), "stable");
        assert_eq!(field(release("version", "1.0".into())), "version");
        assert_eq!(field(release("id", "Not An Id".into())), "id");
        assert_eq!(field(release("channel", "Nightly!".into())), "channel");
        assert_eq!(
            field(release("archive", "http://example.org/a".into())),
            "archive"
        );
        assert_eq!(
            field(release("content_digest", "sha256:abc".into())),
            "content_digest"
        );
        assert_eq!(release("extra", 1.into()), Err(FeedError::Format));
        assert_eq!(UpdateFeed::from_json(b"{}"), Err(FeedError::Format));
        assert_eq!(
            UpdateFeed::from_json(&vec![b' '; MAX_FEED_BYTES + 1]),
            Err(FeedError::TooLarge)
        );
    }

    #[test]
    fn a_library_release_is_offered_only_for_its_target() {
        let digest = format!("sha256:{HEX}");
        let release = |version: &str, target: Option<&str>| {
            let mut release = json!({"id": "org.example.device", "version": version,
                "archive": "https://example.org/device", "content_digest": digest});
            if let Some(target) = target {
                release["target"] = json!(target);
            }
            release
        };
        let document = |releases: Json| {
            json!({"format": FEED_FORMAT, "format_version": 1, "releases": releases}).to_string()
        };
        let feed = UpdateFeed::from_json(
            document(json!([
                release("0.3.0", None),
                release("0.4.0", Some("x86_64-pc-windows-msvc")),
                release("0.5.0", Some("aarch64-apple-darwin")),
            ]))
            .as_bytes(),
        )
        .unwrap();
        let current = manifest("0.2.0", None);
        let newest = |target| {
            feed.newer(current.key(), "stable", target)
                .map(|release| release.key().version.as_str())
        };
        // An installed archive is never offered a library, and the reverse.
        assert_eq!(feed.update_for(&current).unwrap().target(), None);
        assert_eq!(newest(None), Some("0.3.0"));
        assert_eq!(newest(Some("x86_64-pc-windows-msvc")), Some("0.4.0"));
        assert_eq!(newest(Some("aarch64-apple-darwin")), Some("0.5.0"));
        assert_eq!(newest(Some("x86_64-unknown-linux-gnu")), None);
        assert_eq!(
            UpdateFeed::from_json(
                document(json!([release("0.3.0", Some("Not A Target"))])).as_bytes()
            ),
            Err(FeedError::Release {
                index: 0,
                field: "target"
            })
        );
    }

    #[test]
    fn an_update_source_names_an_optional_signing_key() {
        use crate::UpdateSource;
        let key = format!("ed25519:{HEX}");
        let feed = "https://example.org/feed.json";
        let source = UpdateSource::new(feed, None, Some(&key)).unwrap();
        assert_eq!(
            (source.feed(), source.channel(), source.key()),
            (feed, "stable", Some(key.as_str()))
        );
        assert_eq!(
            UpdateSource::new(feed, Some("beta"), None).unwrap().key(),
            None
        );
        for bad in ["ed25519:abc", HEX, "rsa:00", &key.to_uppercase()] {
            assert!(UpdateSource::new(feed, None, Some(bad)).is_err(), "{bad}");
        }
        assert!(UpdateSource::new("http://example.org/feed.json", None, None).is_err());
    }
}
