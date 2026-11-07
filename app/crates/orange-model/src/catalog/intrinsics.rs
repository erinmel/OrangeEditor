//! Port layouts of the intrinsics this core version registers (11 §8.4). Their identity and
//! fixed facts are in `language`.

use super::language::intrinsic_reference;
use super::{ContractError, EventAccess, NodeContract, NodeDefinition, Ports, VariableAccess};
use crate::catalog::intrinsic_id as ids;
use crate::document::{
    DefinitionRef, NodeInstance, NodeKind, OutputSource, PortDescriptor, Suspends, TypeParam,
};
use crate::id::TypeExpr;

fn ty(text: &'static str) -> TypeExpr {
    crate::id::fixed_id(text)
}

fn definition(
    id: &'static str,
    kind: NodeKind,
    suspends: Option<Suspends>,
    ports: Vec<PortDescriptor>,
) -> NodeDefinition {
    NodeDefinition {
        definition: DefinitionRef {
            id: crate::id::fixed_id(id),
            version: 1,
        },
        config_version: 1,
        kind,
        volatility: None,
        determinism: super::Determinism::Deterministic,
        suspends,
        type_params: Vec::new(),
        ports: Ports::Declared(ports),
        references: intrinsic_reference(id).into_iter().cloned().collect(),
        specializations: None,
        config_schema: None,
    }
}

/// A generic copy of `base`: parameter `name` with these constraints.
fn generic(mut base: NodeDefinition, name: &str, constraints: &[&str]) -> NodeDefinition {
    base.type_params.push(TypeParam {
        name: name.into(),
        constraints: constraints.iter().map(|text| (*text).to_owned()).collect(),
        extensions: Default::default(),
    });
    base
}

/// A variable node: its single `value` port is typed by the referenced declaration.
fn variable_definition(
    id: &'static str,
    kind: NodeKind,
    suspends: Option<Suspends>,
    access: VariableAccess,
) -> NodeDefinition {
    NodeDefinition {
        volatility: (access == VariableAccess::Read).then_some(super::Volatility::StateRead),
        ports: Ports::Variable(access),
        config_schema: (access == VariableAccess::Update).then(|| {
            serde_json::json!({
                "type":"object","properties":{
                    "int_overflow":{"type":"string","enum":["checked","wrapping","saturating"]},
                    "float_policy":{"type":"string","enum":["finite","ieee"]}
                }
            })
        }),
        ..definition(id, kind, suspends, Vec::new())
    }
}

/// An event node: its `payload` port is typed by the referenced event declaration.
fn event_definition(
    id: &'static str,
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
    let mut definitions = core_control_definitions();
    definitions.extend(selection_and_state_definitions());
    definitions.extend(register_definitions());
    definitions.extend(result_definitions());
    definitions.extend(lazy_pure_definitions());
    definitions.extend(stream_definitions());
    definitions.push(generic(
        definition(
            ids::CLOSE_HANDLE,
            NodeKind::Action,
            Some(Suspends::Never),
            vec![PortDescriptor::data_in("handle", ty("core/handle@1<$K>"))],
        ),
        "K",
        &[],
    ));
    let mut watch = generic(
        definition(
            ids::ON_WATCH,
            NodeKind::Entry,
            None,
            vec![
                PortDescriptor::data_in("watch", ty("$T")),
                PortDescriptor::data_out("value", ty("$T"), OutputSource::Payload),
            ],
        ),
        "T",
        &["eq"],
    );
    watch.config_schema = Some(
        serde_json::json!({"type":"object","required":["mode"],"properties":{"mode":{"type":"string","enum":["changed","rising"]}},"additionalProperties":false}),
    );
    definitions.push(watch);
    definitions.push(NodeDefinition {
        ports: Ports::ForEach,
        ..definition(
            ids::FOR_EACH,
            NodeKind::Iterator,
            Some(Suspends::Maybe),
            Vec::new(),
        )
    });
    definitions.push(NodeDefinition {
        ports: Ports::Manual,
        ..definition(ids::ON_MANUAL, NodeKind::Entry, None, Vec::new())
    });
    // The clock reader: no inputs, read once per epoch like a state read (11 §7.4).
    definitions.push(NodeDefinition {
        volatility: Some(super::Volatility::Clock),
        determinism: super::Determinism::External,
        ..definition(
            ids::NOW,
            NodeKind::Volatile,
            None,
            vec![PortDescriptor::data_out(
                "elapsed",
                ty("core/duration@1"),
                // Host reads publish like state reads: one host input per epoch.
                OutputSource::State,
            )],
        )
    });
    for id in [ids::ON_TIMER, ids::ON_TIMEOUT] {
        definitions.push(NodeDefinition {
            ports: Ports::Timer,
            ..definition(id, NodeKind::Entry, None, Vec::new())
        });
    }
    definitions.extend(zone_definitions());
    definitions
}

