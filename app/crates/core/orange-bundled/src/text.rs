//! `org.orange.text` string vocabulary. Bind once; evaluation reads no locale or host.

use std::sync::Arc;

use orange_model::Value;
use orange_model::catalog::{NodeDefinition, Ports, ProviderKey, RegistrySnapshot};
use orange_model::document::{ConfigValue, DefinitionRef, NodeKind, OutputSource, PortDescriptor};
use orange_model::id::TypeExpr;
use orange_model::value::shape::VALUE_CHARGE;
use orange_model::value::{ListRepr, base64};
use serde_json::json;

use crate::{BindingError, CatalogError, KernelError, MAX_COLLECTION_BYTES, MathPackage};

/// Largest string, in UTF-8 bytes, that a text node produces. Larger results fail
/// with `core/LimitExceeded` before the result is built.
pub const MAX_TEXT_BYTES: usize = 16 * 1024 * 1024;
const MAX_REGEX_PATTERN_BYTES: usize = 4 * 1024;
const MAX_REGEX_TEXT_BYTES: usize = 1024 * 1024;
const MAX_REGEX_COMPILED_BYTES: usize = 1024 * 1024;
const MAX_CSV_BYTES: usize = 1024 * 1024;
const MAX_CSV_FIELDS: usize = 16_384;

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
    ToUppercase,
    ToLowercase,
    Split,
    Join,
    Format,
    InspectError,
    EncodeBase64,
    DecodeBase64,
    EncodeHex,
    DecodeHex,
    RegexMatch,
    RegexExtract,
    ParseCsv,
    WriteCsv,
}

/// Which ends of the text `trim` takes whitespace from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrimEnds {
    Both,
    Start,
    End,
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
const CSV_ROWS: &str = "core/list@1<core/list@1<core/string@1>>";

impl TextOperation {
    pub const ALL: [Self; 24] = [
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
        Self::ToUppercase,
        Self::ToLowercase,
        Self::Split,
        Self::Join,
        Self::Format,
        Self::InspectError,
        Self::EncodeBase64,
        Self::DecodeBase64,
        Self::EncodeHex,
        Self::DecodeHex,
        Self::RegexMatch,
        Self::RegexExtract,
        Self::ParseCsv,
        Self::WriteCsv,
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
            Self::ToUppercase => "org.orange.text/to-uppercase",
            Self::ToLowercase => "org.orange.text/to-lowercase",
            Self::Split => "org.orange.text/split",
            Self::Join => "org.orange.text/join",
            Self::Format => "org.orange.text/format",
            Self::InspectError => "org.orange.text/inspect-error",
            Self::EncodeBase64 => "org.orange.text/encode-base64",
            Self::DecodeBase64 => "org.orange.text/decode-base64",
            Self::EncodeHex => "org.orange.text/encode-hex",
            Self::DecodeHex => "org.orange.text/decode-hex",
            Self::RegexMatch => "org.orange.text/regex-match",
            Self::RegexExtract => "org.orange.text/regex-extract",
            Self::ParseCsv => "org.orange.text/parse-csv",
            Self::WriteCsv => "org.orange.text/write-csv",
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
            | Self::ToUppercase
            | Self::ToLowercase => &[("value", STRING)],
            Self::Concat => &[("a", STRING), ("b", STRING)],
            Self::Contains
            | Self::StartsWith
            | Self::EndsWith
            | Self::Find
            | Self::RegexMatch
            | Self::RegexExtract => &[("text", STRING), ("pattern", STRING)],
            Self::Substring => &[("text", STRING), ("start", U64), ("length", U64)],
            Self::Replace => &[
                ("text", STRING),
                ("pattern", STRING),
                ("replacement", STRING),
            ],
            Self::Split => &[("text", STRING), ("separator", STRING)],
            Self::Join => &[("items", LIST), ("separator", STRING)],
            Self::Format => &[
                ("template", STRING),
                ("arguments", "core/list@1<core/json@1>"),
            ],
            Self::InspectError => &[("fault", "core/error@1")],
            Self::EncodeBase64 | Self::EncodeHex => &[("bytes", "core/bytes@1")],
            Self::DecodeBase64 | Self::DecodeHex | Self::ParseCsv => &[("text", STRING)],
            Self::WriteCsv => &[("rows", CSV_ROWS)],
        }
    }

    pub const fn output_port(self) -> &'static str {
        "result"
    }

    pub const fn output_type(self) -> &'static str {
        match self {
            Self::ByteLength | Self::ScalarLength => U64,
            Self::Contains | Self::StartsWith | Self::EndsWith | Self::RegexMatch => BOOL,
            Self::RegexExtract => "core/option@1<core/string@1>",
            Self::ParseCsv => CSV_ROWS,
            Self::Find => OPTION_U64,
            Self::Split => LIST,
            Self::DecodeBase64 | Self::DecodeHex => "core/bytes@1",
            _ => STRING,
        }
    }

    /// Whether the node takes the `unit` config (scalar default, or byte).
    pub const fn has_unit(self) -> bool {
        matches!(self, Self::Find | Self::Substring)
    }

    /// The one setting the node takes, if any: `unit` for find and substring, `ends`
    /// for trim.
    pub const fn setting(self) -> Option<&'static str> {
        match self {
            Self::Find | Self::Substring => Some("unit"),
            Self::Trim => Some("ends"),
            _ => None,
        }
    }
}

