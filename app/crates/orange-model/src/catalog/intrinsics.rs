//! Port layouts of the intrinsics this core version registers (11 §8.4). Their identity and
//! fixed facts are in `language`.

use super::language::intrinsic_reference;
use super::{EventAccess, NodeDefinition, Ports, VariableAccess};
use crate::document::{DefinitionRef, NodeKind, OutputSource, PortDescriptor, Suspends};
use crate::id::TypeExpr;

fn ty(text: &str) -> TypeExpr {
    text.parse().expect("intrinsic type expression is valid")
}

fn definition(
    id: &str,
    kind: NodeKind,
    suspends: Option<Suspends>,
    ports: Vec<PortDescriptor>,
) -> NodeDefinition {
    NodeDefinition {
        definition: DefinitionRef {
            id: id.parse().expect("intrinsic definition id is valid"),
            version: 1,
        },
        config_version: 1,
        kind,
        suspends,
        type_params: Vec::new(),
        ports: Ports::Declared(ports),
        references: intrinsic_reference(id).into_iter().cloned().collect(),
        specializations: None,
        config_schema: None,
    }
}

/// A variable node: its single `value` port is typed by the referenced declaration.
fn variable_definition(
    id: &str,
    kind: NodeKind,
    suspends: Option<Suspends>,
    access: VariableAccess,
) -> NodeDefinition {
    NodeDefinition {
        ports: Ports::Variable(access),
        ..definition(id, kind, suspends, Vec::new())
    }
}

/// An event node: its `payload` port is typed by the referenced event declaration.
fn event_definition(
    id: &str,
    kind: NodeKind,
    suspends: Option<Suspends>,
    access: EventAccess,
) -> NodeDefinition {
    NodeDefinition {
        ports: Ports::Event(access),
        ..definition(id, kind, suspends, Vec::new())
    }
}

/// Definitions of the built-in control nodes, at definition and config version 1.
pub(crate) fn intrinsic_definitions() -> Vec<NodeDefinition> {
    use NodeKind::{Action, Async, Iterator};
    let exec_in = || PortDescriptor::exec_in("in");
    let sequence_then = PortDescriptor::exec_out("then").variadic(2, None);
    let fork_branch = PortDescriptor::exec_out("branch").variadic(1, None);
    vec![
        definition(
            "core/branch",
            Action,
            None,
            vec![
                exec_in(),
                PortDescriptor::data_in("condition", ty("core/bool@1")),
                PortDescriptor::exec_out("true"),
                PortDescriptor::exec_out("false"),
            ],
        ),
        definition(
            "core/sequence",
            Action,
            None,
            vec![exec_in(), sequence_then],
        ),
        definition(
            "core/fork",
            Action,
            None,
            vec![
                exec_in(),
                PortDescriptor::exec_out("completed"),
                fork_branch,
            ],
        ),
        definition(
            "core/count",
            Iterator,
            Some(Suspends::Maybe),
            vec![
                exec_in(),
                PortDescriptor::data_in("count", ty("core/i64@1")),
                PortDescriptor::data_out("index", ty("core/i64@1"), OutputSource::Iteration),
            ],
        ),
        definition(
            "core/while",
            Iterator,
            Some(Suspends::Maybe),
            vec![
                exec_in(),
                PortDescriptor::data_in("condition", ty("core/bool@1")),
            ],
        ),
        definition("core/yield", Async, Some(Suspends::Always), vec![exec_in()]),
        definition(
            "core/wait",
            Async,
            Some(Suspends::Always),
            vec![
                exec_in(),
                PortDescriptor::data_in("duration", ty("core/duration@1")),
            ],
        ),
        definition("core/break", Action, None, vec![exec_in()]),
        definition("core/continue", Action, None, vec![exec_in()]),
        variable_definition(
            "core/get-variable",
            NodeKind::Volatile,
            Some(Suspends::Never),
            VariableAccess::Read,
        ),
        variable_definition(
            "core/set-variable",
            Action,
            Some(Suspends::Never),
            VariableAccess::Write,
        ),
        variable_definition(
            "core/update-variable",
            Action,
            Some(Suspends::Never),
            VariableAccess::Update,
        ),
        variable_definition(
            "core/set-shared",
            Async,
            Some(Suspends::Maybe),
            VariableAccess::WriteShared,
        ),
        variable_definition(
            "core/update-shared",
            Async,
            Some(Suspends::Maybe),
            VariableAccess::UpdateShared,
        ),
        event_definition("core/on-event", NodeKind::Entry, None, EventAccess::Handle),
        event_definition(
            "core/emit",
            Action,
            Some(Suspends::Never),
            EventAccess::Emit,
        ),
        event_definition(
            "core/await-event",
            Async,
            Some(Suspends::Always),
            EventAccess::Await,
        ),
    ]
}
