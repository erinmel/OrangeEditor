//! `org.orange.text` string vocabulary. Bind once; evaluation reads no locale or host.

use std::sync::Arc;

use orange_model::Value;
use orange_model::catalog::{NodeDefinition, Ports, ProviderKey, RegistrySnapshot};
use orange_model::document::{ConfigValue, DefinitionRef, NodeKind, OutputSource, PortDescriptor};
use orange_model::id::TypeExpr;
use orange_model::value::ListRepr;
use orange_model::value::shape::VALUE_CHARGE;
use serde_json::json;

use crate::{BindingError, CatalogError, KernelError, MAX_COLLECTION_BYTES, MathPackage};

/// Largest string, in UTF-8 bytes, that a text node produces. Larger results fail
/// with `core/LimitExceeded` before the result is built.
pub const MAX_TEXT_BYTES: usize = 16 * 1024 * 1024;

/// Pure string operations. Inputs are strict; every operation is deterministic for a
/// given toolchain (case mapping and White_Space use its Unicode tables).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextOperation {
    ByteLength,
    ScalarLength,
    Concat,
    Contains,
    StartsWith,
    EndsWith,
    Find,
    Substring,
    Replace,
    Trim,
    TrimStart,
    TrimEnd,
    ToUppercase,
    ToLowercase,
    Split,
    Join,
}

/// Index unit of `find` and `substring`: Unicode scalar values or UTF-8 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextUnit {
    Scalar,
    Byte,
}

const STRING: &str = "core/string@1";
const U64: &str = "core/u64@1";
const BOOL: &str = "core/bool@1";
const OPTION_U64: &str = "core/option@1<core/u64@1>";
const LIST: &str = "core/list@1<core/string@1>";

impl TextOperation {
    pub const ALL: [Self; 16] = [
        Self::ByteLength,
        Self::ScalarLength,
        Self::Concat,
        Self::Contains,
        Self::StartsWith,
        Self::EndsWith,
        Self::Find,
        Self::Substring,
        Self::Replace,
        Self::Trim,
        Self::TrimStart,
        Self::TrimEnd,
        Self::ToUppercase,
        Self::ToLowercase,
        Self::Split,
        Self::Join,
    ];

    pub const fn definition_id(self) -> &'static str {
        match self {
            Self::ByteLength => "org.orange.text/byte-length",
            Self::ScalarLength => "org.orange.text/scalar-length",
            Self::Concat => "org.orange.text/concat",
            Self::Contains => "org.orange.text/contains",
            Self::StartsWith => "org.orange.text/starts-with",
            Self::EndsWith => "org.orange.text/ends-with",
            Self::Find => "org.orange.text/find",
            Self::Substring => "org.orange.text/substring",
            Self::Replace => "org.orange.text/replace",
            Self::Trim => "org.orange.text/trim",
            Self::TrimStart => "org.orange.text/trim-start",
            Self::TrimEnd => "org.orange.text/trim-end",
            Self::ToUppercase => "org.orange.text/to-uppercase",
            Self::ToLowercase => "org.orange.text/to-lowercase",
            Self::Split => "org.orange.text/split",
            Self::Join => "org.orange.text/join",
        }
    }

    pub fn from_definition_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|op| op.definition_id() == id)
    }

    /// Strict executable inputs with their types.
    pub const fn inputs(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::ByteLength
            | Self::ScalarLength
            | Self::Trim
            | Self::TrimStart
            | Self::TrimEnd
            | Self::ToUppercase
            | Self::ToLowercase => &[("value", STRING)],
            Self::Concat => &[("a", STRING), ("b", STRING)],
            Self::Contains | Self::StartsWith | Self::EndsWith | Self::Find => {
                &[("text", STRING), ("pattern", STRING)]
            }
            Self::Substring => &[("text", STRING), ("start", U64), ("length", U64)],
            Self::Replace => &[
                ("text", STRING),
                ("pattern", STRING),
                ("replacement", STRING),
            ],
            Self::Split => &[("text", STRING), ("separator", STRING)],
            Self::Join => &[("items", LIST), ("separator", STRING)],
        }
    }

    pub const fn output_port(self) -> &'static str {
        "result"
    }

    pub const fn output_type(self) -> &'static str {
        match self {
            Self::ByteLength | Self::ScalarLength => U64,
            Self::Contains | Self::StartsWith | Self::EndsWith => BOOL,
            Self::Find => OPTION_U64,
            Self::Split => LIST,
            _ => STRING,
        }
    }

    /// Whether the node takes the `unit` config (scalar default, or byte).
    pub const fn has_unit(self) -> bool {
        matches!(self, Self::Find | Self::Substring)
    }
}

