//! Stable validation diagnostics and their safe, translatable message envelope.

use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::id::{EdgeId, FunctionId, NodeId, PortKey};

/// Stable identifiers for implemented graph validation diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum DiagnosticCode {
    #[serde(rename = "E-PORT-UNKNOWN")]
    PortUnknown,
    #[serde(rename = "E-PORT-DIRECTION")]
    PortDirection,
    #[serde(rename = "E-KIND-MISMATCH")]
    KindMismatch,
    #[serde(rename = "E-FANIN-DATA")]
    FaninData,
    #[serde(rename = "E-FANOUT-EXEC")]
    FanoutExec,
    #[serde(rename = "E-FANOUT-EVENT")]
    FanoutEvent,
    #[serde(rename = "E-STREAM-FANOUT")]
    StreamFanout,
    #[serde(rename = "E-SCOPE-EDGE")]
    ScopeEdge,
    #[serde(rename = "E-TYPE-MISMATCH")]
    TypeMismatch,
    #[serde(rename = "E-SIGNATURE-MISSING")]
    SignatureMissing,
    #[serde(rename = "E-PROVIDER-MISSING")]
    ProviderMissing,
    #[serde(rename = "E-PACKAGE-LOCK-MISSING")]
    PackageLockMissing,
    #[serde(rename = "E-PROVIDER-UNAVAILABLE")]
    ProviderUnavailable,
    #[serde(rename = "E-SIGNATURE-DRIFT")]
    SignatureDrift,
    #[serde(rename = "E-TYPE-AMBIGUOUS")]
    TypeAmbiguous,
    #[serde(rename = "E-TYPE-COMPLEXITY")]
    TypeComplexity,
    #[serde(rename = "E-CONSTRAINT-UNSATISFIED")]
    ConstraintUnsatisfied,
    #[serde(rename = "E-NO-SPECIALIZATION")]
    NoSpecialization,
    #[serde(rename = "E-INPUT-REQUIRED")]
    InputRequired,
    #[serde(rename = "E-LITERAL-TYPE")]
    LiteralType,
    #[serde(rename = "E-HANDLE-LITERAL")]
    HandleLiteral,
    #[serde(rename = "W-LITERAL-SHADOWED")]
    LiteralShadowed,
    #[serde(rename = "E-SUSPENDS-MISSING")]
    SuspendsMissing,
    #[serde(rename = "E-SUSPENDS-INVALID")]
    SuspendsInvalid,
    #[serde(rename = "E-RECURSION")]
    Recursion,
    #[serde(rename = "E-FUNCTION-KIND")]
    FunctionKind,
    #[serde(rename = "E-CALL-TARGET-MISSING")]
    CallTargetMissing,
    #[serde(rename = "E-CALL-TARGET-UNKNOWN")]
    CallTargetUnknown,
    #[serde(rename = "E-FUNCTION-METADATA-UNAVAILABLE")]
    FunctionMetadataUnavailable,
    #[serde(rename = "E-VARIABLE-UNKNOWN")]
    VariableUnknown,
    #[serde(rename = "E-UPDATE-OPERATION")]
    UpdateOperation,
    #[serde(rename = "E-ENTRY-IN-FUNCTION")]
    EntryInFunction,
}

