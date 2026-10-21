use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::ConverterRegistrationError;
use crate::error::IdError;
use crate::id::{DefinitionId, TypeExpr};

/// The behavior class promised by one direct converter entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConversionClass {
    /// Always succeeds without information loss.
    LosslessTotal,
    /// Can fail through a declared [`ConverterErrorCode`].
    CheckedPartial,
    /// May lose information and exposes a `loss: core/json@1` output.
    Lossy,
}

/// Stable semantic identity for a converter contract.
///
/// The namespace follows the authority that registers the semantic entry. A provider
/// override keeps this identity and contract version; it does not register a new entry.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConverterId(DefinitionId);

impl ConverterId {
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub fn namespace(&self) -> &str {
        self.0.namespace()
    }
}

impl FromStr for ConverterId {
    type Err = IdError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        value.parse().map(Self)
    }
}

impl fmt::Display for ConverterId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl Serialize for ConverterId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ConverterId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

/// One published semantic entry, including its contract version.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ConverterKey {
    pub id: ConverterId,
    pub contract_version: u32,
}

impl fmt::Display for ConverterKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.id, self.contract_version)
    }
}

/// A stable namespaced error emitted by a checked or lossy converter.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ConverterErrorCode(String);

impl ConverterErrorCode {
    pub fn new(code: impl Into<String>) -> Result<Self, ConverterRegistrationError> {
        let value = Self(code.into());
        value.validate()?;
        Ok(value)
    }

    pub(super) fn validate(&self) -> Result<(), ConverterRegistrationError> {
        let Some((namespace, name)) = self.0.split_once('/') else {
            return Err(ConverterRegistrationError::InvalidErrorCode {
                code: self.0.clone(),
            });
        };
        if namespace.is_empty()
            || name.is_empty()
            || self.0.chars().any(char::is_whitespace)
            || name.contains('/')
        {
            return Err(ConverterRegistrationError::InvalidErrorCode {
                code: self.0.clone(),
            });
        }
        Ok(())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ConverterErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A finite, named policy whose values are string choices.
///
/// `default: None` means the policy is required for a new converter binding. This is a
/// deliberately small policy form, not a general JSON Schema. A converter requiring a
/// different policy shape is rejected by this version of the registry.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct NamedPolicy {
    pub name: String,
    pub choices: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

/// Metadata for one direct converter semantic contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConverterDefinition {
    pub id: ConverterId,
    pub contract_version: u32,
    pub from: TypeExpr,
    pub to: TypeExpr,
    pub class: ConversionClass,
    #[serde(default)]
    pub policies: Vec<NamedPolicy>,
    #[serde(default)]
    pub errors: Vec<ConverterErrorCode>,
}

impl ConverterDefinition {
    pub fn key(&self) -> ConverterKey {
        ConverterKey {
            id: self.id.clone(),
            contract_version: self.contract_version,
        }
    }

    /// The visible loss metadata port required by every lossy converter.
    pub fn loss_output_type(&self) -> Option<TypeExpr> {
        (self.class == ConversionClass::Lossy)
            .then(|| "core/json@1".parse().expect("core/json@1 is canonical"))
    }

    /// Validates and fills policy defaults. Unknown options and choices are rejected.
    pub fn resolve_options(
        &self,
        supplied: &BTreeMap<String, String>,
    ) -> Result<BTreeMap<String, String>, ConverterRegistrationError> {
        let declared: BTreeSet<&str> = self
            .policies
            .iter()
            .map(|policy| policy.name.as_str())
            .collect();
        if let Some(name) = supplied
            .keys()
            .find(|name| !declared.contains(name.as_str()))
        {
            return Err(ConverterRegistrationError::UnknownPolicy {
                entry: self.key(),
                name: name.clone(),
            });
        }

        let mut resolved = BTreeMap::new();
        for policy in &self.policies {
            let value = supplied
                .get(&policy.name)
                .or(policy.default.as_ref())
                .ok_or_else(|| ConverterRegistrationError::MissingPolicy {
                    entry: self.key(),
                    name: policy.name.clone(),
                })?;
            if !policy.choices.iter().any(|choice| choice == value) {
                return Err(ConverterRegistrationError::UnknownPolicyChoice {
                    entry: self.key(),
                    name: policy.name.clone(),
                    value: value.clone(),
                });
            }
            resolved.insert(policy.name.clone(), value.clone());
        }
        Ok(resolved)
    }
}
