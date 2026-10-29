//! Semantic signature snapshots (12 §3.1): projection, canonical key, implicit ports and
//! drift comparison.
//!
//! A snapshot contains semantics only. Presentation (labels, descriptions, groups, editor
//! hints) is removed before hashing, so a locale or catalogue change never changes a key
//! and can never cause `E-SIGNATURE-DRIFT` (G-39).

use std::collections::BTreeMap;

use serde_json::{Map, Value as Json};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::document::{
    Extensions, NodeKind, PortDescriptor, PortDirection, PortKind, SignatureSnapshot, Suspends,
    TypeParamName,
};
use crate::error::IdError;
use crate::id::{PortId, SignatureKey, TypeExpr};
use crate::type_term::{MAX_NESTING, NamedTerm, TypeTerm};

/// Presentation-only fields of a port or snapshot (11 §4.1, 12 §3.1).
const PRESENTATION_KEYS: [&str; 4] = ["label", "description", "group", "editor"];

/// Builds the semantic snapshot of resolved definition ports: presentation removed and
/// type parameters replaced by the node's `type_args`.
pub fn project(
    kind: NodeKind,
    ports: impl IntoIterator<Item = PortDescriptor>,
    type_args: &BTreeMap<TypeParamName, TypeExpr>,
) -> SignatureSnapshot {
    project_with_suspension(kind, None, ports, type_args)
}

/// Builds a semantic snapshot with the definition's declared suspension contract.
/// Always pass the host declaration; node kind alone does not prove that a node always
/// suspends.
pub fn project_with_suspension(
    kind: NodeKind,
    suspends: Option<Suspends>,
    ports: impl IntoIterator<Item = PortDescriptor>,
    type_args: &BTreeMap<TypeParamName, TypeExpr>,
) -> SignatureSnapshot {
    let ports = ports
        .into_iter()
        .map(|mut port| {
            strip_presentation(&mut port.extensions);
            port.ty = port.ty.map(|ty| substitute_params(&ty, type_args));
            if let Some(default) = &mut port.default {
                // Float display text is a non-authoritative rendering hint (12 §6.1).
                // Keep the literal's type and encoded value in semantic identity.
                default.display = None;
            }
            port
        })
        .collect();
    SignatureSnapshot {
        kind,
        suspends,
        ports,
        extensions: Extensions::new(),
    }
}

fn strip_presentation(extensions: &mut Extensions) {
    for key in PRESENTATION_KEYS {
        extensions.shift_remove(key);
    }
}

/// Replaces each `$name` parameter (11 §5.1) that `args` resolves. Unresolved parameters
/// stay for the validator to report. Substitution is simultaneous: parameters inside a
/// replacement are preserved rather than looked up again in `args`.
///
/// The checked form reports malformed source/replacement expressions and results that
/// exceed [`MAX_NESTING`].
pub fn try_substitute_params(
    ty: &TypeExpr,
    args: &BTreeMap<TypeParamName, TypeExpr>,
) -> Result<TypeExpr, SignatureSubstitutionError> {
    let term = TypeTerm::parse(ty).map_err(|source| SignatureSubstitutionError::InvalidSource {
        text: ty.to_string(),
        source,
    })?;
    let substituted = substitute_term_at_depth(&term, args, 0)?;
    let text = substituted.to_string();
    text.parse()
        .map_err(|source| SignatureSubstitutionError::InvalidResult { text, source })
}

/// Replaces parameters structurally. If either expression is malformed or the replacement
/// would exceed the type nesting budget, this compatibility wrapper returns `ty` unchanged.
/// Call [`try_substitute_params`] when the caller needs to distinguish a failed
/// substitution from an unchanged expression.
pub fn substitute_params(ty: &TypeExpr, args: &BTreeMap<TypeParamName, TypeExpr>) -> TypeExpr {
    try_substitute_params(ty, args).unwrap_or_else(|_| ty.clone())
}

/// Failure to substitute an opaque [`TypeExpr`] through the structural type grammar.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SignatureSubstitutionError {
    #[error("signature type `{text}` is not a canonical type expression")]
    InvalidSource {
        text: String,
        #[source]
        source: IdError,
    },
    #[error("type argument `{parameter}` (`{text}`) is not a canonical type expression")]
    InvalidReplacement {
        parameter: TypeParamName,
        text: String,
        #[source]
        source: IdError,
    },
    #[error("substituted type expression `{text}` is invalid")]
    InvalidResult {
        text: String,
        #[source]
        source: IdError,
    },
    #[error("substituted type expression exceeds the type nesting limit")]
    TooDeep,
}

