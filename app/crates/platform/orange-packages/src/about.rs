//! What a package says about itself and where newer versions are announced: the
//! optional `about` and `update` manifest sections.
//!
//! Both are descriptive. Nothing here is fetched, executed or trusted: a URL is only
//! checked for shape, and the host decides whether and when to contact an update feed.
//! `name` and `description` are fallbacks for hosts without the package's localization
//! catalogue (13 §3).

use serde::Deserialize;

use crate::manifest::ManifestError;

/// Longest `about.name`, in characters.
pub const MAX_NAME_CHARS: usize = 100;
/// Longest `about.description`, in characters.
pub const MAX_DESCRIPTION_CHARS: usize = 2000;
/// Most `about.authors` and `about.keywords` entries.
pub const MAX_ABOUT_ITEMS: usize = 16;
/// Longest URL, in bytes.
pub const MAX_URL_BYTES: usize = 2048;

/// The package's human-facing description.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct About {
    name: Option<String>,
    description: Option<String>,
    authors: Vec<String>,
    homepage: Option<String>,
    repository: Option<String>,
    keywords: Vec<String>,
}

impl About {
    /// Display name, for a host that has no localized one.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
    /// One paragraph on what the package adds.
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }
    /// People or organizations, as free text.
    pub fn authors(&self) -> &[String] {
        &self.authors
    }
    /// An `https://` page for users.
    pub fn homepage(&self) -> Option<&str> {
        self.homepage.as_deref()
    }
    /// An `https://` page for the package's source.
    pub fn repository(&self) -> Option<&str> {
        self.repository.as_deref()
    }
    /// Lowercase search terms, `[a-z0-9-]`, distinct.
    pub fn keywords(&self) -> &[String] {
        &self.keywords
    }
}

/// Where the publisher announces versions of this package.
///
/// The feed is an `https://` URL. The manifest says where to look, which channel this
/// build belongs to and, with `key`, which publisher key must have signed the feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateSource {
    feed: String,
    channel: String,
    key: Option<String>,
}

impl UpdateSource {
    /// An update source outside a manifest, under the manifest's rules: an `https://`
    /// feed URL, a channel (`stable` when `None`) and an optional signing key.
    pub fn new(
        feed: &str,
        channel: Option<&str>,
        key: Option<&str>,
    ) -> Result<Self, ManifestError> {
        update(WireUpdate {
            feed: feed.to_owned(),
            channel: channel.map(str::to_owned),
            key: key.map(str::to_owned),
        })
    }
    /// The publisher's feed-signing key, `ed25519:<64 hex>`: the Ed25519 public key
    /// that must have signed the feed. `None` when the package accepts an unsigned
    /// feed, trusting its `https://` origin alone.
    pub fn key(&self) -> Option<&str> {
        self.key.as_deref()
    }
    /// The feed URL.
    pub fn feed(&self) -> &str {
        &self.feed
    }
    /// `stable` unless the manifest names another channel, such as `beta`.
    pub fn channel(&self) -> &str {
        &self.channel
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireAbout {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    authors: Vec<String>,
    #[serde(default)]
    homepage: Option<String>,
    #[serde(default)]
    repository: Option<String>,
    #[serde(default)]
    keywords: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireUpdate {
    feed: String,
    #[serde(default)]
    channel: Option<String>,
    #[serde(default)]
    key: Option<String>,
}

/// `ed25519:` and 64 lowercase hexadecimal digits.
pub(crate) fn is_signing_key(value: &str) -> bool {
    value.strip_prefix("ed25519:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

fn refused(field: &'static str) -> ManifestError {
    ManifestError::Metadata { field }
}

/// Non-empty text of at most `max` characters with no control characters and no
/// leading or trailing space.
fn text(value: &str, max: usize, field: &'static str) -> Result<(), ManifestError> {
    let acceptable = !value.is_empty()
        && value.trim() == value
        && value.chars().count() <= max
        && !value.chars().any(char::is_control);
    if acceptable {
        Ok(())
    } else {
        Err(refused(field))
    }
}

/// An `https://` URL with a host and nothing a terminal or log could misread.
/// An `https://` URL of printable ASCII with a host and no user information, within
/// [`MAX_URL_BYTES`].
pub(crate) fn is_https_url(value: &str) -> bool {
    let host = value
        .strip_prefix("https://")
        .map(|rest| rest.split(['/', '?', '#']).next().unwrap_or_default());
    value.len() <= MAX_URL_BYTES
        && host.is_some_and(|host| !host.is_empty() && !host.contains('@'))
        && value.chars().all(|c| c.is_ascii_graphic())
}

fn url(value: &str, field: &'static str) -> Result<(), ManifestError> {
    if is_https_url(value) {
        Ok(())
    } else {
        Err(refused(field))
    }
}

/// `[a-z0-9]` words joined by single hyphens, at most 32 bytes.
fn slug(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 32
        && value.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

pub(crate) fn about(wire: WireAbout) -> Result<About, ManifestError> {
    if let Some(name) = &wire.name {
        text(name, MAX_NAME_CHARS, "about.name")?;
    }
    if let Some(description) = &wire.description {
        text(description, MAX_DESCRIPTION_CHARS, "about.description")?;
    }
    if wire.authors.len() > MAX_ABOUT_ITEMS {
        return Err(refused("about.authors"));
    }
    for author in &wire.authors {
        text(author, MAX_NAME_CHARS, "about.authors")?;
    }
    if let Some(homepage) = &wire.homepage {
        url(homepage, "about.homepage")?;
    }
    if let Some(repository) = &wire.repository {
        url(repository, "about.repository")?;
    }
    let distinct: std::collections::BTreeSet<&String> = wire.keywords.iter().collect();
    if wire.keywords.len() > MAX_ABOUT_ITEMS
        || distinct.len() != wire.keywords.len()
        || !wire.keywords.iter().all(|keyword| slug(keyword))
    {
        return Err(refused("about.keywords"));
    }
    Ok(About {
        name: wire.name,
        description: wire.description,
        authors: wire.authors,
        homepage: wire.homepage,
        repository: wire.repository,
        keywords: wire.keywords,
    })
}

pub(crate) fn update(wire: WireUpdate) -> Result<UpdateSource, ManifestError> {
    url(&wire.feed, "update.feed")?;
    let channel = wire.channel.unwrap_or_else(|| "stable".to_owned());
    if !slug(&channel) || channel.starts_with(|c: char| c.is_ascii_digit()) {
        return Err(refused("update.channel"));
    }
    if wire.key.as_deref().is_some_and(|key| !is_signing_key(key)) {
        return Err(refused("update.key"));
    }
    Ok(UpdateSource {
        feed: wire.feed,
        channel,
        key: wire.key,
    })
}