/// Ordinary non-generic package definitions in the `org.orange.text` namespace.
pub fn text_definitions() -> Result<Vec<NodeDefinition>, CatalogError> {
    TextOperation::ALL
        .into_iter()
        .map(|operation| {
            let mut ports = crate::catalog::input_ports(operation.inputs())?;
            ports.push(PortDescriptor::data_out(
                operation.output_port(),
                operation.output_type().parse::<TypeExpr>()?,
                OutputSource::Computed,
            ));
            let properties = match operation.setting() {
                Some("unit") => json!({"unit":{"enum":["scalar","byte"],"default":"scalar"}}),
                Some(_) => {
                    json!({"ends":{"title":"Ends","enum":["both","start","end"],"default":"both"}})
                }
                None => json!({}),
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
    ends: TrimEnds,
}

/// A field and the unconsumed delimiter (or end of input).
fn csv_field(input: &str) -> Result<(String, &str), KernelError> {
    let Some(mut quoted) = input.strip_prefix('"') else {
        let end = input.find([',', '\r', '\n', '"']).unwrap_or(input.len());
        return Ok((input[..end].to_owned(), &input[end..]));
    };
    let mut field = String::new();
    loop {
        let (part, tail) = quoted.split_once('"').ok_or(KernelError::Domain)?;
        field.push_str(part);
        let Some(escaped) = tail.strip_prefix('"') else {
            return Ok((field, tail));
        };
        field.push('"');
        quoted = escaped;
    }
}

fn parse_csv(mut input: &str) -> Result<Value, KernelError> {
    if input.len() > MAX_CSV_BYTES {
        return Err(KernelError::LimitExceeded);
    }
    let mut rows = Vec::new();
    let mut total_fields = 0;
    while !input.is_empty() {
        let mut row = Vec::new();
        loop {
            if total_fields == MAX_CSV_FIELDS {
                return Err(KernelError::LimitExceeded);
            }
            let (field, tail) = csv_field(input)?;
            row.push(Arc::new(field));
            total_fields += 1;
            if let Some(rest) = tail.strip_prefix(',') {
                input = rest;
            } else {
                input = if tail.is_empty() {
                    tail
                } else {
                    tail.strip_prefix("\r\n")
                        .or_else(|| tail.strip_prefix('\n'))
                        .ok_or(KernelError::Domain)?
                };
                break;
            }
        }
        rows.push(Value::List(Arc::new(ListRepr::Str(row.into()))));
    }
    Ok(Value::List(Arc::new(ListRepr::Values(rows.into()))))
}

fn write_csv(value: &Value) -> Result<String, KernelError> {
    let Value::List(rows) = value else {
        return Err(KernelError::InputContract);
    };
    let mut csv = String::new();
    let mut total_fields = 0;
    for row in rows.iter() {
        let Value::List(fields) = row else {
            return Err(KernelError::InputContract);
        };
        if fields.is_empty() {
            return Err(KernelError::Domain);
        }
        total_fields += fields.len();
        if total_fields > MAX_CSV_FIELDS {
            return Err(KernelError::LimitExceeded);
        }
        for (index, field) in fields.iter().enumerate() {
            let field = string(&field)?;
            if field.len() > MAX_CSV_BYTES.saturating_sub(csv.len()) {
                return Err(KernelError::LimitExceeded);
            }
            let quoted = field.contains([',', '\r', '\n', '"']);
            let extra = if quoted {
                2 + field.bytes().filter(|&byte| byte == b'"').count()
            } else {
                0
            };
            let delimiters = usize::from(index > 0) + usize::from(index + 1 == fields.len());
            if csv.len() + field.len() + extra + delimiters > MAX_CSV_BYTES {
                return Err(KernelError::LimitExceeded);
            }
            if index > 0 {
                csv.push(',');
            }
            if quoted {
                csv.push('"');
                for (index, part) in field.split('"').enumerate() {
                    if index != 0 {
                        csv.push_str("\"\"");
                    }
                    csv.push_str(part);
                }
                csv.push('"');
            } else {
                csv.push_str(field);
            }
        }
        csv.push('\n');
    }
    Ok(csv)
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

fn append(out: &mut String, text: &str) -> Result<(), KernelError> {
    limit(out.len().saturating_add(text.len()))?;
    out.push_str(text);
    Ok(())
}

fn encode_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 15)]));
    }
    output
}