impl DiagnosticCode {
    /// Stable code used in wire records and assertions.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PortUnknown => "E-PORT-UNKNOWN",
            Self::PortDirection => "E-PORT-DIRECTION",
            Self::KindMismatch => "E-KIND-MISMATCH",
            Self::FaninData => "E-FANIN-DATA",
            Self::FanoutExec => "E-FANOUT-EXEC",
            Self::FanoutEvent => "E-FANOUT-EVENT",
            Self::StreamFanout => "E-STREAM-FANOUT",
            Self::ScopeEdge => "E-SCOPE-EDGE",
            Self::TypeMismatch => "E-TYPE-MISMATCH",
            Self::SignatureMissing => "E-SIGNATURE-MISSING",
            Self::ProviderMissing => "E-PROVIDER-MISSING",
            Self::PackageLockMissing => "E-PACKAGE-LOCK-MISSING",
            Self::ProviderUnavailable => "E-PROVIDER-UNAVAILABLE",
            Self::SignatureDrift => "E-SIGNATURE-DRIFT",
            Self::TypeAmbiguous => "E-TYPE-AMBIGUOUS",
            Self::TypeComplexity => "E-TYPE-COMPLEXITY",
            Self::ConstraintUnsatisfied => "E-CONSTRAINT-UNSATISFIED",
            Self::NoSpecialization => "E-NO-SPECIALIZATION",
            Self::InputRequired => "E-INPUT-REQUIRED",
            Self::LiteralType => "E-LITERAL-TYPE",
            Self::HandleLiteral => "E-HANDLE-LITERAL",
            Self::LiteralShadowed => "W-LITERAL-SHADOWED",
            Self::SuspendsMissing => "E-SUSPENDS-MISSING",
            Self::SuspendsInvalid => "E-SUSPENDS-INVALID",
            Self::Recursion => "E-RECURSION",
            Self::FunctionKind => "E-FUNCTION-KIND",
            Self::CallTargetMissing => "E-CALL-TARGET-MISSING",
            Self::CallTargetUnknown => "E-CALL-TARGET-UNKNOWN",
            Self::FunctionMetadataUnavailable => "E-FUNCTION-METADATA-UNAVAILABLE",
            Self::VariableUnknown => "E-VARIABLE-UNKNOWN",
            Self::UpdateOperation => "E-UPDATE-OPERATION",
            Self::EntryInFunction => "E-ENTRY-IN-FUNCTION",
        }
    }

    const fn message_key(self) -> &'static str {
        match self {
            Self::PortUnknown => "core/graph-port-unknown",
            Self::PortDirection => "core/graph-port-direction",
            Self::KindMismatch => "core/graph-kind-mismatch",
            Self::FaninData => "core/graph-fanin-data",
            Self::FanoutExec => "core/graph-fanout-exec",
            Self::FanoutEvent => "core/graph-fanout-event",
            Self::StreamFanout => "core/graph-stream-fanout",
            Self::ScopeEdge => "core/graph-scope-edge",
            Self::TypeMismatch => "core/graph-type-mismatch",
            Self::SignatureMissing => "core/graph-signature-missing",
            Self::ProviderMissing => "core/graph-provider-missing",
            Self::PackageLockMissing => "core/graph-package-lock-missing",
            Self::ProviderUnavailable => "core/graph-provider-unavailable",
            Self::SignatureDrift => "core/graph-signature-drift",
            Self::TypeAmbiguous => "core/graph-type-ambiguous",
            Self::TypeComplexity => "core/graph-type-complexity",
            Self::ConstraintUnsatisfied => "core/graph-constraint-unsatisfied",
            Self::NoSpecialization => "core/graph-no-specialization",
            Self::InputRequired => "core/graph-input-required",
            Self::LiteralType => "core/graph-literal-type",
            Self::HandleLiteral => "core/graph-handle-literal",
            Self::LiteralShadowed => "core/graph-literal-shadowed",
            Self::SuspendsMissing => "core/graph-suspends-missing",
            Self::SuspendsInvalid => "core/graph-suspends-invalid",
            Self::Recursion => "core/graph-recursion",
            Self::FunctionKind => "core/graph-function-kind",
            Self::CallTargetMissing => "core/graph-call-target-missing",
            Self::CallTargetUnknown => "core/graph-call-target-unknown",
            Self::FunctionMetadataUnavailable => "core/graph-function-metadata-unavailable",
            Self::VariableUnknown => "core/graph-variable-unknown",
            Self::UpdateOperation => "core/graph-update-operation",
            Self::EntryInFunction => "core/graph-entry-in-function",
        }
    }

    fn fallback(self, subject: &Subject) -> String {
        match self {
            Self::PortUnknown => format!(
                "Connection references an unknown port{}.",
                subject.port_suffix()
            ),
            Self::PortDirection => {
                "Connection must run from an output port to an input port.".into()
            }
            Self::KindMismatch => "Connection ports have incompatible kinds.".into(),
            Self::FaninData => "Data input has more than one connection.".into(),
            Self::FanoutExec => "Execution output has more than one connection.".into(),
            Self::FanoutEvent => "Event output has more than one connection.".into(),
            Self::StreamFanout => "Stream output has more than one consumer.".into(),
            Self::ScopeEdge => "Connection crosses graph body scopes.".into(),
            Self::TypeMismatch => "Connection port types are incompatible.".into(),
            Self::SignatureMissing => "Node has no stored signature snapshot.".into(),
            Self::ProviderMissing => "Node has no provider binding.".into(),
            Self::PackageLockMissing => "Node provider package version is not locked.".into(),
            Self::ProviderUnavailable => "Node definition or exact provider is unavailable.".into(),
            Self::SignatureDrift => {
                "Stored node signature differs from the resolved definition.".into()
            }
            Self::TypeAmbiguous => "A generic node has unresolved type parameters.".into(),
            Self::TypeComplexity => "Type analysis exceeded its resource limit.".into(),
            Self::ConstraintUnsatisfied => {
                "A type argument does not satisfy its constraint.".into()
            }
            Self::NoSpecialization => {
                "Provider does not support the resolved type arguments.".into()
            }
            Self::InputRequired => "Required input has no value.".into(),
            Self::LiteralType => "Input literal does not match the declared port type.".into(),
            Self::HandleLiteral => "Handle ports cannot have literals.".into(),
            Self::LiteralShadowed => "Input literal is shadowed by a connected edge.".into(),
            Self::SuspendsMissing => "Suspension metadata is required for this node kind.".into(),
            Self::SuspendsInvalid => "Suspension metadata conflicts with the node kind.".into(),
            Self::Recursion => "Function calls contain a recursion cycle.".into(),
            Self::FunctionKind => {
                "Declared function kind differs from the inferred body kind.".into()
            }
            Self::CallTargetMissing => "Function call requires a canonical function target.".into(),
            Self::CallTargetUnknown => "Function call target is not declared in this graph.".into(),
            Self::FunctionMetadataUnavailable => {
                "Function body or leaf semantics are unavailable.".into()
            }
            Self::VariableUnknown => "Variable reference is missing, invalid or undeclared.".into(),
            Self::UpdateOperation => "Variable update requires a supported operation.".into(),
            Self::EntryInFunction => {
                "Entry and watch nodes are not permitted in a function.".into()
            }
        }
    }
}

