//! The language rules the host owns (11 §3, §4.1, §8.4, §11, §14): every intrinsic ID with
//! the fixed facts about it, and the implicit ports each kind gets. Packages can neither add
//! nor replace these (11 §16). Whether this core version registers an intrinsic's contract
//! is separate; see `intrinsic_definitions`.

use crate::catalog::intrinsic_id as ids;
use std::sync::LazyLock;

use super::{EventAccess, ReferenceField, ReferenceKind, VariableAccess};
use crate::document::{
    NodeInstance, NodeKind, OutputSource, PortDescriptor, SignatureSnapshot, Suspends,
};
use crate::id::{PortId, PortKey};

/// Port IDs that kinds supply implicitly (11 §4.1); only intrinsics may declare them.
pub(crate) const IMPLICIT_PORTS: [&str; 7] = [
    "in",
    "then",
    "failed",
    "error",
    "cancelled",
    "body",
    "completed",
];

/// Ports a node of `kind` gets without declaring them (11 §4.1). Declared exec outputs
/// (outcome outs) replace `then`.
pub(crate) fn implicit_ports(kind: NodeKind, has_outcomes: bool) -> Vec<&'static str> {
    use NodeKind::*;
    let mut ports = Vec::new();
    if matches!(kind, Action | Async | Iterator) {
        ports.extend(["in", "failed", "error"]);
    }
    if matches!(kind, Action | Async | Entry) && !has_outcomes {
        ports.push("then");
    }
    if matches!(kind, Async | Iterator) {
        ports.push("cancelled");
    }
    if kind == Iterator {
        ports.extend(["body", "completed"]);
    }
    ports
}

/// The implicit port `id` of a node of `definition` with these ports, if it has one: the
/// kind's (11 §4.1), except that a terminator has no `then` (11 §8.3).
pub(crate) fn implicit_port(
    definition: &str,
    snapshot: &SignatureSnapshot,
    id: &PortId,
) -> Option<PortDescriptor> {
    let name = id.as_str();
    let terminator = intrinsic(definition).is_some_and(|rule| rule.terminator);
    if (terminator && name == "then") || !snapshot.declares_port(id) {
        return None;
    }
    Some(match name {
        "in" => PortDescriptor::exec_in(name),
        "failed" | "cancelled" | "then" | "body" | "completed" => PortDescriptor::exec_out(name),
        "error" => PortDescriptor {
            available_on: Some(vec!["failed".parse().expect("port id is valid")]),
            ..PortDescriptor::data_out(
                name,
                "core/error@1"
                    .parse()
                    .expect("intrinsic type expression is valid"),
                OutputSource::Execution,
            )
        },
        _ => return None,
    })
}

/// The port `key` names on `node` under `snapshot` (11 §4.5), or `None` when the node has no
/// such port. A variadic instance must be active in the node's variadic state and belong to
/// a declared variadic port; a variadic base port is never an endpoint by itself.
///
/// Validation and authoring projections share this rule, so a UI and the validator agree
/// on which edges are orphaned.
pub(crate) fn instance_port(
    node: &NodeInstance,
    snapshot: &SignatureSnapshot,
    key: &PortKey,
) -> Option<PortDescriptor> {
    let base = key.port();
    if let Some(index) = key.instance() {
        let active = node
            .variadic
            .get(base)
            .is_some_and(|state| state.order.contains(&index));
        return snapshot
            .ports
            .iter()
            .find(|candidate| candidate.port_id == *base && candidate.variadic.is_some())
            .filter(|_| active)
            .cloned();
    }
    let variadic_base = node.variadic.contains_key(base)
        || snapshot
            .ports
            .iter()
            .any(|port| port.port_id == *base && port.variadic.is_some());
    if variadic_base {
        return None;
    }
    snapshot
        .ports
        .iter()
        .find(|port| port.port_id == *base)
        .cloned()
        .or_else(|| implicit_port(node.definition.id.as_str(), snapshot, base))
}

/// One language intrinsic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Intrinsic {
    pub(crate) id: &'static str,
    /// Kind and suspension a function body counts it as (11 §14). `None` when function
    /// analysis has no facts for it, including the function-structure markers.
    pub(crate) summary: Option<(NodeKind, Suspends)>,
    /// Kind of the graph declaration its config names, in the standard field.
    pub(crate) reference: Option<ReferenceKind>,
    /// How this exact variable intrinsic derives its declaration-owned ports.
    pub(crate) variable_access: Option<VariableAccess>,
    /// How this exact event intrinsic derives its declaration-owned ports.
    pub(crate) event_access: Option<EventAccess>,
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
        variable_access: None,
        event_access: None,
        terminator: false,
    }
}

const fn event_rule(
    id: &'static str,
    summary: Option<(NodeKind, Suspends)>,
    access: EventAccess,
) -> Intrinsic {
    Intrinsic {
        event_access: Some(access),
        ..rule(id, summary, EVENT)
    }
}