fn decode_hex(text: &str) -> Option<Vec<u8>> {
    let (pairs, remainder) = text.as_bytes().as_chunks::<2>();
    if !remainder.is_empty() {
        return None;
    }
    pairs
        .iter()
        .map(|&[high, low]| {
            let high = char::from(high).to_digit(16)?;
            let low = char::from(low).to_digit(16)?;
            Some((high * 16 + low) as u8)
        })
        .collect()
}

fn format_text(mut template: &str, arguments: &ListRepr) -> Result<String, KernelError> {
    limit(template.len())?;
    let mut output = String::new();
    while let Some(brace) = template.find(['{', '}']) {
        let (plain, control) = template.split_at(brace);
        append(&mut output, plain)?;
        if control.starts_with("{{") || control.starts_with("}}") {
            append(&mut output, &control[..1])?;
            template = &control[2..];
            continue;
        }
        let (placeholder, next) = control
            .strip_prefix('{')
            .and_then(|text| text.split_once('}'))
            .ok_or(KernelError::Domain)?;
        let (index, precision) = match placeholder.split_once(":.") {
            Some((index, precision)) => {
                let precision = decimal(precision)
                    .filter(|&digits| digits <= 16)
                    .ok_or(KernelError::Domain)?;
                (index, Some(precision))
            }
            None => (placeholder, None),
        };
        let index = decimal(index).ok_or(KernelError::Domain)?;
        let Value::Json(value) = arguments.get(index).ok_or(KernelError::OutOfRange)? else {
            return Err(KernelError::InputContract);
        };
        let value = value.as_str().trim();
        if let Some(precision) = precision {
            append_decimal(&mut output, value, precision)?;
        } else if value.starts_with('"') {
            let value: String =
                serde_json::from_str(value).map_err(|_| KernelError::InputContract)?;
            append(&mut output, &value)?;
        } else {
            append(&mut output, value)?;
        }
        template = next;
    }
    append(&mut output, template)?;
    Ok(output)
}

fn decimal(text: &str) -> Option<usize> {
    if !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit()) {
        text.parse().ok()
    } else {
        None
    }
}