/// Ordinary non-generic package definitions in the `org.orange.text` namespace.
pub fn text_definitions() -> Result<Vec<NodeDefinition>, CatalogError> {
    TextOperation::ALL
        .into_iter()
        .map(|operation| {
            let mut ports = operation
                .inputs()
                .iter()
                .map(|(name, ty)| Ok(PortDescriptor::data_in(name, ty.parse::<TypeExpr>()?)))
                .collect::<Result<Vec<_>, CatalogError>>()?;
            ports.push(PortDescriptor::data_out(
                operation.output_port(),
                operation.output_type().parse::<TypeExpr>()?,
                OutputSource::Computed,
            ));
            let properties = if operation.has_unit() {
                json!({"unit":{"enum":["scalar","byte"],"default":"scalar"}})
            } else {
                json!({})
            };
            Ok(NodeDefinition {
                definition: DefinitionRef {
                    id: operation.definition_id().parse()?,
                    version: 1,
                },
                config_version: 1,
                kind: NodeKind::Pure,
                volatility: None,
                determinism: orange_model::catalog::Determinism::Deterministic,
                suspends: None,
                type_params: vec![],
                ports: Ports::Declared(ports),
                references: vec![],
                specializations: None,
                config_schema: Some(json!({"$schema":"orange.config-schema/v1","type":"object",
                "properties":properties,"additionalProperties":false})),
            })
        })
        .collect()
}

/// A pinned exact provider/config binding; no graph-validity claim is implied.
#[derive(Debug, Clone)]
pub struct ResolvedText {
    snapshot: Arc<RegistrySnapshot>,
    provider: ProviderKey,
    operation: TextOperation,
    unit: TextUnit,
}

fn string(value: &Value) -> Result<&str, KernelError> {
    match value {
        Value::Str(text) => Ok(text.as_str()),
        _ => Err(KernelError::InputContract),
    }
}
fn u64_input(value: &Value) -> Result<u64, KernelError> {
    match value {
        Value::U64(value) => Ok(*value),
        _ => Err(KernelError::InputContract),
    }
}
fn text(value: String) -> Value {
    Value::Str(Arc::new(value))
}
fn limit(bytes: usize) -> Result<(), KernelError> {
    if bytes > MAX_TEXT_BYTES {
        Err(KernelError::LimitExceeded)
    } else {
        Ok(())
    }
}
/// Byte offset of scalar index `index`, or the length for `index == count`.
fn scalar_offset(text: &str, index: u64) -> Option<usize> {
    let index = usize::try_from(index).ok()?;
    if index == 0 {
        return Some(0);
    }
    match text.char_indices().nth(index) {
        Some((offset, _)) => Some(offset),
        None => (text.chars().count() == index).then_some(text.len()),
    }
}

impl ResolvedText {
    pub fn snapshot(&self) -> &Arc<RegistrySnapshot> {
        &self.snapshot
    }
    pub fn provider(&self) -> &ProviderKey {
        &self.provider
    }
    pub fn operation(&self) -> TextOperation {
        self.operation
    }
    pub fn unit(&self) -> TextUnit {
        self.unit
    }

