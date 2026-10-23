//! The language rules the host owns (11 §3, §8.4, §11, §14): every intrinsic ID and the
//! fixed facts about it. Packages can neither add nor replace these (11 §16). Whether this
//! core version registers an intrinsic's contract is separate; see `intrinsic_definitions`.

use std::sync::LazyLock;

use super::{ReferenceField, ReferenceKind};
use crate::document::{NodeKind, Suspends};

/// One language intrinsic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Intrinsic {
    pub(crate) id: &'static str,
    /// Kind and suspension a function body counts it as (11 §14). `None` when function
    /// analysis has no facts for it, including the function-structure markers.
    pub(crate) summary: Option<(NodeKind, Suspends)>,
    /// Kind of the graph declaration its config names, in the standard field.
    pub(crate) reference: Option<ReferenceKind>,
    /// Ends its region instead of continuing through `then` (11 §8.3).
    pub(crate) terminator: bool,
}

const fn rule(
    id: &'static str,
    summary: Option<(NodeKind, Suspends)>,
    reference: Option<ReferenceKind>,
) -> Intrinsic {
    Intrinsic {
        id,
        summary,
        reference,
        terminator: false,
    }
}

const fn terminator(id: &'static str) -> Intrinsic {
    Intrinsic {
        terminator: true,
        ..rule(id, ACTION, None)
    }
}

const PURE: Option<(NodeKind, Suspends)> = Some((NodeKind::Pure, Suspends::Never));
const VOLATILE: Option<(NodeKind, Suspends)> = Some((NodeKind::Volatile, Suspends::Never));
const ACTION: Option<(NodeKind, Suspends)> = Some((NodeKind::Action, Suspends::Never));
const ASYNC: Option<(NodeKind, Suspends)> = Some((NodeKind::Async, Suspends::Always));
const ITERATOR: Option<(NodeKind, Suspends)> = Some((NodeKind::Iterator, Suspends::Maybe));
const VARIABLE: Option<ReferenceKind> = Some(ReferenceKind::Variable);
const EVENT: Option<ReferenceKind> = Some(ReferenceKind::Event);
const FUNCTION: Option<ReferenceKind> = Some(ReferenceKind::Function);

/// Every intrinsic of format v1, in the order of 11 §8.4, §11 and §14.
const INTRINSICS: [Intrinsic; 41] = [
    rule("core/call", None, FUNCTION),
    rule("core/function-entry", None, None),
    rule("core/function-outputs", None, None),
    rule("core/return", None, None),
    rule("core/get-variable", VOLATILE, VARIABLE),
    rule("core/set-variable", ACTION, VARIABLE),
    rule("core/update-variable", ACTION, VARIABLE),
    rule("core/set-shared", None, VARIABLE),
    rule("core/update-shared", None, VARIABLE),
    rule("core/on-event", None, EVENT),
    rule("core/emit", None, EVENT),
    rule("core/branch", ACTION, None),
    rule("core/switch", ACTION, None),
    rule("core/sequence", ACTION, None),
    rule("core/fork", ACTION, None),
    rule("core/for-each", ITERATOR, None),
    rule("core/count", ITERATOR, None),
    rule("core/while", ITERATOR, None),
    terminator("core/break"),
    terminator("core/continue"),
    rule("core/gate", ACTION, None),
    rule("core/do-once", ACTION, None),
    rule("core/flip-flop", ACTION, None),
    rule("core/yield", ASYNC, None),
    rule("core/wait", ASYNC, None),
    rule("core/await-event", None, EVENT),
    rule("core/register", None, None),
    rule("core/select", PURE, None),
    rule("core/and", PURE, None),
    rule("core/or", PURE, None),
    rule("core/coalesce", PURE, None),
    rule("core/try", PURE, None),
    rule("core/match-result", None, None),
    rule("core/repeat-begin", None, None),
    rule("core/repeat-end", None, None),
    rule("core/map-begin", None, None),
    rule("core/map-end", None, None),
    rule("core/fold-begin", None, None),
    rule("core/fold-end", None, None),
    rule("core/filter-begin", None, None),
    rule("core/filter-end", None, None),
];

/// The intrinsic with this exact ID. Intrinsic status comes from this table, never from the
/// `core/` prefix alone (11 §3).
pub(crate) fn intrinsic(id: &str) -> Option<&'static Intrinsic> {
    INTRINSICS.iter().find(|rule| rule.id == id)
}

pub(crate) fn is_intrinsic(id: &str) -> bool {
    intrinsic(id).is_some()
}

/// The standard config field naming a declaration of `kind`: `/variable`, `/event` or
/// `/function`.
pub(crate) fn reference_field(kind: ReferenceKind) -> &'static ReferenceField {
    static FIELDS: LazyLock<[ReferenceField; 3]> = LazyLock::new(|| {
        let field = |pointer: &str, kind| ReferenceField {
            pointer: pointer.to_owned(),
            kind,
        };
        [
            field("/variable", ReferenceKind::Variable),
            field("/event", ReferenceKind::Event),
            field("/function", ReferenceKind::Function),
        ]
    });
    match kind {
        ReferenceKind::Variable => &FIELDS[0],
        ReferenceKind::Event => &FIELDS[1],
        ReferenceKind::Function => &FIELDS[2],
    }
}

/// The reference field an intrinsic's config holds, if any.
pub(crate) fn intrinsic_reference(id: &str) -> Option<&'static ReferenceField> {
    intrinsic(id)?.reference.map(reference_field)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::catalog::intrinsics::intrinsic_definitions;

    #[test]
    fn intrinsic_ids_are_unique_core_definition_ids() {
        let mut seen = BTreeSet::new();
        for rule in &INTRINSICS {
            let id: crate::id::DefinitionId = rule.id.parse().unwrap();
            assert!(id.is_core(), "{}", rule.id);
            assert!(seen.insert(rule.id), "duplicate {}", rule.id);
        }
        assert!(!is_intrinsic("core/not-an-intrinsic"));
    }

    #[test]
    fn registered_intrinsics_agree_with_the_table() {
        for definition in intrinsic_definitions() {
            let id = definition.definition.id.as_str();
            let rule = intrinsic(id).unwrap_or_else(|| panic!("{id} is not in the table"));
            let (kind, suspends) = rule.summary.expect("a registered intrinsic has a kind");
            assert_eq!(definition.kind, kind, "{id}");
            assert_eq!(
                definition.suspends.unwrap_or(Suspends::Never),
                suspends,
                "{id}"
            );
            assert_eq!(
                definition.references,
                intrinsic_reference(id)
                    .into_iter()
                    .cloned()
                    .collect::<Vec<_>>(),
                "{id}"
            );
        }
    }

    #[test]
    fn only_break_and_continue_terminate() {
        let terminators: Vec<_> = INTRINSICS
            .iter()
            .filter(|rule| rule.terminator)
            .map(|rule| rule.id)
            .collect();
        assert_eq!(terminators, ["core/break", "core/continue"]);
    }
}
