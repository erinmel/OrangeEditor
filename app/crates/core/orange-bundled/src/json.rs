//! `org.orange.json` read vocabulary over validated `core/json@1` text. Navigation works
//! on the text itself, so extracted values keep their exact number text (03).

use std::sync::Arc;

use orange_model::Value;
use orange_model::catalog::{NodeDefinition, Ports, ProviderKey, RegistrySnapshot};
use orange_model::document::{ConfigValue, DefinitionRef, NodeKind, OutputSource, PortDescriptor};
use orange_model::id::TypeExpr;
use orange_model::value::JsonText;
use serde_json::json;

use crate::{BindingError, CatalogError, KernelError, MathPackage};

/// Pure JSON reads. Inputs are strict; results never invent null or missing values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonOperation {
    Kind,
    IsNull,
    Get,
    At,
    Length,
    AsBool,
    AsString,
    AsI64,
    AsU64,
    AsF64,
}

const JSON: &str = "core/json@1";

impl JsonOperation {
    pub const ALL: [Self; 10] = [
        Self::Kind,
        Self::IsNull,
        Self::Get,
        Self::At,
        Self::Length,
        Self::AsBool,
        Self::AsString,
        Self::AsI64,
        Self::AsU64,
        Self::AsF64,
    ];

    pub const fn definition_id(self) -> &'static str {
        match self {
            Self::Kind => "org.orange.json/kind",
            Self::IsNull => "org.orange.json/is-null",
            Self::Get => "org.orange.json/get",
            Self::At => "org.orange.json/at",
            Self::Length => "org.orange.json/length",
            Self::AsBool => "org.orange.json/as-bool",
            Self::AsString => "org.orange.json/as-string",
            Self::AsI64 => "org.orange.json/as-i64",
            Self::AsU64 => "org.orange.json/as-u64",
            Self::AsF64 => "org.orange.json/as-f64",
        }
    }

    pub fn from_definition_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|op| op.definition_id() == id)
    }

    pub const fn inputs(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Get => &[("value", JSON), ("key", "core/string@1")],
            Self::At => &[("value", JSON), ("index", "core/u64@1")],
            _ => &[("value", JSON)],
        }
    }

    pub const fn output_type(self) -> &'static str {
        match self {
            Self::Kind => "core/string@1",
            Self::IsNull => "core/bool@1",
            Self::Get | Self::At => "core/option@1<core/json@1>",
            Self::Length => "core/u64@1",
            Self::AsBool => "core/option@1<core/bool@1>",
            Self::AsString => "core/option@1<core/string@1>",
            Self::AsI64 => "core/option@1<core/i64@1>",
            Self::AsU64 => "core/option@1<core/u64@1>",
            Self::AsF64 => "core/option@1<core/f64@1>",
        }
    }
}

/// Ordinary non-generic definitions in the `org.orange.json` namespace.
pub fn json_definitions() -> Result<Vec<NodeDefinition>, CatalogError> {
    JsonOperation::ALL
        .into_iter()
        .map(|operation| {
            let mut ports = operation
                .inputs()
                .iter()
                .map(|(name, ty)| Ok(PortDescriptor::data_in(name, ty.parse::<TypeExpr>()?)))
                .collect::<Result<Vec<_>, CatalogError>>()?;
            ports.push(PortDescriptor::data_out(
                "result",
                operation.output_type().parse::<TypeExpr>()?,
                OutputSource::Computed,
            ));
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
                "properties":{},"additionalProperties":false})),
            })
        })
        .collect()
}

/// Byte scanner over text already accepted by [`JsonText`]; it never re-validates.
struct Scan<'a> {
    text: &'a [u8],
    at: usize,
}