fn stream_definitions() -> Vec<NodeDefinition> {
    let input = || PortDescriptor::data_in("stream", ty("core/stream@1<$T>"));
    let next = generic(
        definition(
            ids::NEXT_ITEM,
            NodeKind::Async,
            Some(Suspends::Maybe),
            vec![
                input(),
                on(
                    PortDescriptor::data_out(
                        "item",
                        ty("core/option@1<$T>"),
                        OutputSource::Execution,
                    ),
                    &["then"],
                ),
            ],
        ),
        "T",
        &[],
    );
    let mut collect = generic(
        definition(
            ids::COLLECT,
            NodeKind::Async,
            Some(Suspends::Maybe),
            vec![
                input(),
                on(
                    PortDescriptor::data_out(
                        "items",
                        ty("core/list@1<$T>"),
                        OutputSource::Execution,
                    ),
                    &["then"],
                ),
            ],
        ),
        "T",
        &[],
    );
    collect.config_schema = Some(
        serde_json::json!({"type":"object","required":["max_items"],"properties":{"max_items":{"type":"integer","minimum":0,"maximum":4294967295u64}},"additionalProperties":false}),
    );
    let iterate = generic(
        definition(
            ids::FOR_EACH_ITEM,
            NodeKind::Iterator,
            Some(Suspends::Maybe),
            vec![
                input(),
                PortDescriptor::data_out("item", ty("$T"), OutputSource::Iteration),
                PortDescriptor::data_out("index", ty("core/i64@1"), OutputSource::Iteration),
            ],
        ),
        "T",
        &[],
    );
    let close = generic(
        definition(
            ids::CLOSE_STREAM,
            NodeKind::Action,
            Some(Suspends::Never),
            vec![input()],
        ),
        "T",
        &[],
    );
    let mut tee = generic(
        definition(
            ids::TEE,
            NodeKind::Action,
            Some(Suspends::Never),
            vec![
                input(),
                on(
                    PortDescriptor::data_out(
                        "left",
                        ty("core/stream@1<$T>"),
                        OutputSource::Execution,
                    ),
                    &["then"],
                ),
                on(
                    PortDescriptor::data_out(
                        "right",
                        ty("core/stream@1<$T>"),
                        OutputSource::Execution,
                    ),
                    &["then"],
                ),
            ],
        ),
        "T",
        &[],
    );
    tee.config_schema = Some(
        serde_json::json!({"type":"object","required":["capacity","max_bytes","overflow"],"properties":{
        "capacity":{"type":"integer","minimum":1,"maximum":4294967295u64},"max_bytes":{"type":"integer","minimum":1,"maximum":4294967295u64},
        "overflow":{"type":"string","enum":["backpressure","drop-oldest","drop-newest","fault"]}},"additionalProperties":false}),
    );
    let mut definitions = vec![next, collect, iterate, close, tee];
    for definition in &mut definitions {
        if definition.config_schema.is_none() {
            definition.config_schema =
                Some(serde_json::json!({"type":"object","additionalProperties":false}));
        }
    }
    definitions
}