fn substitute_term_at_depth(
    term: &TypeTerm,
    args: &BTreeMap<TypeParamName, TypeExpr>,
    depth: usize,
) -> Result<TypeTerm, SignatureSubstitutionError> {
    if depth > MAX_NESTING {
        return Err(SignatureSubstitutionError::TooDeep);
    }
    match term {
        TypeTerm::Param(name) => match args.get(name) {
            Some(replacement) => {
                let replacement_term = TypeTerm::parse(replacement).map_err(|source| {
                    SignatureSubstitutionError::InvalidReplacement {
                        parameter: name.clone(),
                        text: replacement.to_string(),
                        source,
                    }
                })?;
                check_term_depth(&replacement_term, depth)?;
                Ok(replacement_term)
            }
            None => Ok(term.clone()),
        },
        TypeTerm::Named(named) => {
            let args = named
                .args
                .iter()
                .map(|arg| substitute_term_at_depth(arg, args, depth + 1))
                .collect::<Result<_, _>>()?;
            Ok(TypeTerm::Named(NamedTerm {
                namespace: named.namespace.clone(),
                name: named.name.clone(),
                major: named.major,
                args,
            }))
        }
    }
}

/// Checks a replacement at its insertion depth without recursively substituting its params.
fn check_term_depth(term: &TypeTerm, depth: usize) -> Result<(), SignatureSubstitutionError> {
    if depth > MAX_NESTING {
        return Err(SignatureSubstitutionError::TooDeep);
    }
    if let TypeTerm::Named(named) = term {
        for arg in &named.args {
            check_term_depth(arg, depth + 1)?;
        }
    }
    Ok(())
}

/// Canonical JSON of a snapshot: object keys sorted by byte order, compact, serde_json
/// string escaping. For the ASCII keys and integer numbers of snapshots this matches
/// RFC 8785; cross-language digest vectors are published with the WU-03 contracts.
pub fn canonical_json(snapshot: &SignatureSnapshot) -> String {
    let mut normalized = snapshot.clone();
    if !matches!(normalized.kind, NodeKind::Async | NodeKind::Iterator)
        && normalized.suspends == Some(Suspends::Never)
    {
        normalized.suspends = None;
    }
    #[expect(
        clippy::expect_used,
        reason = "a snapshot has only string-keyed maps, so it always serializes"
    )]
    let value = serde_json::to_value(&normalized).expect("snapshots always serialize");
    sorted(value).to_string()
}

fn sorted(value: Json) -> Json {
    match value {
        Json::Object(map) => {
            let mut entries: Vec<(String, Json)> = map.into_iter().collect();
            entries.sort_by(|(a, _), (b, _)| a.cmp(b));
            Json::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key, sorted(value)))
                    .collect::<Map<_, _>>(),
            )
        }
        Json::Array(items) => Json::Array(items.into_iter().map(sorted).collect()),
        other => other,
    }
}

/// Content-addressed key: identical snapshots share one document entry.
pub fn semantic_key(snapshot: &SignatureSnapshot) -> SignatureKey {
    let digest = Sha256::digest(canonical_json(snapshot).as_bytes());
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    SignatureKey::from_canonical(format!("sha256:{hex}"))
}

impl SignatureSnapshot {
    /// True if `port` is declared or is an implicit port of the snapshot's kind (11 §4.1).
    pub fn declares_port(&self, port: &PortId) -> bool {
        self.ports.iter().any(|declared| &declared.port_id == port)
            || self.implicit_ports().contains(&port.as_str())
    }

    /// Ports a kind gets without declaring them (11 §4.1). Declared exec outputs (outcome
    /// outs) replace `then`.
    pub fn implicit_ports(&self) -> Vec<&'static str> {
        let has_outcomes = self
            .ports
            .iter()
            .any(|port| port.direction == PortDirection::Output && port.kind == PortKind::Exec);
        crate::catalog::implicit_ports(self.kind, has_outcomes)
    }
}

/// How a stored snapshot differs from the one resolved from the current registry (G-29).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SignatureDrift {
    pub kind_changed: bool,
    pub suspension_changed: bool,
    /// Declared in the stored snapshot only: edges using them become orphan edges.
    pub removed: Vec<PortId>,
    pub added: Vec<PortId>,
    /// Declared in both with different semantics (type, direction, source, ...).
    pub changed: Vec<PortId>,
}

