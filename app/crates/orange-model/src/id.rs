//! Persisted identifiers (12 §2). Each type can only hold text that satisfies its rule:
//! parsing goes through `FromStr`, and serde goes through `FromStr`/`Display`.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::IdError;

/// Implements string-based serde for a type with `FromStr<Err = IdError>` and `Display`.
macro_rules! serde_via_str {
    ($ty:ty) => {
        impl Serialize for $ty {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $ty {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let text = String::deserialize(deserializer)?;
                text.parse().map_err(serde::de::Error::custom)
            }
        }
    };
}

macro_rules! uuid_id {
    ($($(#[$doc:meta])* $name:ident;)+) => {$(
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(Uuid);

        impl $name {
            /// Wraps a UUID; fails unless it is v4 or v7.
            pub fn from_uuid(uuid: Uuid) -> Result<Self, IdError> {
                match uuid.get_version_num() {
                    4 | 7 => Ok(Self(uuid)),
                    version => Err(IdError::UnsupportedUuidVersion { text: uuid.to_string(), version }),
                }
            }

            pub fn as_uuid(&self) -> Uuid {
                self.0
            }
        }

        impl FromStr for $name {
            type Err = IdError;

            fn from_str(text: &str) -> Result<Self, IdError> {
                Self::from_uuid(parse_canonical_uuid(text)?)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        serde_via_str!($name);
    )+};
}

uuid_id! {
    /// Identity of a whole graph document.
    GraphId;
    /// Node instance; unique across all bodies of a document.
    NodeId;
    /// Edge; unique across all bodies of a document.
    EdgeId;
    VariableId;
    EventDeclId;
    FunctionId;
}

/// Only the lowercase hyphenated form is accepted, so load→save is byte-stable.
fn parse_canonical_uuid(text: &str) -> Result<Uuid, IdError> {
    match Uuid::parse_str(text) {
        Ok(uuid) if uuid.hyphenated().to_string() == text => Ok(uuid),
        _ => Err(IdError::NonCanonicalUuid {
            text: text.to_owned(),
        }),
    }
}

/// Port identifier `[a-z][a-z0-9_]{0,63}`, unique per definition.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PortId(String);

impl PortId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for PortId {
    type Err = IdError;

    fn from_str(text: &str) -> Result<Self, IdError> {
        let mut chars = text.chars();
        let valid = chars.next().is_some_and(|c| c.is_ascii_lowercase())
            && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
            && text.len() <= 64;
        if valid {
            Ok(Self(text.to_owned()))
        } else {
            Err(IdError::InvalidPortId {
                text: text.to_owned(),
            })
        }
    }
}

impl fmt::Display for PortId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

serde_via_str!(PortId);

/// A node's port: `port_id`, or `port_id#k` for a variadic instance port (11 §4.5).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PortKey {
    port: PortId,
    instance: Option<u16>,
}

impl PortKey {
    pub fn new(port: PortId, instance: Option<u16>) -> Self {
        Self { port, instance }
    }

    pub fn port(&self) -> &PortId {
        &self.port
    }

    /// Variadic counter `k`; never reused within a node.
    pub fn instance(&self) -> Option<u16> {
        self.instance
    }
}

impl FromStr for PortKey {
    type Err = IdError;

    fn from_str(text: &str) -> Result<Self, IdError> {
        let invalid = || IdError::InvalidPortKey {
            text: text.to_owned(),
        };
        let (port, instance) = match text.split_once('#') {
            None => (text, None),
            Some((port, k)) => {
                let k: u16 = k.parse().map_err(|_| invalid())?;
                // Reject `+1`, `01` and similar so the key has one spelling.
                if k.to_string() != text[port.len() + 1..] {
                    return Err(invalid());
                }
                (port, Some(k))
            }
        };
        Ok(Self {
            port: port.parse().map_err(|_| invalid())?,
            instance,
        })
    }
}

impl fmt::Display for PortKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.instance {
            None => write!(f, "{}", self.port),
            Some(k) => write!(f, "{}#{k}", self.port),
        }
    }
}

serde_via_str!(PortKey);

/// Node definition identity `namespace/name`; the `core` namespace is reserved for intrinsics.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DefinitionId(String);

impl DefinitionId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn namespace(&self) -> &str {
        self.0
            .split_once('/')
            .map_or("", |(namespace, _)| namespace)
    }

    pub fn is_core(&self) -> bool {
        self.namespace() == "core"
    }
}

impl FromStr for DefinitionId {
    type Err = IdError;

