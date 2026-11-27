//! Versioned saved files and the steps that bring an older one forward (12 §6.3).
//!
//! Every file the product saves starts with a `"format"` name and a `"format_version"`
//! number. A [`History`] lists, for one format, the oldest version this build reads and
//! one [`Step`] per later version. Reading a file runs the steps from its version up to
//! the current one, in order; writing always uses the current version. A file newer than
//! this build, or older than its oldest version, is refused with its version, never
//! guessed at.
//!
//! Steps work on JSON, before the file is decoded into Rust types, so a step can rename,
//! move or drop fields that the current types no longer have. Each step is a pure function
//! of the file it is given.

use serde_json::{Map, Value as Json};
use thiserror::Error;

use crate::error::Rejected;

/// The field that names a saved file's format.
pub const FORMAT_FIELD: &str = "format";
/// The field that holds a saved file's format version.
pub const VERSION_FIELD: &str = "format_version";

/// Brings a file of version `n` to version `n + 1`. It receives the whole top-level
/// object, envelope fields included, and must leave `format` alone; the version field is
/// set by [`History`] after the step returns.
pub type Step = fn(&mut Map<String, Json>) -> Result<(), StepError>;

/// Why a step could not upgrade a file. `reason` says what the file lacked, in English, for
/// logs; hosts show [`FormatError::message_key`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("{reason}")]
pub struct StepError {
    pub reason: &'static str,
}

/// The versions of one saved format: the oldest one this build reads and one step per
/// later version, so the current version is `oldest + steps.len()`.
#[derive(Debug, Clone, Copy)]
pub struct History {
    format: &'static str,
    oldest: u32,
    steps: &'static [Step],
}

/// Whether a file was read at the current version or brought forward from an older one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Upgrade {
    Current,
    /// The file was saved at version `from` and has been upgraded in memory; saving it
    /// writes the current version.
    From(u32),
}

impl Upgrade {
    /// The version the file was saved at, when it was older than the current one.
    pub const fn from_version(self) -> Option<u32> {
        match self {
            Self::Current => None,
            Self::From(version) => Some(version),
        }
    }
}

/// Why a saved file's envelope or upgrade was refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum FormatError {
    #[error("the file is not JSON")]
    NotJson,
    #[error("the file is not a JSON object")]
    NotAnObject,
    #[error("the file does not say its format")]
    MissingFormat,
    #[error("the file is not a `{expected}` file")]
    WrongFormat {
        expected: &'static str,
        found: Rejected,
    },
    #[error("the file does not say its format version")]
    MissingVersion,
    /// Newer than this build, or older than the oldest version it reads.
    #[error("format version {found} is not readable (this build reads {oldest} to {current})")]
    UnsupportedVersion {
        found: u32,
        oldest: u32,
        current: u32,
    },
    #[error("upgrading from format version {from} failed: {reason}")]
    UpgradeFailed { from: u32, reason: StepError },
}

impl FormatError {
    /// Stable message key for host presentation (13 §2).
    pub const fn message_key(&self) -> &'static str {
        match self {
            Self::NotJson | Self::NotAnObject => "core/file-not-json",
            Self::MissingFormat | Self::WrongFormat { .. } => "core/file-wrong-format",
            Self::MissingVersion | Self::UnsupportedVersion { .. } => {
                "core/file-unsupported-version"
            }
            Self::UpgradeFailed { .. } => "core/file-upgrade-failed",
        }
    }
}

impl History {
    /// The history of `format`, reading versions `oldest` to `oldest + steps.len()`, where
    /// `steps[i]` upgrades version `oldest + i`.
    pub const fn new(format: &'static str, oldest: u32, steps: &'static [Step]) -> Self {
        Self {
            format,
            oldest,
            steps,
        }
    }

