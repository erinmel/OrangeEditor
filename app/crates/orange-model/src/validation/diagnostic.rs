//! Stable validation diagnostics and their safe, translatable message envelope.

use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::id::{EdgeId, EventDeclId, FunctionId, NodeId, PortKey, VariableId};

/// One declaration owns wire identity, localization, severity and fallback text.
macro_rules! diagnostics {
    ($subject:ident; $( $(#[$meta:meta])* $name:ident => ($wire:literal, $key:literal, $severity:ident, $fallback:expr); )*) => {
        /// Stable identifiers for implemented graph validation diagnostics.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        pub enum DiagnosticCode { $( $(#[$meta])* #[serde(rename = $wire)] $name, )* }
        impl DiagnosticCode {
            /// Stable code used in wire records and assertions.
            pub const fn as_str(self) -> &'static str { match self { $(Self::$name => $wire,)* } }
            const fn message_key(self) -> &'static str { match self { $(Self::$name => $key,)* } }
            const fn severity(self) -> Severity { match self { $(Self::$name => Severity::$severity,)* } }
            fn fallback(self, $subject: &Subject) -> String { match self { $(Self::$name => $fallback,)* } }
        }
    };
}

diagnostics! { subject;
    /// Compiler-owned warning: the constant fault remains in the plan.
    ConstError => ("W-CONST-ERROR", "core/graph-const-error", Warning, "The constant expression produces an error.".into());
    DeclarationType => ("E-DECLARATION-TYPE", "core/graph-declaration-type", Error, "Declaration type is invalid or unavailable.".into());
    AnalysisLimit => ("E-ANALYSIS-LIMIT", "core/graph-analysis-limit", Error, "Graph analysis exceeds the host's admission limits.".into());
    AnalysisCancelled => ("E-ANALYSIS-CANCELLED", "core/graph-analysis-cancelled", Error, "Graph analysis was cancelled.".into());
    DomainUnsupported => ("E-DOMAIN-UNSUPPORTED", "core/graph-domain-unsupported", Error, "This execution domain is unavailable.".into());
    DomainAnalysisBlocked => ("E-DOMAIN-ANALYSIS-BLOCKED", "core/graph-domain-analysis-blocked", Error, {
                "Graph analysis requires supported execution domains.".into()
            });
    MetadataStale => ("E-METADATA-STALE", "core/graph-metadata-stale", Error, "Node metadata no longer matches this document.".into());
    PortUnknown => ("E-PORT-UNKNOWN", "core/graph-port-unknown", Error, format!(
                "Connection references an unknown port{}.",
                subject.port_suffix()
            ));
    PortDirection => ("E-PORT-DIRECTION", "core/graph-port-direction", Error, {
                "Connection must run from an output port to an input port.".into()
            });
    KindMismatch => ("E-KIND-MISMATCH", "core/graph-kind-mismatch", Error, "Connection ports have incompatible kinds.".into());
    FaninData => ("E-FANIN-DATA", "core/graph-fanin-data", Error, "Data input has more than one connection.".into());
    FanoutExec => ("E-FANOUT-EXEC", "core/graph-fanout-exec", Error, "Execution output has more than one connection.".into());
    FanoutEvent => ("E-FANOUT-EVENT", "core/graph-fanout-event", Error, "Event output has more than one connection.".into());
    StreamFanout => ("E-STREAM-FANOUT", "core/graph-stream-fanout", Error, "Stream output has more than one consumer.".into());
    ScopeEdge => ("E-SCOPE-EDGE", "core/graph-scope-edge", Error, "Connection crosses graph body scopes.".into());
    TypeMismatch => ("E-TYPE-MISMATCH", "core/graph-type-mismatch", Error, "Connection port types are incompatible.".into());
    SignatureMissing => ("E-SIGNATURE-MISSING", "core/graph-signature-missing", Error, "Node has no stored signature snapshot.".into());
    ProviderMissing => ("E-PROVIDER-MISSING", "core/graph-provider-missing", Error, "Node has no provider binding.".into());
    PackageLockMissing => ("E-PACKAGE-LOCK-MISSING", "core/graph-package-lock-missing", Error, "Node provider package version is not locked.".into());
    ProviderUnavailable => ("E-PROVIDER-UNAVAILABLE", "core/graph-provider-unavailable", Error, "Node definition or exact provider is unavailable.".into());
    DependencyUnavailable => ("E-DEPENDENCY-UNAVAILABLE", "core/graph-dependency-unavailable", Error, {
                "Node cannot be checked because a connected node is unresolved.".into()
            });
    IntrinsicUnsupported => ("E-INTRINSIC-UNSUPPORTED", "core/graph-intrinsic-unsupported", Error, {
                "This core version does not support this language intrinsic yet.".into()
            });
    SignatureDrift => ("E-SIGNATURE-DRIFT", "core/graph-signature-drift", Error, {
                "Stored node signature differs from the resolved definition.".into()
            });
    TypeAmbiguous => ("E-TYPE-AMBIGUOUS", "core/graph-type-ambiguous", Error, "A generic node has unresolved type parameters.".into());
    TypeComplexity => ("E-TYPE-COMPLEXITY", "core/graph-type-complexity", Error, "Type analysis exceeded its resource limit.".into());
    ConstraintUnsatisfied => ("E-CONSTRAINT-UNSATISFIED", "core/graph-constraint-unsatisfied", Error, {
                "A type argument does not satisfy its constraint.".into()
            });
    NoSpecialization => ("E-NO-SPECIALIZATION", "core/graph-no-specialization", Error, {
                "Provider does not support the resolved type arguments.".into()
            });
    InputRequired => ("E-INPUT-REQUIRED", "core/graph-input-required", Error, "Required input has no value.".into());
    VariadicCount => ("E-VARIADIC-COUNT", "core/graph-variadic-count", Error, {
                "The number of variadic port instances is outside the allowed range.".into()
            });
    LiteralType => ("E-LITERAL-TYPE", "core/graph-literal-type", Error, "Input literal does not match the declared port type.".into());
    HandleLiteral => ("E-HANDLE-LITERAL", "core/graph-handle-literal", Error, "Handle ports cannot have literals.".into());
    LiteralShadowed => ("W-LITERAL-SHADOWED", "core/graph-literal-shadowed", Warning, "Input literal is shadowed by a connected edge.".into());
    SuspendsMissing => ("E-SUSPENDS-MISSING", "core/graph-suspends-missing", Error, "Suspension metadata is required for this node kind.".into());
    SuspendsInvalid => ("E-SUSPENDS-INVALID", "core/graph-suspends-invalid", Error, "Suspension metadata conflicts with the node kind.".into());
    Recursion => ("E-RECURSION", "core/graph-recursion", Error, "Function calls contain a recursion cycle.".into());
    FunctionKind => ("E-FUNCTION-KIND", "core/graph-function-kind", Error, {
                "Declared function kind differs from the inferred body kind.".into()
            });
    CallTargetMissing => ("E-CALL-TARGET-MISSING", "core/graph-call-target-missing", Error, "Function call requires a canonical function target.".into());
    CallTargetUnknown => ("E-CALL-TARGET-UNKNOWN", "core/graph-call-target-unknown", Error, "Function call target is not declared in this graph.".into());
    FunctionMetadataUnavailable => ("E-FUNCTION-METADATA-UNAVAILABLE", "core/graph-function-metadata-unavailable", Error, {
                "Function body or leaf semantics are unavailable.".into()
            });
    FunctionInterface => ("E-FUNCTION-INTERFACE", "core/graph-function-interface", Error, {
                "Function interface is malformed: ports must be unique typed data ports using declared type parameters.".into()
            });
    VariableUnknown => ("E-VARIABLE-UNKNOWN", "core/graph-variable-unknown", Error, "Variable reference is missing, invalid or undeclared.".into());
    VariableScope => ("E-VARIABLE-SCOPE", "core/graph-variable-scope", Error, {
                "Workspace variables require an asynchronous shared write.".into()
            });
    SharedVariableScope => ("E-SHARED-VARIABLE-SCOPE", "core/graph-shared-variable-scope", Error, "Shared writes require a workspace variable.".into());
    CycleData => ("E-CYCLE-DATA", "core/graph-cycle-data", Error, "Data dependencies contain a cycle.".into());
    UpdateOperation => ("E-UPDATE-OPERATION", "core/graph-update-operation", Error, "Variable update requires a supported operation.".into());
    UpdateUnsupported => ("E-UPDATE-UNSUPPORTED", "core/graph-update-unsupported", Error, {
                "This variable update operation is not supported by this core version.".into()
            });
    EntryInFunction => ("E-ENTRY-IN-FUNCTION", "core/graph-entry-in-function", Error, {
                "Entry and watch nodes are not permitted in a function.".into()
            });
    SubscriptionInFunction => ("E-SUBSCRIPTION-IN-FUNCTION", "core/graph-subscription-in-function", Error, {
                "Subscriptions are not permitted in a function.".into()
            });
    EventUnknown => ("E-EVENT-UNKNOWN", "core/graph-event-unknown", Error, "Event reference is missing, invalid or undeclared.".into());
    ActivationPolicy => ("E-ACTIVATION-POLICY", "core/graph-activation-policy", Error, "Entry activation policy is missing or invalid.".into());
    ConverterConfig => ("E-CONVERTER-CONFIG", "core/graph-converter-config", Error, {
                "Converter node uses an unsupported or malformed config.".into()
            });
    ConverterEntry => ("E-CONVERTER-ENTRY", "core/graph-converter-entry", Error, "Converter registry entry is unavailable.".into());
    ConverterOptions => ("E-CONVERTER-OPTIONS", "core/graph-converter-options", Error, "Converter options are invalid or incomplete.".into());
    FunctionBodyMarker => ("E-FUNCTION-BODY-MARKER", "core/graph-function-body-marker", Error, {
                "Function entry, return and output markers do not match the declared interface.".into()
            });
    MissingReturn => ("E-MISSING-RETURN", "core/graph-missing-return", Error, "A path of this function ends without reaching a return.".into());
    RegionAmbiguous => ("E-REGION-AMBIGUOUS", "core/graph-region-ambiguous", Error, "An execution node is reachable from more than one region.".into());
    RegionEscape => ("E-REGION-ESCAPE", "core/graph-region-escape", Error, "An execution connection leaves its region.".into());
    UnavailableValue => ("E-UNAVAILABLE-VALUE", "core/graph-unavailable-value", Error, {
                "A value is not available on every path that reaches this input.".into()
            });
    CrossActivation => ("E-CROSS-ACTIVATION", "core/graph-cross-activation", Error, {
                "A value is read from a different activation than the one that produced it.".into()
            });
    BreakOutsideLoop => ("E-BREAK-OUTSIDE-LOOP", "core/graph-break-outside-loop", Error, {
                "Break and continue need an enclosing loop body in the same function.".into()
            });
    CycleSyncControl => ("E-CYCLE-SYNC-CONTROL", "core/graph-cycle-sync-control", Error, "A control cycle needs a node that always suspends.".into());
    LoopUnbounded => ("E-LOOP-UNBOUNDED", "core/graph-loop-unbounded", Error, "A while loop requires a finite positive max_iterations.".into());
    ZoneStructure => ("E-ZONE-STRUCTURE", "core/graph-zone-structure", Error, {
                "A zone needs a matching begin and end, a pure body and no values leaving it except through the end."
                    .into()
            });
    SwitchCase => ("E-SWITCH-CASE", "core/graph-switch-case", Error, {
                "Switch cases need distinct values of the key's kind.".into()
            });
    ForEachMode => ("E-FOR-EACH-MODE", "core/graph-for-each-mode", Error, "For each mode must be list or map.".into());
    BusyLoop => ("W-BUSY-LOOP", "core/graph-busy-loop", Warning, "This control cycle only yields and keeps the CPU busy.".into());
    Unreachable => ("W-UNREACHABLE", "core/graph-unreachable", Warning, {
                "Execution node is not reachable from any entry or region root.".into()
            });
    WriteConflict => ("E-WRITE-CONFLICT", "core/graph-write-conflict", Error, {
                "Parallel Fork branches write the same variable without a compatible policy.".into()
            });
    HandlePersist => ("E-HANDLE-PERSIST", "core/graph-handle-persist", Error, {
                "Handle types cannot be persisted, shared, written as literals or exported.".into()
            });
    StreamStore => ("E-STREAM-STORE", "core/graph-stream-store", Error, "Streams cannot be stored in variables or exported.".into());
    StaleCapture => ("W-STALE-CAPTURE", "core/graph-stale-capture", Warning, {
                "A value captured before a suspension is written back; use update-variable.".into()
            });
    VariablePersistent => ("E-VARIABLE-PERSISTENT", "core/graph-variable-persistent", Error, "Local variables cannot be persistent.".into());
    RegisterConfig => ("E-REGISTER-CONFIG", "core/graph-register-config", Error, {
                "Register scope must be run or activation and initial must be a literal of its type."
                    .into()
            });
    DetachInFunction => ("W-DETACH-IN-FUNCTION", "core/graph-detach-in-function", Warning, {
                "A return cancels the detached strands left by this fork.".into()
            });
    ExportTarget => ("E-EXPORT-TARGET", "core/graph-export-target", Error, "Export does not name an existing data output port.".into());
    ExportDuplicate => ("E-EXPORT-DUPLICATE", "core/graph-export-duplicate", Error, "Export identifier is used more than once.".into());
    ExportType => ("E-EXPORT-TYPE", "core/graph-export-type", Error, "Exported value type cannot be serialized.".into());
    ConfigInvalid => ("E-CONFIG-INVALID", "core/graph-config-invalid", Error, "Node configuration does not satisfy its definition schema.".into());
    ConfigSchemaUnsupported => ("E-CONFIG-SCHEMA-UNSUPPORTED", "core/graph-config-schema-unsupported", Error, {
                "Definition config schema uses constructs outside the supported dialect.".into()
            });
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variable: Option<VariableId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<EventDeclId>,
}

impl Subject {
    pub const fn new(node: Option<NodeId>, port: Option<PortKey>, edge: Option<EdgeId>) -> Self {
        Self {
            function: None,
            node,
            port,
            edge,
            variable: None,
            event: None,
        }
    }

    /// Adds the declared variable this diagnostic is about.
    pub const fn with_variable(mut self, variable: VariableId) -> Self {
        self.variable = Some(variable);
        self
    }

    /// Adds the declared event this diagnostic is about.
    pub const fn with_event(mut self, event: EventDeclId) -> Self {
        self.event = Some(event);
        self
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
        let severity = code.severity();
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
            &left.message_origin,
            &left.message,
        )
            .cmp(&(
                &right.subject,
                right.code,
                right.severity,
                &right.message_key,
                &right.message_args,
                &right.message_origin,
                &right.message,
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
    fn sorting_is_total_so_equal_records_are_adjacent() {
        let make = |message: &str| {
            let subject = Subject::new(
                Some("10000000-0000-4000-8000-000000000001".parse().unwrap()),
                None,
                None,
            );
            let mut item = Diagnostic::new(DiagnosticCode::PortUnknown, subject, &origin());
            item.message = message.into();
            item
        };
        let mut items = vec![make("a"), make("b"), make("a")];
        sort_diagnostics(&mut items);
        items.dedup();
        assert_eq!(items, vec![make("a"), make("b")]);
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

    #[test]
    fn unsupported_update_diagnostic_has_stable_empty_argument_envelope() {
        let diagnostic = Diagnostic::new(
            DiagnosticCode::UpdateUnsupported,
            Subject::default(),
            &origin(),
        );
        assert_eq!(diagnostic.code.as_str(), "E-UPDATE-UNSUPPORTED");
        assert_eq!(diagnostic.message_key, "core/graph-update-unsupported");
        assert_eq!(
            diagnostic.message,
            "This variable update operation is not supported by this core version."
        );
        assert!(diagnostic.message_args.is_empty());
        let wire = to_value(&diagnostic).unwrap();
        assert_eq!(wire["code"], "E-UPDATE-UNSUPPORTED");
        assert_eq!(
            serde_json::from_value::<DiagnosticCode>(wire["code"].clone()).unwrap(),
            DiagnosticCode::UpdateUnsupported
        );
        assert_eq!(wire["message_key"], "core/graph-update-unsupported");
        assert_eq!(wire["message_args"], json!({}));
    }

    #[test]
    fn data_cycle_message_keeps_port_paths_out_of_message_arguments() {
        let diagnostic = Diagnostic::new(DiagnosticCode::CycleData, Subject::default(), &origin());
        assert_eq!(diagnostic.code.as_str(), "E-CYCLE-DATA");
        assert_eq!(diagnostic.message_key, "core/graph-cycle-data");
        assert_eq!(diagnostic.message, "Data dependencies contain a cycle.");
        assert!(diagnostic.message_args.is_empty());
        let wire = to_value(&diagnostic).unwrap();
        assert_eq!(wire["code"], "E-CYCLE-DATA");
        assert_eq!(
            serde_json::from_value::<DiagnosticCode>(wire["code"].clone()).unwrap(),
            DiagnosticCode::CycleData
        );
    }

    #[test]
    fn shared_variable_scope_has_stable_redacted_message() {
        let diagnostic = Diagnostic::new(
            DiagnosticCode::SharedVariableScope,
            Subject::default(),
            &origin(),
        );
        assert_eq!(diagnostic.code.as_str(), "E-SHARED-VARIABLE-SCOPE");
        assert_eq!(diagnostic.message_key, "core/graph-shared-variable-scope");
        assert_eq!(
            diagnostic.message,
            "Shared writes require a workspace variable."
        );
        assert!(diagnostic.message_args.is_empty());
        let wire = to_value(&diagnostic).unwrap();
        assert_eq!(wire["code"], "E-SHARED-VARIABLE-SCOPE");
        assert_eq!(
            serde_json::from_value::<DiagnosticCode>(wire["code"].clone()).unwrap(),
            DiagnosticCode::SharedVariableScope
        );
    }

    #[test]
    fn variadic_count_diagnostic_has_stable_empty_argument_envelope() {
        let diagnostic =
            Diagnostic::new(DiagnosticCode::VariadicCount, Subject::default(), &origin());
        assert_eq!(diagnostic.code.as_str(), "E-VARIADIC-COUNT");
        assert_eq!(diagnostic.message_key, "core/graph-variadic-count");
        assert_eq!(
            diagnostic.message,
            "The number of variadic port instances is outside the allowed range."
        );
        assert!(diagnostic.message_args.is_empty());
        let wire = to_value(&diagnostic).unwrap();
        assert_eq!(wire["code"], "E-VARIADIC-COUNT");
        assert_eq!(
            serde_json::from_value::<DiagnosticCode>(wire["code"].clone()).unwrap(),
            DiagnosticCode::VariadicCount
        );
        assert_eq!(wire["message_key"], "core/graph-variadic-count");
        assert_eq!(wire["message_args"], json!({}));
    }

    #[test]
    fn domain_diagnostics_have_stable_redacted_envelopes() {
        for (code, expected_code, key, message) in [
            (
                DiagnosticCode::DomainUnsupported,
                "E-DOMAIN-UNSUPPORTED",
                "core/graph-domain-unsupported",
                "This execution domain is unavailable.",
            ),
            (
                DiagnosticCode::DomainAnalysisBlocked,
                "E-DOMAIN-ANALYSIS-BLOCKED",
                "core/graph-domain-analysis-blocked",
                "Graph analysis requires supported execution domains.",
            ),
        ] {
            let diagnostic = Diagnostic::new(code, Subject::default(), &origin());
            assert_eq!(diagnostic.code.as_str(), expected_code);
            assert_eq!(diagnostic.message_key, key);
            assert_eq!(diagnostic.message, message);
            assert!(diagnostic.message_args.is_empty());
            let wire = to_value(&diagnostic).unwrap();
            assert_eq!(wire["code"], expected_code);
            assert_eq!(
                serde_json::from_value::<DiagnosticCode>(wire["code"].clone()).unwrap(),
                code
            );
        }
    }
}