    /// Values in declared input order; a wrong type or arity is a contract error.
    /// Bounds are `core/OutOfRange`, a byte index inside a scalar and an empty
    /// pattern/separator are `core/Domain`, oversized results `core/LimitExceeded`.
    pub fn evaluate(&self, inputs: &[Value]) -> Result<Value, KernelError> {
        use TextOperation::*;
        if inputs.len() != self.operation.inputs().len() {
            return Err(KernelError::InputContract);
        }
        let first = || string(&inputs[0]);
        Ok(match self.operation {
            ByteLength => Value::U64(first()?.len() as u64),
            ScalarLength => Value::U64(first()?.chars().count() as u64),
            Concat => {
                let (a, b) = (first()?, string(&inputs[1])?);
                limit(a.len().saturating_add(b.len()))?;
                text([a, b].concat())
            }
            Contains => Value::Bool(first()?.contains(string(&inputs[1])?)),
            StartsWith => Value::Bool(first()?.starts_with(string(&inputs[1])?)),
            EndsWith => Value::Bool(first()?.ends_with(string(&inputs[1])?)),
            Find => {
                let (haystack, pattern) = (first()?, string(&inputs[1])?);
                match haystack.find(pattern) {
                    None => Value::None,
                    Some(offset) => Value::U64(match self.unit {
                        TextUnit::Byte => offset as u64,
                        TextUnit::Scalar => haystack[..offset].chars().count() as u64,
                    }),
                }
            }
            Substring => {
                let source = first()?;
                let (start, length) = (u64_input(&inputs[1])?, u64_input(&inputs[2])?);
                let end = start.checked_add(length).ok_or(KernelError::OutOfRange)?;
                let (from, to) = match self.unit {
                    TextUnit::Byte => {
                        let (from, to) = (
                            usize::try_from(start).map_err(|_| KernelError::OutOfRange)?,
                            usize::try_from(end).map_err(|_| KernelError::OutOfRange)?,
                        );
                        if to > source.len() {
                            return Err(KernelError::OutOfRange);
                        }
                        if !source.is_char_boundary(from) || !source.is_char_boundary(to) {
                            return Err(KernelError::Domain);
                        }
                        (from, to)
                    }
                    TextUnit::Scalar => {
                        let from = scalar_offset(source, start).ok_or(KernelError::OutOfRange)?;
                        let to = scalar_offset(&source[from..], length)
                            .ok_or(KernelError::OutOfRange)?;
                        (from, from + to)
                    }
                };
                text(source[from..to].to_owned())
            }
            Replace => {
                let (source, pattern, replacement) =
                    (first()?, string(&inputs[1])?, string(&inputs[2])?);
                if pattern.is_empty() {
                    return Err(KernelError::Domain);
                }
                if replacement.len() > pattern.len() {
                    let matches = source.matches(pattern).count();
                    limit(
                        (replacement.len() - pattern.len())
                            .saturating_mul(matches)
                            .saturating_add(source.len()),
                    )?;
                }
                text(source.replace(pattern, replacement))
            }
            Trim => text(first()?.trim().to_owned()),
            TrimStart => text(first()?.trim_start().to_owned()),
            TrimEnd => text(first()?.trim_end().to_owned()),
            ToUppercase | ToLowercase => {
                let source = first()?;
                // A scalar maps to at most three scalars of at most four bytes, and
                // never grows more than threefold in UTF-8.
                if source.len().saturating_mul(3) > MAX_TEXT_BYTES {
                    let mapped: usize = source
                        .chars()
                        .map(|c| {
                            if self.operation == ToUppercase {
                                c.to_uppercase().map(char::len_utf8).sum::<usize>()
                            } else {
                                c.to_lowercase().map(char::len_utf8).sum::<usize>()
                            }
                        })
                        .sum();
                    limit(mapped)?;
                }
                text(if self.operation == ToUppercase {
                    source.to_uppercase()
                } else {
                    source.to_lowercase()
                })
            }
            Split => {
                let (source, separator) = (first()?, string(&inputs[1])?);
                if separator.is_empty() {
                    return Err(KernelError::Domain);
                }
                // A split is also a collection construction. Preflight its complete
                // nominal list charge before reserving the vector or any piece strings.
                let mut charge = VALUE_CHARGE;
                let mut count = 0usize;
                for piece in source.split(separator) {
                    count = count.checked_add(1).ok_or(KernelError::LimitExceeded)?;
                    charge = charge
                        .checked_add(VALUE_CHARGE)
                        .and_then(|total| total.checked_add(piece.len()))
                        .filter(|total| *total <= MAX_COLLECTION_BYTES)
                        .ok_or(KernelError::LimitExceeded)?;
                }
                let mut items = Vec::new();
                items
                    .try_reserve_exact(count)
                    .map_err(|_| KernelError::LimitExceeded)?;
                items.extend(
                    source
                        .split(separator)
                        .map(|piece| Arc::new(piece.to_owned())),
                );
                Value::List(Arc::new(ListRepr::Str(items.into())))
            }
            Join => {
                let Value::List(items) = &inputs[0] else {
                    return Err(KernelError::InputContract);
                };
                let separator = string(&inputs[1])?;
                let mut total = separator
                    .len()
                    .saturating_mul(items.len().saturating_sub(1));
                for index in 0..items.len() {
                    let Some(Value::Str(item)) = items.get(index) else {
                        return Err(KernelError::InputContract);
                    };
                    total = total.saturating_add(item.len());
                }
                limit(total)?;
                let mut joined = String::with_capacity(total);
                for index in 0..items.len() {
                    if index > 0 {
                        joined.push_str(separator);
                    }
                    if let Some(Value::Str(item)) = items.get(index) {
                        joined.push_str(&item);
                    }
                }
                text(joined)
            }
        })
    }
}

impl MathPackage {
    /// Resolves only this build's exact `org.orange.text` provider.
    pub fn resolve_text(
        &self,
        snapshot: Arc<RegistrySnapshot>,
        provider: &ProviderKey,
        definition: &DefinitionRef,
        config_version: u32,
        config: &ConfigValue,
    ) -> Result<ResolvedText, BindingError> {
        let operation = TextOperation::from_definition_id(definition.id.as_str())
            .ok_or(BindingError::Unavailable)?;
        if provider != self.text_provider_key()
            || definition.version != 1
            || config_version != 1
            || !snapshot.provides(definition, config_version, provider)
        {
            return Err(BindingError::Unavailable);
        }
        if config.keys().any(|key| key != "unit") || !operation.has_unit() && !config.is_empty() {
            return Err(BindingError::InvalidConfig);
        }
        let unit = match config.get("unit").map(|v| v.as_str()) {
            None | Some(Some("scalar")) => TextUnit::Scalar,
            Some(Some("byte")) => TextUnit::Byte,
            _ => return Err(BindingError::InvalidConfig),
        };
        Ok(ResolvedText {
            snapshot,
            provider: provider.clone(),
            operation,
            unit,
        })
    }
}
