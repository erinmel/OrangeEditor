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
/// The feed is an `https://` URL. Its format and how a host verifies it are not part of
/// the manifest; the manifest only says where to look and which channel this build
/// belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateSource {
    feed: String,
    channel: String,
}

impl UpdateSource {
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
fn url(value: &str, field: &'static str) -> Result<(), ManifestError> {
    let host = value
        .strip_prefix("https://")
        .map(|rest| rest.split(['/', '?', '#']).next().unwrap_or_default());
    let acceptable = value.len() <= MAX_URL_BYTES
        && host.is_some_and(|host| !host.is_empty() && !host.contains('@'))
        && value.chars().all(|c| c.is_ascii_graphic());
    if acceptable {
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
    Ok(UpdateSource {
        feed: wire.feed,
        channel,
    })
}