fn append_decimal(output: &mut String, value: &str, precision: usize) -> Result<(), KernelError> {
    let magnitude = value.strip_prefix('-').unwrap_or(value);
    if !magnitude.is_empty() && magnitude.bytes().all(|byte| byte.is_ascii_digit()) {
        append(output, value)?;
        if precision > 0 {
            append(output, ".")?;
            append(output, &"0000000000000000"[..precision])?;
        }
    } else {
        let number: f64 = value.parse().map_err(|_| KernelError::Domain)?;
        if !number.is_finite() {
            return Err(KernelError::NonFinite);
        }
        append(output, &format!("{number:.precision$}"))?;
    }
    Ok(())
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
            RegexMatch | RegexExtract => {
                let (source, pattern) = (first()?, string(&inputs[1])?);
                if source.len() > MAX_REGEX_TEXT_BYTES || pattern.len() > MAX_REGEX_PATTERN_BYTES {
                    return Err(KernelError::LimitExceeded);
                }
                let regex = regex::RegexBuilder::new(pattern)
                    .size_limit(MAX_REGEX_COMPILED_BYTES)
                    .dfa_size_limit(MAX_REGEX_COMPILED_BYTES)
                    .nest_limit(32)
                    .build()
                    .map_err(|error| match error {
                        regex::Error::CompiledTooBig(_) => KernelError::LimitExceeded,
                        _ => KernelError::Domain,
                    })?;
                if self.operation == RegexMatch {
                    Value::Bool(regex.is_match(source))
                } else {
                    regex
                        .find(source)
                        .map_or(Value::None, |found| text(found.as_str().to_owned()))
                }
            }
            ParseCsv => parse_csv(first()?)?,
            WriteCsv => text(write_csv(&inputs[0])?),
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
            Trim => {
                let source = first()?;
                text(
                    match self.ends {
                        TrimEnds::Both => source.trim(),
                        TrimEnds::Start => source.trim_start(),
                        TrimEnds::End => source.trim_end(),
                    }
                    .to_owned(),
                )
            }
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
            Format => {
                let Value::List(arguments) = &inputs[1] else {
                    return Err(KernelError::InputContract);
                };
                text(format_text(first()?, arguments)?)
            }
            InspectError => {
                let Value::Err(error) = &inputs[0] else {
                    return Err(KernelError::InputContract);
                };
                text(error.code.clone())
            }
            EncodeBase64 | EncodeHex => {
                let Value::Bytes(bytes) = &inputs[0] else {
                    return Err(KernelError::InputContract);
                };
                if self.operation == EncodeBase64 {
                    limit(bytes.len().div_ceil(3).saturating_mul(4))?;
                    text(base64::encode(bytes))
                } else {
                    limit(bytes.len().saturating_mul(2))?;
                    text(encode_hex(bytes))
                }
            }
            DecodeBase64 | DecodeHex => {
                let encoded = first()?;
                limit(encoded.len())?;
                let decoded = if self.operation == DecodeBase64 {
                    base64::decode(encoded)
                } else {
                    decode_hex(encoded)
                };
                Value::Bytes(Arc::new(decoded.ok_or(KernelError::Domain)?))
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
        if config
            .keys()
            .any(|key| Some(key.as_str()) != operation.setting())
        {
            return Err(BindingError::InvalidConfig);
        }
        let unit = match config.get("unit").map(|v| v.as_str()) {
            None | Some(Some("scalar")) => TextUnit::Scalar,
            Some(Some("byte")) => TextUnit::Byte,
            _ => return Err(BindingError::InvalidConfig),
        };
        let ends = match config.get("ends").map(|v| v.as_str()) {
            None | Some(Some("both")) => TrimEnds::Both,
            Some(Some("start")) => TrimEnds::Start,
            Some(Some("end")) => TrimEnds::End,
            _ => return Err(BindingError::InvalidConfig),
        };
        Ok(ResolvedText {
            snapshot,
            provider: provider.clone(),
            operation,
            unit,
            ends,
        })
    }
}