/// `core/for-each` ports for its `config.mode` (11 §8.4): `list` (default) iterates
/// `element`, `map` iterates `key` and `value`. Only the type parameters the mode uses exist.
pub(crate) fn for_each_contract(
    definition: &NodeDefinition,
    node: &NodeInstance,
) -> Result<NodeContract, ContractError> {
    let param = |name: &str, constraints: &[&str]| TypeParam {
        name: name.into(),
        constraints: constraints.iter().map(|text| (*text).to_owned()).collect(),
        extensions: Default::default(),
    };
    let iteration = |id, text| PortDescriptor::data_out(id, ty(text), OutputSource::Iteration);
    let mode = match node.config.get("mode") {
        None => "list",
        Some(value) => value.as_str().ok_or(ContractError::ForEachMode)?,
    };
    let (type_params, items, mut element) = match mode {
        "list" => (
            vec![param("T", &[])],
            "core/list@1<$T>",
            vec![iteration("element", "$T")],
        ),
        "map" => (
            vec![param("K", &["hash", "eq"]), param("V", &[])],
            "core/map@1<$K,$V>",
            vec![iteration("key", "$K"), iteration("value", "$V")],
        ),
        _ => return Err(ContractError::ForEachMode),
    };
    let mut ports = vec![
        PortDescriptor::exec_in("in"),
        PortDescriptor::data_in("items", ty(items)),
        PortDescriptor::exec_out("body"),
        PortDescriptor::exec_out("completed"),
        iteration("index", "core/i64@1"),
    ];
    ports.append(&mut element);
    Ok(NodeContract {
        kind: definition.kind,
        suspends: definition.suspends,
        type_params,
        ports,
    })
}

/// `core/on-manual` ports (11 §10.2): the implicit `then` plus a `payload` output typed by
/// `config.payload`, which must name a concrete registered type. Without that field the
/// entry carries no payload. No other config field is defined.
pub(crate) fn manual_contract(
    definition: &NodeDefinition,
    node: &NodeInstance,
    types: &crate::types::TypeRegistry,
) -> Result<NodeContract, ContractError> {
    if node.config.keys().any(|key| key != "payload") {
        return Err(ContractError::ManualPayload);
    }
    let mut ports = Vec::new();
    if let Some(payload) = node.config.get("payload") {
        let payload: TypeExpr = payload
            .as_str()
            .and_then(|text| text.parse().ok())
            .ok_or(ContractError::ManualPayload)?;
        let term = crate::type_term::TypeTerm::parse(&payload).map_err(|error| match error {
            crate::IdError::TypeTooDeep { .. } => ContractError::TypeComplexity,
            _ => ContractError::ManualPayload,
        })?;
        types.validate(&term).map_err(|error| match error {
            crate::types::TypeResolveError::TooDeep { .. }
            | crate::types::TypeResolveError::DerivationTooDeep { .. } => {
                ContractError::TypeComplexity
            }
            _ => ContractError::ManualPayload,
        })?;
        ports.push(PortDescriptor::data_out(
            "payload",
            payload,
            OutputSource::Payload,
        ));
    }
    Ok(NodeContract {
        kind: definition.kind,
        suspends: definition.suspends,
        type_params: Vec::new(),
        ports,
    })
}

/// `core/on-timer` and `core/on-timeout` ports after their config is checked (11 §10.2):
/// `scheduled_at` for both, plus `missed` for the interval timer.
pub(crate) fn timer_contract(
    definition: &NodeDefinition,
    node: &NodeInstance,
) -> Result<NodeContract, ContractError> {
    let schedule = super::TimerSchedule::from_node(node).ok_or(ContractError::TimerConfig)?;
    let mut ports = vec![PortDescriptor::data_out(
        "scheduled_at",
        ty("core/duration@1"),
        OutputSource::Payload,
    )];
    if matches!(schedule, super::TimerSchedule::Interval { .. }) {
        ports.push(PortDescriptor::data_out(
            "missed",
            ty("core/u64@1"),
            OutputSource::Payload,
        ));
    }
    Ok(NodeContract {
        kind: definition.kind,
        suspends: definition.suspends,
        type_params: Vec::new(),
        ports,
    })
}