impl<'a> Scan<'a> {
    fn new(text: &'a str) -> Self {
        let mut scan = Self {
            text: text.as_bytes(),
            at: 0,
        };
        scan.skip_space();
        scan
    }
    fn skip_space(&mut self) {
        while matches!(self.text.get(self.at), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }
    fn peek(&self) -> Option<u8> {
        self.text.get(self.at).copied()
    }
    /// Advances past one complete value and returns its byte span.
    fn value(&mut self) -> (usize, usize) {
        let start = self.at;
        match self.peek() {
            Some(b'"') => self.string(),
            Some(b'[' | b'{') => {
                let mut depth = 0usize;
                while let Some(byte) = self.peek() {
                    match byte {
                        b'"' => {
                            self.string();
                            continue;
                        }
                        b'[' | b'{' => depth += 1,
                        b']' | b'}' => {
                            depth -= 1;
                            if depth == 0 {
                                self.at += 1;
                                break;
                            }
                        }
                        _ => {}
                    }
                    self.at += 1;
                }
            }
            _ => {
                while !matches!(
                    self.peek(),
                    None | Some(b',' | b']' | b'}' | b' ' | b'\t' | b'\n' | b'\r')
                ) {
                    self.at += 1;
                }
            }
        }
        (start, self.at)
    }
    fn string(&mut self) {
        self.at += 1;
        while let Some(byte) = self.peek() {
            self.at += 1;
            match byte {
                b'\\' => self.at += 1,
                b'"' => return,
                _ => {}
            }
        }
    }
    /// Steps over `[` / `{`, then reports whether the container is empty.
    fn open(&mut self) -> bool {
        self.at += 1;
        self.skip_space();
        matches!(self.peek(), Some(b']' | b'}'))
    }
    /// After an element: true if another follows (a `,` was consumed).
    fn next(&mut self) -> bool {
        self.skip_space();
        if self.peek() == Some(b',') {
            self.at += 1;
            self.skip_space();
            true
        } else {
            false
        }
    }
    /// An object key at the cursor, unescaped, then steps past the `:`.
    fn key(&mut self) -> Result<String, KernelError> {
        let (start, end) = self.value();
        let raw =
            std::str::from_utf8(&self.text[start..end]).map_err(|_| KernelError::InputContract)?;
        self.skip_space();
        self.at += 1; // ':'
        self.skip_space();
        if raw[1..raw.len() - 1].contains('\\') {
            serde_json::from_str(raw).map_err(|_| KernelError::InputContract)
        } else {
            Ok(raw[1..raw.len() - 1].to_owned())
        }
    }
    fn slice(&self, (start, end): (usize, usize)) -> &'a str {
        // Spans start and end on ASCII delimiters of valid UTF-8 text.
        std::str::from_utf8(&self.text[start..end]).unwrap_or_default()
    }
}

fn json_value(text: &str) -> Result<Value, KernelError> {
    let text = JsonText::try_from(text).map_err(|_| KernelError::InputContract)?;
    Ok(Value::Json(Arc::new(text)))
}

/// Object members as (unescaped key, value byte span), in text order.
type Members = Vec<(String, (usize, usize))>;

/// The members of an object at the cursor, in text order.
fn members(scan: &mut Scan<'_>) -> Result<Members, KernelError> {
    let mut members = Vec::new();
    if scan.open() {
        return Ok(members);
    }
    loop {
        let key = scan.key()?;
        let span = scan.value();
        members.push((key, span));
        if !scan.next() {
            return Ok(members);
        }
    }
}

/// A pinned exact provider binding; JSON nodes have no config.
#[derive(Debug, Clone)]
pub struct ResolvedJson {
    snapshot: Arc<RegistrySnapshot>,
    provider: ProviderKey,
    operation: JsonOperation,
}

impl ResolvedJson {
    pub fn snapshot(&self) -> &Arc<RegistrySnapshot> {
        &self.snapshot
    }
    pub fn provider(&self) -> &ProviderKey {
        &self.provider
    }
    pub fn operation(&self) -> JsonOperation {
        self.operation
    }