impl fmt::Display for DiagnosticCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Whether a diagnostic prevents compiling the affected graph region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

/// Stable graph identities associated with a diagnostic.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Subject {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<FunctionId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<NodeId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<PortKey>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edge: Option<EdgeId>,
}

impl Subject {
    pub const fn new(node: Option<NodeId>, port: Option<PortKey>, edge: Option<EdgeId>) -> Self {
        Self {
            function: None,
            node,
            port,
            edge,
        }
    }

    /// Adds the graph-local function owning this diagnostic subject.
    pub const fn with_function(mut self, function: FunctionId) -> Self {
        self.function = Some(function);
        self
    }

    fn port_suffix(&self) -> String {
        self.port
            .as_ref()
            .map_or_else(String::new, |port| format!(" `{port}`"))
    }
}

/// Validated immutable identity of the host catalogue bytes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct HostOrigin {
    kind: HostOriginKind,
    version: String,
    build_digest: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum HostOriginKind {
    Host,
}

impl HostOrigin {
    pub fn new(
        version: impl Into<String>,
        build_digest: impl Into<String>,
    ) -> Result<Self, OriginError> {
        let version = version.into();
        let build_digest = build_digest.into();
        if version.is_empty() || version.trim() != version || version.chars().any(char::is_control)
        {
            return Err(OriginError::InvalidVersion);
        }
        if !is_sha256(&build_digest) {
            return Err(OriginError::InvalidDigest);
        }
        Ok(Self {
            kind: HostOriginKind::Host,
            version,
            build_digest,
        })
    }

    pub fn version(&self) -> &str {
        &self.version
    }
    pub fn build_digest(&self) -> &str {
        &self.build_digest
    }
}

impl<'de> Deserialize<'de> for HostOrigin {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct WireOrigin {
            kind: HostOriginKind,
            version: String,
            build_digest: String,
        }

        let wire = WireOrigin::deserialize(deserializer)?;
        let mut origin =
            Self::new(wire.version, wire.build_digest).map_err(serde::de::Error::custom)?;
        origin.kind = wire.kind;
        Ok(origin)
    }
}

fn is_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum OriginError {
    #[error("host version must be non-empty, unpadded text without control characters")]
    InvalidVersion,
    #[error("host build digest must be sha256 followed by 64 lowercase hexadecimal digits")]
    InvalidDigest,
}

/// Safe named arguments supported by the initial diagnostics catalog.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum MessageArg {
    #[serde(rename = "core/string@1")]
    Identifier(IdentifierArg),
    #[serde(rename = "core/u64@1", with = "decimal_u64")]
    U64(u64),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct IdentifierArg(String);