/// Zone begin/end pairs (11 §8.4): pure nodes with no exec ports; the pair carries the items.
fn zone_definitions() -> Vec<NodeDefinition> {
    let iteration = |id, text| PortDescriptor::data_out(id, ty(text), OutputSource::Iteration);
    let computed = |id, text| PortDescriptor::data_out(id, ty(text), OutputSource::Computed);
    let input = |id, text| PortDescriptor::data_in(id, ty(text));
    let pure = |id, ports, params: &[&str]| {
        params
            .iter()
            .fold(definition(id, NodeKind::Pure, None, ports), |node, name| {
                generic(node, name, &[])
            })
    };
    vec![
        pure(
            ids::REPEAT_BEGIN,
            vec![
                input("count", "core/i64@1"),
                input("initial", "$A"),
                iteration("index", "core/i64@1"),
                iteration("carried", "$A"),
            ],
            &["A"],
        ),
        pure(
            ids::REPEAT_END,
            vec![input("next", "$A"), computed("result", "$A")],
            &["A"],
        ),
        pure(
            ids::MAP_BEGIN,
            vec![
                input("items", "core/list@1<$T>"),
                iteration("element", "$T"),
                iteration("index", "core/i64@1"),
            ],
            &["T"],
        ),
        pure(
            ids::MAP_END,
            vec![input("item", "$U"), computed("result", "core/list@1<$U>")],
            &["U"],
        ),
        pure(
            ids::FOLD_BEGIN,
            vec![
                input("items", "core/list@1<$T>"),
                input("initial", "$A"),
                iteration("element", "$T"),
                iteration("index", "core/i64@1"),
                iteration("accumulator", "$A"),
            ],
            &["T", "A"],
        ),
        pure(
            ids::FOLD_END,
            vec![input("next", "$A"), computed("result", "$A")],
            &["A"],
        ),
        pure(
            ids::FILTER_BEGIN,
            vec![
                input("items", "core/list@1<$T>"),
                iteration("element", "$T"),
                iteration("index", "core/i64@1"),
            ],
            &["T"],
        ),
        pure(
            ids::FILTER_END,
            vec![
                input("keep", "core/bool@1"),
                input("item", "$T"),
                computed("result", "core/list@1<$T>"),
            ],
            &["T"],
        ),
    ]
}

/// `select`, `and`, `or` and `coalesce` (11 §8.4): pure nodes whose lazy inputs are evaluated
/// only when selected. The spec names no ports, so the strict reading is: the deciding input
/// (`condition`, `a`, `value`) is eager and the alternatives are lazy.
fn lazy_pure_definitions() -> Vec<NodeDefinition> {
    let pure = |id, ports| definition(id, NodeKind::Pure, None, ports);
    let boolean = || ty("core/bool@1");
    let logic = |id| {
        pure(
            id,
            vec![
                PortDescriptor::data_in("a", boolean()),
                PortDescriptor::data_in("b", boolean()).lazy(),
                PortDescriptor::data_out("result", boolean(), OutputSource::Computed),
            ],
        )
    };
    vec![
        generic(
            pure(
                ids::SELECT,
                vec![
                    PortDescriptor::data_in("condition", boolean()),
                    PortDescriptor::data_in("if_true", ty("$T")).lazy(),
                    PortDescriptor::data_in("if_false", ty("$T")).lazy(),
                    PortDescriptor::data_out("result", ty("$T"), OutputSource::Computed),
                ],
            ),
            "T",
            &[],
        ),
        logic(ids::AND),
        logic(ids::OR),
        generic(
            pure(
                ids::COALESCE,
                vec![
                    PortDescriptor::data_in("value", ty("core/option@1<$T>")),
                    PortDescriptor::data_in("fallback", ty("$T")).lazy(),
                    PortDescriptor::data_out("result", ty("$T"), OutputSource::Computed),
                ],
            ),
            "T",
            &[],
        ),
    ]
}