    /// `get`/`at`/`length` on the wrong kind of JSON value are `core/Domain`; a number
    /// read as an integer must have integer form (`core/Domain` otherwise) and fit the
    /// type (`core/OutOfRange`). Duplicate object keys resolve to the last member.
    pub fn evaluate(&self, inputs: &[Value]) -> Result<Value, KernelError> {
        use JsonOperation::*;
        if inputs.len() != self.operation.inputs().len() {
            return Err(KernelError::InputContract);
        }
        let Value::Json(text) = &inputs[0] else {
            return Err(KernelError::InputContract);
        };
        let mut scan = Scan::new(text.as_str());
        let first = scan.peek().ok_or(KernelError::InputContract)?;
        Ok(match self.operation {
            Kind => Value::Str(Arc::new(
                match first {
                    b'n' => "null",
                    b't' | b'f' => "boolean",
                    b'"' => "string",
                    b'[' => "array",
                    b'{' => "object",
                    _ => "number",
                }
                .to_owned(),
            )),
            IsNull => Value::Bool(first == b'n'),
            AsBool => match first {
                b't' => Value::Bool(true),
                b'f' => Value::Bool(false),
                _ => Value::None,
            },
            AsString => match first {
                b'"' => {
                    let span = scan.value();
                    Value::Str(Arc::new(
                        serde_json::from_str(scan.slice(span))
                            .map_err(|_| KernelError::InputContract)?,
                    ))
                }
                _ => Value::None,
            },
            AsI64 | AsU64 | AsF64 if !matches!(first, b'-' | b'0'..=b'9') => Value::None,
            AsI64 | AsU64 => {
                let span = scan.value();
                let number = scan.slice(span);
                // Integer form only: an optional minus and digits, no fraction/exponent.
                let digits = number.strip_prefix('-').unwrap_or(number);
                if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(KernelError::Domain);
                }
                let value: i128 = number.parse().map_err(|_| KernelError::OutOfRange)?;
                if self.operation == AsI64 {
                    Value::I64(i64::try_from(value).map_err(|_| KernelError::OutOfRange)?)
                } else {
                    Value::U64(u64::try_from(value).map_err(|_| KernelError::OutOfRange)?)
                }
            }
            AsF64 => {
                let span = scan.value();
                let value: f64 = scan
                    .slice(span)
                    .parse()
                    .map_err(|_| KernelError::InputContract)?;
                if value.is_infinite() {
                    return Err(KernelError::OutOfRange);
                }
                Value::F64(value)
            }
            Get => {
                let Value::Str(key) = &inputs[1] else {
                    return Err(KernelError::InputContract);
                };
                if first != b'{' {
                    return Err(KernelError::Domain);
                }
                match members(&mut scan)?
                    .iter()
                    .rev()
                    .find(|(name, _)| name == key.as_str())
                {
                    Some((_, span)) => json_value(scan.slice(*span))?,
                    None => Value::None,
                }
            }
            At => {
                let Value::U64(index) = inputs[1] else {
                    return Err(KernelError::InputContract);
                };
                if first != b'[' {
                    return Err(KernelError::Domain);
                }
                if scan.open() {
                    return Ok(Value::None);
                }
                let mut position = 0u64;
                loop {
                    let span = scan.value();
                    if position == index {
                        break json_value(scan.slice(span))?;
                    }
                    position += 1;
                    if !scan.next() {
                        break Value::None;
                    }
                }
            }
            Length => match first {
                b'[' => {
                    let mut count = 0u64;
                    if !scan.open() {
                        loop {
                            scan.value();
                            count += 1;
                            if !scan.next() {
                                break;
                            }
                        }
                    }
                    Value::U64(count)
                }
                b'{' => {
                    let mut keys: Vec<String> = members(&mut scan)?
                        .into_iter()
                        .map(|(key, _)| key)
                        .collect();
                    keys.sort_unstable();
                    keys.dedup();
                    Value::U64(keys.len() as u64)
                }
                _ => return Err(KernelError::Domain),
            },
        })
    }
}

impl MathPackage {
    /// Resolves only this build's exact `org.orange.json` provider.
    pub fn resolve_json(
        &self,
        snapshot: Arc<RegistrySnapshot>,
        provider: &ProviderKey,
        definition: &DefinitionRef,
        config_version: u32,
        config: &ConfigValue,
    ) -> Result<ResolvedJson, BindingError> {
        let operation = JsonOperation::from_definition_id(definition.id.as_str())
            .ok_or(BindingError::Unavailable)?;
        if provider != self.json_provider_key()
            || definition.version != 1
            || config_version != 1
            || !snapshot.provides(definition, config_version, provider)
        {
            return Err(BindingError::Unavailable);
        }
        if !config.is_empty() {
            return Err(BindingError::InvalidConfig);
        }
        Ok(ResolvedJson {
            snapshot,
            provider: provider.clone(),
            operation,
        })
    }
}
