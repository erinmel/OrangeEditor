//! Built-in control nodes and variable access registered as data (11 §8.4).

use std::sync::LazyLock;

use super::{NodeDefinition, Ports, ReferenceField, ReferenceKind, VariableAccess};
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
        references: Vec::new(),
        specializations: None,
        config_schema: None,
    }
}

/// A variable node: its single `value` port is typed by the referenced declaration.
fn variable_definition(id: &str, kind: NodeKind, access: VariableAccess) -> NodeDefinition {
    NodeDefinition {
        ports: Ports::Variable(access),
        references: vec![ReferenceField {
            pointer: "/variable".to_owned(),
            kind: ReferenceKind::Variable,
        }],
        ..definition(id, kind, Some(Suspends::Never), Vec::new())
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
            VariableAccess::Read,
        ),
        variable_definition("core/set-variable", Action, VariableAccess::Write),
    ]
}

/// Reference field of a core ID with no registered definition yet, by ID. Registered
/// definitions (Get and Set Variable) declare their own.
pub(crate) fn core_reference(id: &str) -> Option<&'static ReferenceField> {
    static TABLE: LazyLock<[ReferenceField; 3]> = LazyLock::new(|| {
        let field = |pointer: &str, kind| ReferenceField {
            pointer: pointer.to_owned(),
            kind,
        };
        [
            field("/function", ReferenceKind::Function),
            field("/variable", ReferenceKind::Variable),
            field("/event", ReferenceKind::Event),
        ]
    });
    match id {
        "core/call" => Some(&TABLE[0]),
        "core/update-variable" | "core/set-shared" | "core/update-shared" => Some(&TABLE[1]),
        "core/on-event" | "core/emit" | "core/await-event" => Some(&TABLE[2]),
        _ => None,
    }
}