/// `None` when the snapshots are semantically identical.
pub fn compare(stored: &SignatureSnapshot, resolved: &SignatureSnapshot) -> Option<SignatureDrift> {
    if semantic_key(stored) == semantic_key(resolved) {
        return None;
    }
    let find = |snapshot: &SignatureSnapshot, id: &PortId| {
        snapshot
            .ports
            .iter()
            .find(|port| &port.port_id == id)
            .cloned()
    };
    let mut drift = SignatureDrift {
        kind_changed: stored.kind != resolved.kind,
        suspension_changed: stored.suspension() != resolved.suspension(),
        ..Default::default()
    };
    for port in &stored.ports {
        match find(resolved, &port.port_id) {
            None => drift.removed.push(port.port_id.clone()),
            Some(other) if other != *port => drift.changed.push(port.port_id.clone()),
            Some(_) => {}
        }
    }
    drift.added = resolved
        .ports
        .iter()
        .filter(|port| find(stored, &port.port_id).is_none())
        .map(|port| port.port_id.clone())
        .collect();
    Some(drift)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn args(pairs: &[(&str, &str)]) -> BTreeMap<TypeParamName, TypeExpr> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.parse().unwrap()))
            .collect()
    }

    fn ty(text: &str) -> TypeExpr {
        text.parse().unwrap()
    }

    #[test]
    fn substitutes_nested_parameters_and_keeps_unknown_ones() {
        let args = args(&[("T", "core/i64@1"), ("K", "core/string@1")]);
        assert_eq!(substitute_params(&ty("$T"), &args), ty("core/i64@1"));
        assert_eq!(
            substitute_params(&ty("core/map@1<$K,core/list@1<$T>>"), &args),
            ty("core/map@1<core/string@1,core/list@1<core/i64@1>>")
        );
        assert_eq!(
            substitute_params(&ty("core/list@1<$U>"), &args),
            ty("core/list@1<$U>")
        );
        assert_eq!(
            substitute_params(&ty("core/bool@1"), &args),
            ty("core/bool@1")
        );
    }

    #[test]
    fn substitution_is_simultaneous_and_parameter_names_are_structural() {
        let substitutions = args(&[("T", "$U"), ("U", "core/i64@1"), ("Type", "core/string@1")]);
        assert_eq!(substitute_params(&ty("$T"), &substitutions), ty("$U"));
        assert_eq!(
            substitute_params(&ty("core/map@1<$T,$Type>"), &substitutions),
            ty("core/map@1<$U,core/string@1>")
        );
        // A cyclic replacement map is harmless because replacement terms are not
        // recursively substituted in the same pass.
        let cyclic = args(&[("T", "$U"), ("U", "$T")]);
        assert_eq!(substitute_params(&ty("$T"), &cyclic), ty("$U"));
    }

    #[test]
    fn checked_substitution_reports_malformed_terms_and_legacy_api_is_opaque() {
        let malformed_source = ty("not a type");
        assert!(matches!(
            try_substitute_params(&malformed_source, &args(&[("T", "core/i64@1")])),
            Err(SignatureSubstitutionError::InvalidSource { .. })
        ));
        assert_eq!(
            substitute_params(&malformed_source, &args(&[("T", "core/i64@1")])),
            malformed_source
        );

        let malformed_replacement = ty("$T");
        let replacements = args(&[("T", "not a type")]);
        assert!(matches!(
            try_substitute_params(&malformed_replacement, &replacements),
            Err(SignatureSubstitutionError::InvalidReplacement { .. })
        ));
        assert_eq!(
            substitute_params(&malformed_replacement, &replacements),
            malformed_replacement
        );
    }

    #[test]
    fn replacement_cannot_expand_past_parser_nesting_budget() {
        let input = ty("core/list@1<$T>");
        let replacement = format!(
            "{}core/i64@1{}",
            "core/list@1<".repeat(MAX_NESTING),
            ">".repeat(MAX_NESTING)
        );
        let replacements = args(&[("T", &replacement)]);
        assert!(matches!(
            try_substitute_params(&input, &replacements),
            Err(SignatureSubstitutionError::TooDeep)
        ));
        assert_eq!(substitute_params(&input, &replacements), input);
    }

    fn snapshot(value: Json) -> SignatureSnapshot {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn canonical_json_sorts_keys_at_every_level() {
        let snapshot = snapshot(json!({"kind": "pure", "ports": [
            {"port_id": "a", "direction": "input", "kind": "data", "type": "core/i64@1", "z_ext": {"b": 1, "a": 2}}
        ]}));
        assert_eq!(
            canonical_json(&snapshot),
            r#"{"kind":"pure","ports":[{"direction":"input","kind":"data","port_id":"a","type":"core/i64@1","z_ext":{"a":2,"b":1}}]}"#
        );
    }

    #[test]
    fn key_is_sha256_of_canonical_json() {
        // Vector: sha256 of the canonical JSON of the 12 §6 `s-add-i64` snapshot, computed
        // independently with `printf '%s' '<canonical json>' | sha256sum`.
        let add = snapshot(json!({"kind": "pure", "ports": [
            {"port_id": "a", "direction": "input", "kind": "data", "type": "core/i64@1"},
            {"port_id": "b", "direction": "input", "kind": "data", "type": "core/i64@1"},
            {"port_id": "sum", "direction": "output", "kind": "data", "type": "core/i64@1", "source": "computed"}
        ]}));
        assert_eq!(
            semantic_key(&add).as_str(),
            include_str!("../tests/fixtures/s-add-i64.sha256").trim()
        );
    }
}