/// `available_on` outcomes of an execution-sourced output.
fn on(mut port: PortDescriptor, outcomes: &[&'static str]) -> PortDescriptor {
    port.available_on = Some(outcomes.iter().map(|&id| crate::id::fixed_id(id)).collect());
    port
}

/// `core/try` and `core/match-result` (11 §13): the only place an error becomes ordinary data.
/// `try` is a pure node whose input is lazy so its cone's errors are captured; `match-result`
/// replaces `then` with the `ok`/`err` outcomes and exposes the unwrapped value or error only
/// after the matching outcome.
fn result_definitions() -> Vec<NodeDefinition> {
    vec![
        generic(
            definition(
                ids::TRY,
                NodeKind::Pure,
                None,
                vec![
                    PortDescriptor::data_in("value", ty("$T")).lazy(),
                    PortDescriptor::data_out(
                        "result",
                        ty("core/result@1<$T>"),
                        OutputSource::Computed,
                    ),
                ],
            ),
            "T",
            &[],
        ),
        generic(
            definition(
                ids::MATCH_RESULT,
                NodeKind::Action,
                None,
                vec![
                    PortDescriptor::exec_in("in"),
                    PortDescriptor::data_in("result", ty("core/result@1<$T>")),
                    PortDescriptor::exec_out("ok"),
                    PortDescriptor::exec_out("err"),
                    on(
                        PortDescriptor::data_out("value", ty("$T"), OutputSource::Execution),
                        &["ok"],
                    ),
                    on(
                        PortDescriptor::data_out(
                            "error",
                            ty("core/error@1"),
                            OutputSource::Execution,
                        ),
                        &["err"],
                    ),
                ],
            ),
            "T",
            &[],
        ),
    ]
}

/// `core/register`: an anonymous typed state cell (11 §8.4). `store` writes `next` and `reset`
/// restores `initial`; `current` is a volatile read of the cell, so feedback through it adds
/// no data dependency (11 §8.1). `initial`/`scope` are config the model does not enforce.
fn register_definitions() -> Vec<NodeDefinition> {
    vec![generic(
        definition(
            ids::REGISTER,
            NodeKind::Action,
            None,
            vec![
                PortDescriptor::exec_in("store"),
                PortDescriptor::exec_in("reset"),
                PortDescriptor::data_in("next", ty("$T")),
                PortDescriptor::data_out("current", ty("$T"), OutputSource::State),
            ],
        ),
        "T",
        &[],
    )]
}

fn core_control_definitions() -> Vec<NodeDefinition> {
    use NodeKind::{Action, Async, Iterator};
    let exec_in = || PortDescriptor::exec_in("in");
    let sequence_then = PortDescriptor::exec_out("then").variadic(2, None);
    let fork_branch = PortDescriptor::exec_out("branch").variadic(1, None);
    vec![
        definition(ids::ON_START, NodeKind::Entry, None, vec![]),
        definition(ids::ON_STOP, NodeKind::Entry, None, vec![]),
        definition(
            ids::BRANCH,
            Action,
            None,
            vec![
                exec_in(),
                PortDescriptor::data_in("condition", ty("core/bool@1")),
                PortDescriptor::exec_out("true"),
                PortDescriptor::exec_out("false"),
            ],
        ),
        definition(ids::SEQUENCE, Action, None, vec![exec_in(), sequence_then]),
        definition(
            ids::FORK,
            Action,
            None,
            vec![
                exec_in(),
                PortDescriptor::exec_out("completed"),
                fork_branch,
            ],
        ),
        definition(
            ids::COUNT,
            Iterator,
            Some(Suspends::Maybe),
            vec![
                exec_in(),
                PortDescriptor::data_in("count", ty("core/i64@1")),
                PortDescriptor::data_out("index", ty("core/i64@1"), OutputSource::Iteration),
            ],
        ),
        definition(
            ids::WHILE,
            Iterator,
            Some(Suspends::Maybe),
            vec![
                exec_in(),
                PortDescriptor::data_in("condition", ty("core/bool@1")),
            ],
        ),
        definition(ids::YIELD, Async, Some(Suspends::Always), vec![exec_in()]),
        definition(
            ids::WAIT,
            Async,
            Some(Suspends::Always),
            vec![
                exec_in(),
                PortDescriptor::data_in("duration", ty("core/duration@1")),
            ],
        ),
        definition(ids::BREAK, Action, None, vec![exec_in()]),
        definition(ids::CONTINUE, Action, None, vec![exec_in()]),
        variable_definition(
            ids::GET_VARIABLE,
            NodeKind::Volatile,
            Some(Suspends::Never),
            VariableAccess::Read,
        ),
        variable_definition(
            ids::SET_VARIABLE,
            Action,
            Some(Suspends::Never),
            VariableAccess::Write,
        ),
        variable_definition(
            ids::UPDATE_VARIABLE,
            Action,
            Some(Suspends::Never),
            VariableAccess::Update,
        ),
        variable_definition(
            ids::SET_SHARED,
            Async,
            Some(Suspends::Maybe),
            VariableAccess::WriteShared,
        ),
        variable_definition(
            ids::UPDATE_SHARED,
            Async,
            Some(Suspends::Maybe),
            VariableAccess::UpdateShared,
        ),
        variable_definition(
            ids::ON_VARIABLE_CHANGED,
            NodeKind::Entry,
            None,
            VariableAccess::Changed,
        ),
        event_definition(ids::ON_EVENT, NodeKind::Entry, None, EventAccess::Handle),
        event_definition(ids::EMIT, Action, Some(Suspends::Never), EventAccess::Emit),
        event_definition(
            ids::AWAIT_EVENT,
            Async,
            Some(Suspends::Always),
            EventAccess::Await,
        ),
    ]
}

/// Switch and the run-scoped state controls (11 §8.4). Their outcome outs replace `then`;
/// `case`/`default` are ordinary outs of the parent region (11 §7.1).
fn selection_and_state_definitions() -> Vec<NodeDefinition> {
    use NodeKind::Action;
    let bool_state = |id| PortDescriptor::data_out(id, ty("core/bool@1"), OutputSource::State);
    vec![
        generic(
            definition(
                ids::SWITCH,
                Action,
                None,
                vec![
                    PortDescriptor::exec_in("in"),
                    PortDescriptor::data_in("key", ty("$K")),
                    PortDescriptor::exec_out("case").variadic(1, None),
                    PortDescriptor::exec_out("default"),
                ],
            ),
            "K",
            &["eq"],
        ),
        {
            let mut gate = definition(
                ids::GATE,
                Action,
                None,
                vec![
                    PortDescriptor::exec_in("enter"),
                    PortDescriptor::exec_in("open"),
                    PortDescriptor::exec_in("close"),
                    PortDescriptor::exec_in("toggle"),
                    bool_state("is_open"),
                ],
            );
            gate.config_schema = Some(
                serde_json::json!({"type":"object","properties":{"start_open":{"type":"boolean"}}}),
            );
            gate
        },
        definition(
            ids::DO_ONCE,
            Action,
            None,
            vec![
                PortDescriptor::exec_in("in"),
                PortDescriptor::exec_in("reset"),
            ],
        ),
        definition(
            ids::FLIP_FLOP,
            Action,
            None,
            vec![
                PortDescriptor::exec_in("in"),
                PortDescriptor::exec_out("a"),
                PortDescriptor::exec_out("b"),
                bool_state("is_a"),
            ],
        ),
    ]
}