impl IdentifierArg {
    pub fn new(value: impl Into<String>) -> Result<Self, MessageArgError> {
        let value = value.into();
        if !is_safe_identifier(&value) {
            return Err(MessageArgError::InvalidIdentifier);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Serialize for IdentifierArg {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for IdentifierArg {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

mod decimal_u64 {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(value)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
        let value = String::deserialize(deserializer)?;
        let parsed = value.parse::<u64>().map_err(serde::de::Error::custom)?;
        if parsed.to_string() != value {
            return Err(serde::de::Error::custom(
                "unsigned integer is not canonical decimal",
            ));
        }
        Ok(parsed)
    }
}

impl MessageArg {
    pub fn identifier(value: impl Into<String>) -> Result<Self, MessageArgError> {
        IdentifierArg::new(value).map(Self::Identifier)
    }
}

fn is_safe_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphanumeric() || first == '_')
        && chars
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/' | '#' | ':'))
        && value.len() <= 256
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum MessageArgError {
    #[error("message identifier argument contains unsupported characters or exceeds 256 bytes")]
    InvalidIdentifier,
}

/// Diagnostic record with stable semantic fields and a safe English fallback.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: DiagnosticCode,
    pub severity: Severity,
    #[serde(flatten)]
    pub subject: Subject,
    pub message_origin: HostOrigin,
    pub message_key: String,
    pub message_args: std::collections::BTreeMap<String, MessageArg>,
    pub message: String,
}

impl Diagnostic {
    pub fn new(code: DiagnosticCode, subject: Subject, origin: &HostOrigin) -> Self {
        let message = code.fallback(&subject);
        let mut message_args = std::collections::BTreeMap::new();
        if code == DiagnosticCode::PortUnknown
            && let Some(port) = &subject.port
            && let Ok(argument) = MessageArg::identifier(port.to_string())
        {
            message_args.insert("port".to_owned(), argument);
        }
        let severity = if code == DiagnosticCode::LiteralShadowed {
            Severity::Warning
        } else {
            Severity::Error
        };
        Self {
            code,
            severity,
            subject,
            message_origin: origin.clone(),
            message_key: code.message_key().to_owned(),
            message_args,
            message,
        }
    }

    /// Adds an unsigned count under a stable catalogue argument name.
    pub fn with_count(mut self, name: &'static str, count: u64) -> Self {
        if is_safe_identifier(name) {
            self.message_args
                .insert(name.to_owned(), MessageArg::U64(count));
        }
        self
    }
}

/// Sorts diagnostics by stable semantic fields for deterministic output.
pub fn sort_diagnostics(diagnostics: &mut [Diagnostic]) {
    diagnostics.sort_by(|left, right| {
        (
            &left.subject,
            left.code,
            left.severity,
            &left.message_key,
            &left.message_args,
        )
            .cmp(&(
                &right.subject,
                right.code,
                right.severity,
                &right.message_key,
                &right.message_args,
            ))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, to_value};

    fn origin() -> HostOrigin {
        HostOrigin::new("1.2.3", format!("sha256:{}", "a".repeat(64))).unwrap()
    }

    #[test]
    fn validates_host_origin() {
        assert!(HostOrigin::new("1.2.3", format!("sha256:{}", "0".repeat(64))).is_ok());
        assert_eq!(
            HostOrigin::new(" 1.2.3", format!("sha256:{}", "0".repeat(64))),
            Err(OriginError::InvalidVersion)
        );
        assert_eq!(
            HostOrigin::new("1.2.3", "sha256:BAD"),
            Err(OriginError::InvalidDigest)
        );
    }

    #[test]
    fn message_args_use_safe_tagged_wire_values() {
        assert_eq!(
            to_value(MessageArg::U64(u64::MAX)).unwrap(),
            json!({"type":"core/u64@1","value":"18446744073709551615"})
        );
        assert_eq!(
            MessageArg::identifier("node:17/port#2").unwrap(),
            MessageArg::Identifier(IdentifierArg::new("node:17/port#2").unwrap())
        );
        assert_eq!(
            MessageArg::identifier("user supplied secret").unwrap_err(),
            MessageArgError::InvalidIdentifier
        );
    }

    #[test]
    fn diagnostic_codes_severity_origin_and_fallback_are_stable() {
        let diagnostic =
            Diagnostic::new(DiagnosticCode::PortUnknown, Subject::default(), &origin());
        assert_eq!(diagnostic.code.as_str(), "E-PORT-UNKNOWN");
        assert_eq!(diagnostic.severity, Severity::Error);
        assert_eq!(diagnostic.message_key, "core/graph-port-unknown");
        assert_eq!(diagnostic.message, "Connection references an unknown port.");
        assert_eq!(
            to_value(diagnostic).unwrap()["message_origin"]["kind"],
            "host"
        );
    }
}