    fn from_str(text: &str) -> Result<Self, IdError> {
        let valid = text.split_once('/').is_some_and(|(namespace, name)| {
            !namespace.is_empty() && !name.is_empty() && !name.contains('/')
        }) && !text.chars().any(char::is_whitespace);
        if valid {
            Ok(Self(text.to_owned()))
        } else {
            Err(IdError::InvalidDefinitionId {
                text: text.to_owned(),
            })
        }
    }
}

impl fmt::Display for DefinitionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

serde_via_str!(DefinitionId);

/// Defines a non-blank opaque string identifier whose syntax a later subunit owns.
macro_rules! opaque_text {
    ($($(#[$doc:meta])* $name:ident => $what:literal;)+) => {$(
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl FromStr for $name {
            type Err = IdError;

            fn from_str(text: &str) -> Result<Self, IdError> {
                if text.is_empty() || text.trim() != text {
                    Err(IdError::Blank { what: $what })
                } else {
                    Ok(Self(text.to_owned()))
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        serde_via_str!($name);
    )+};
}

opaque_text! {
    /// Canonical type expression (11 §5.1). Opaque until the 00c parser/interner replaces
    /// the inside of this type; callers already depend only on this name.
    TypeExpr => "type expression";
    /// Document-local key of a shared signature snapshot (12 §3.1).
    SignatureKey => "signature key";
}

#[cfg(test)]
mod tests {
    use super::*;

    const V4: &str = "a0f3822f-9cf8-4ab6-a2bf-e21fd345c3c6";

    #[test]
    fn uuid_ids_accept_only_canonical_v4_or_v7() {
        assert_eq!(V4.parse::<NodeId>().unwrap().to_string(), V4);
        assert!(
            "0192a3b4-c5d6-7e8f-9a0b-1c2d3e4f5a6b"
                .parse::<NodeId>()
                .is_ok()
        );
        for bad in [
            "n1",
            &V4.to_uppercase(),
            "a0f3822f9cf84ab6a2bfe21fd345c3c6",
            "{a0f3822f-9cf8-4ab6-a2bf-e21fd345c3c6}",
        ] {
            assert!(
                matches!(bad.parse::<NodeId>(), Err(IdError::NonCanonicalUuid { .. })),
                "{bad}"
            );
        }
        assert!(matches!(
            "a0f3822f-9cf8-1ab6-a2bf-e21fd345c3c6".parse::<EdgeId>(),
            Err(IdError::UnsupportedUuidVersion { version: 1, .. })
        ));
    }

    #[test]
    fn port_id_rule() {
        assert!("a".parse::<PortId>().is_ok());
        assert!("line_number2".parse::<PortId>().is_ok());
        assert!("a".repeat(64).parse::<PortId>().is_ok());
        for bad in ["", "A", "1a", "_a", "a-b", "é", &"a".repeat(65)] {
            assert!(bad.parse::<PortId>().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn port_key_rule_and_round_trip() {
        for good in ["a", "item#0", "item#7", "item#65535"] {
            assert_eq!(good.parse::<PortKey>().unwrap().to_string(), good);
        }
        let key: PortKey = "item#3".parse().unwrap();
        assert_eq!((key.port().as_str(), key.instance()), ("item", Some(3)));
        for bad in [
            "a#", "#1", "a#65536", "a#-1", "a#+1", "a#01", "a#1#2", "A#1",
        ] {
            assert!(bad.parse::<PortKey>().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn port_keys_order_by_port_then_instance() {
        let mut keys: Vec<PortKey> = ["b", "a#10", "a", "a#2"]
            .iter()
            .map(|k| k.parse().unwrap())
            .collect();
        keys.sort();
        let sorted: Vec<String> = keys.iter().map(ToString::to_string).collect();
        assert_eq!(sorted, ["a", "a#2", "a#10", "b"]);
    }

    #[test]
    fn definition_id_rule() {
        let core: DefinitionId = "core/branch".parse().unwrap();
        assert!(core.is_core());
        let math: DefinitionId = "org.orange.math/add".parse().unwrap();
        assert_eq!(
            (math.namespace(), math.is_core()),
            ("org.orange.math", false)
        );
        for bad in ["add", "/add", "core/", "a/b/c", "core/on start", ""] {
            assert!(bad.parse::<DefinitionId>().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn opaque_text_rejects_blank() {
        assert!("core/i64@1".parse::<TypeExpr>().is_ok());
        for bad in ["", " core/i64@1", "core/i64@1\n"] {
            assert!(bad.parse::<TypeExpr>().is_err(), "{bad:?}");
        }
    }
}