    pub const fn format(&self) -> &'static str {
        self.format
    }

    pub const fn oldest(&self) -> u32 {
        self.oldest
    }

    /// The version every file of this format is written at.
    pub fn current(&self) -> u32 {
        let steps = u32::try_from(self.steps.len()).unwrap_or(u32::MAX);
        self.oldest.saturating_add(steps)
    }

    /// Checks a file's format name and version. `Ok(Upgrade::From(v))` means the file is
    /// readable once the steps from `v` have run.
    pub fn check(&self, format: &str, version: u32) -> Result<Upgrade, FormatError> {
        if format != self.format {
            return Err(FormatError::WrongFormat {
                expected: self.format,
                found: format.into(),
            });
        }
        let current = self.current();
        if version == current {
            Ok(Upgrade::Current)
        } else if (self.oldest..current).contains(&version) {
            Ok(Upgrade::From(version))
        } else {
            Err(FormatError::UnsupportedVersion {
                found: version,
                oldest: self.oldest,
                current,
            })
        }
    }

    /// Checks the envelope of `file` and runs every step from its version, leaving it at
    /// the current version. A file that fails a step is left as that step left it.
    pub fn upgrade(&self, file: &mut Map<String, Json>) -> Result<Upgrade, FormatError> {
        let format = match file.get(FORMAT_FIELD) {
            None => return Err(FormatError::MissingFormat),
            Some(Json::String(format)) => format.as_str(),
            Some(other) => {
                return Err(FormatError::WrongFormat {
                    expected: self.format,
                    found: other.to_string().into(),
                });
            }
        };
        let version = file
            .get(VERSION_FIELD)
            .ok_or(FormatError::MissingVersion)?
            .as_u64()
            .and_then(|version| u32::try_from(version).ok())
            .ok_or(FormatError::MissingVersion)?;
        let upgrade = self.check(format, version)?;
        if let Upgrade::From(from) = upgrade {
            let first = usize::try_from(from - self.oldest).unwrap_or(usize::MAX);
            for (step, at) in self.steps.iter().skip(first).zip(from..) {
                step(file).map_err(|reason| FormatError::UpgradeFailed { from: at, reason })?;
                let _ = file.insert(VERSION_FIELD.to_owned(), Json::from(at + 1));
            }
        }
        Ok(upgrade)
    }

    /// Parses `text` as a file of this format and upgrades it to the current version.
    pub fn read(&self, text: &str) -> Result<(Map<String, Json>, Upgrade), FormatError> {
        let file: Json = serde_json::from_str(text).map_err(|_| FormatError::NotJson)?;
        let Json::Object(mut file) = file else {
            return Err(FormatError::NotAnObject);
        };
        let upgrade = self.upgrade(&mut file)?;
        Ok((file, upgrade))
    }

    /// A file of this format at the current version holding `body`'s fields after the
    /// envelope. Envelope fields in `body` are replaced.
    pub fn wrap(&self, body: Map<String, Json>) -> Json {
        let mut file = Map::with_capacity(body.len() + 2);
        let _ = file.insert(FORMAT_FIELD.to_owned(), Json::from(self.format));
        let _ = file.insert(VERSION_FIELD.to_owned(), Json::from(self.current()));
        for (key, value) in body {
            if key != FORMAT_FIELD && key != VERSION_FIELD {
                let _ = file.insert(key, value);
            }
        }
        Json::Object(file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Version 1 called the list `items`; version 2 renamed it `entries`; version 3
    /// requires each entry to have a `weight`.
    fn rename_items(file: &mut Map<String, Json>) -> Result<(), StepError> {
        let items = file.remove("items").ok_or(StepError {
            reason: "no `items` list",
        })?;
        let _ = file.insert("entries".to_owned(), items);
        Ok(())
    }

    fn add_weights(file: &mut Map<String, Json>) -> Result<(), StepError> {
        let Some(Json::Array(entries)) = file.get_mut("entries") else {
            return Err(StepError {
                reason: "no `entries` list",
            });
        };
        for entry in entries.iter_mut().filter_map(Json::as_object_mut) {
            let _ = entry.entry("weight").or_insert(json!(1));
        }
        Ok(())
    }

    const LIST: History = History::new("test.list", 1, &[rename_items, add_weights]);

    fn object(value: Json) -> Map<String, Json> {
        match value {
            Json::Object(map) => map,
            _ => unreachable!("tests pass objects"),
        }
    }

    #[test]
    fn the_current_version_is_the_oldest_plus_one_per_step() {
        assert_eq!(LIST.current(), 3);
        assert_eq!(History::new("test.flat", 1, &[]).current(), 1);
    }

    #[test]
    fn an_old_file_runs_every_later_step_in_order() {
        let mut file = object(json!({"format": "test.list", "format_version": 1,
            "items": [{"name": "a"}, {"name": "b", "weight": 5}]}));
        assert_eq!(LIST.upgrade(&mut file), Ok(Upgrade::From(1)));
        assert_eq!(
            Json::Object(file),
            json!({"format": "test.list", "format_version": 3,
                "entries": [{"name": "a", "weight": 1}, {"name": "b", "weight": 5}]})
        );

        // A version-2 file skips the first step.
        let mut file = object(json!({"format": "test.list", "format_version": 2,
            "entries": [{"name": "c"}]}));
        assert_eq!(LIST.upgrade(&mut file).unwrap().from_version(), Some(2));
        assert_eq!(file["entries"], json!([{"name": "c", "weight": 1}]));
    }

    #[test]
    fn a_current_file_is_left_as_it_is() {
        let original = json!({"format": "test.list", "format_version": 3, "entries": []});
        let mut file = object(original.clone());
        assert_eq!(LIST.upgrade(&mut file), Ok(Upgrade::Current));
        assert_eq!(Json::Object(file), original);
    }

    #[test]
    fn newer_older_and_foreign_files_are_refused_with_what_they_say() {
        let refused = |value: Json| LIST.upgrade(&mut object(value)).unwrap_err();
        let range = |found| FormatError::UnsupportedVersion {
            found,
            oldest: 1,
            current: 3,
        };
        assert_eq!(
            refused(json!({"format": "test.list", "format_version": 4})),
            range(4)
        );
        assert_eq!(
            refused(json!({"format": "test.list", "format_version": 0})),
            range(0)
        );
        assert!(matches!(
            refused(json!({"format": "test.other", "format_version": 3})),
            FormatError::WrongFormat {
                expected: "test.list",
                ..
            }
        ));
        assert_eq!(
            refused(json!({"format_version": 3})),
            FormatError::MissingFormat
        );
        assert_eq!(
            refused(json!({"format": "test.list", "format_version": "3"})),
            FormatError::MissingVersion
        );
        assert_eq!(
            refused(json!({"format": "test.list", "format_version": 1})),
            FormatError::UpgradeFailed {
                from: 1,
                reason: StepError {
                    reason: "no `items` list"
                }
            }
        );
    }

    #[test]
    fn reading_parses_and_upgrades_and_wrapping_writes_the_current_envelope_first() {
        assert_eq!(LIST.read("[]").unwrap_err(), FormatError::NotAnObject);
        assert_eq!(LIST.read("{").unwrap_err(), FormatError::NotJson);
        let (file, upgrade) = LIST
            .read(r#"{"format":"test.list","format_version":2,"entries":[]}"#)
            .unwrap();
        assert_eq!(upgrade, Upgrade::From(2));
        let mut body = file;
        let _ = body.insert("format_version".to_owned(), json!(99));
        let written = LIST.wrap(body);
        assert_eq!(
            written.to_string(),
            r#"{"format":"test.list","format_version":3,"entries":[]}"#
        );
    }
}