const fn variable_rule(
    id: &'static str,
    summary: Option<(NodeKind, Suspends)>,
    access: VariableAccess,
) -> Intrinsic {
    Intrinsic {
        variable_access: Some(access),
        ..rule(id, summary, VARIABLE)
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
const ENTRY: Option<(NodeKind, Suspends)> = Some((NodeKind::Entry, Suspends::Never));
const ITERATOR: Option<(NodeKind, Suspends)> = Some((NodeKind::Iterator, Suspends::Maybe));
const VARIABLE: Option<ReferenceKind> = Some(ReferenceKind::Variable);
const EVENT: Option<ReferenceKind> = Some(ReferenceKind::Event);
const FUNCTION: Option<ReferenceKind> = Some(ReferenceKind::Function);

/// Every intrinsic of format v1, in the order of 11 §8.4, §11 and §14.
const INTRINSICS: [Intrinsic; 43] = [
    rule(ids::ON_START, ENTRY, None),
    rule(ids::ON_STOP, ENTRY, None),
    rule(ids::CALL, None, FUNCTION),
    rule(ids::FUNCTION_ENTRY, None, None),
    rule(ids::FUNCTION_OUTPUTS, None, None),
    rule(ids::RETURN, None, None),
    variable_rule(ids::GET_VARIABLE, VOLATILE, VariableAccess::Read),
    variable_rule(ids::SET_VARIABLE, ACTION, VariableAccess::Write),
    variable_rule(ids::UPDATE_VARIABLE, ACTION, VariableAccess::Update),
    variable_rule(
        ids::SET_SHARED,
        Some((NodeKind::Async, Suspends::Maybe)),
        VariableAccess::WriteShared,
    ),
    variable_rule(
        ids::UPDATE_SHARED,
        Some((NodeKind::Async, Suspends::Maybe)),
        VariableAccess::UpdateShared,
    ),
    event_rule(ids::ON_EVENT, ENTRY, EventAccess::Handle),
    event_rule(ids::EMIT, ACTION, EventAccess::Emit),
    rule(ids::BRANCH, ACTION, None),
    rule(ids::SWITCH, ACTION, None),
    rule(ids::SEQUENCE, ACTION, None),
    rule(ids::FORK, ACTION, None),
    rule(ids::FOR_EACH, ITERATOR, None),
    rule(ids::COUNT, ITERATOR, None),
    rule(ids::WHILE, ITERATOR, None),
    terminator(ids::BREAK),
    terminator(ids::CONTINUE),
    rule(ids::GATE, ACTION, None),
    rule(ids::DO_ONCE, ACTION, None),
    rule(ids::FLIP_FLOP, ACTION, None),
    rule(ids::YIELD, ASYNC, None),
    rule(ids::WAIT, ASYNC, None),
    event_rule(ids::AWAIT_EVENT, ASYNC, EventAccess::Await),
    rule(ids::REGISTER, ACTION, None),
    rule(ids::SELECT, PURE, None),
    rule(ids::AND, PURE, None),
    rule(ids::OR, PURE, None),
    rule(ids::COALESCE, PURE, None),
    rule(ids::TRY, PURE, None),
    rule(ids::MATCH_RESULT, ACTION, None),
    rule(ids::REPEAT_BEGIN, PURE, None),
    rule(ids::REPEAT_END, PURE, None),
    rule(ids::MAP_BEGIN, PURE, None),
    rule(ids::MAP_END, PURE, None),
    rule(ids::FOLD_BEGIN, PURE, None),
    rule(ids::FOLD_END, PURE, None),
    rule(ids::FILTER_BEGIN, PURE, None),
    rule(ids::FILTER_END, PURE, None),
];

/// The intrinsic with this exact ID. Intrinsic status comes from this table, never from the
/// `core/` prefix alone (11 §3).
pub(crate) fn intrinsic(id: &str) -> Option<&'static Intrinsic> {
    INTRINSICS.iter().find(|rule| rule.id == id)
}

pub(crate) fn is_intrinsic(id: &str) -> bool {
    intrinsic(id).is_some()
}

/// `core/convert` is a host-owned registry special form. It uses entry metadata for ports
/// and may bind an exact package implementation, so it does not follow ordinary intrinsic
/// or ordinary package-definition resolution.
pub(crate) fn is_converter_special_form(id: &str) -> bool {
    id == ids::CONVERT
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
            let variable_access = match definition.ports {
                crate::catalog::Ports::Variable(access) => Some(access),
                crate::catalog::Ports::Declared(_)
                | crate::catalog::Ports::Event(_)
                | crate::catalog::Ports::ForEach => None,
            };
            assert_eq!(rule.variable_access, variable_access, "{id}");
            let event_access = match definition.ports {
                crate::catalog::Ports::Event(access) => Some(access),
                _ => None,
            };
            assert_eq!(rule.event_access, event_access, "{id}");
        }
    }

    fn action(ports: Vec<PortDescriptor>) -> SignatureSnapshot {
        SignatureSnapshot {
            kind: NodeKind::Action,
            suspends: None,
            ports,
            extensions: Default::default(),
        }
    }

    #[test]
    fn implicit_ports_come_from_the_kind_except_a_terminator_then() {
        let then: PortId = "then".parse().unwrap();
        let error = implicit_port("acme/act", &action(vec![]), &"error".parse().unwrap())
            .expect("an action has an error output");
        assert_eq!(error.ty.unwrap().as_str(), "core/error@1");
        assert!(implicit_port("acme/act", &action(vec![]), &then).is_some());
        let outcome = vec![PortDescriptor::exec_out("done")];
        assert!(implicit_port("acme/act", &action(outcome), &then).is_none());
        let exec_in = vec![PortDescriptor::exec_in("in")];
        assert!(implicit_port("core/break", &action(exec_in), &then).is_none());
        for kind in [NodeKind::Pure, NodeKind::Volatile] {
            assert!(implicit_ports(kind, false).is_empty());
        }
        assert!(
            implicit_ports(NodeKind::Iterator, false)
                .iter()
                .all(|id| IMPLICIT_PORTS.contains(id))
        );
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
