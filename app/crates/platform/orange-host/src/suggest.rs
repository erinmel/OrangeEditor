//! Which node kinds can take a wire from an output, and which can be put into a wire:
//! what an editor lists when a wire is dropped on empty canvas or a node is inserted on
//! a wire, so the next node is chosen from the ones that fit.
//!
//! The answer comes from each kind's declared ports, before a node exists. A step output
//! fits any kind with a way in. A value output fits a value input of the same type, or
//! of a declared type over the kind's own parameters once they are bound
//! ([`PortFit`]): `list<$T>` takes `list<i64>` and binds `T`, never a
//! bare `i64`, and the bound type must satisfy the parameter's constraints. Put into a
//! wire, the kind must also go on into the wire's input: a step through `then` (or a
//! loop's `completed`), never through a failure or cancellation route or a loop body,
//! and a value through an output whose type, with the same bindings, is the input's.
//! It is a shortlist, not a promise: connecting still runs the language's checks and
//! refuses what does not fit with the reason.
use super::*;
use orange_model::catalog::{Ports, VariableAccess, implicit_port_names};
use orange_model::document::{NodeKind, PortDirection, PortKind};
use orange_model::fit::PortFit;
use std::collections::HashMap;

/// A node kind that can take a wire from one output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    /// The definition, such as `core/set-variable`.
    pub definition: String,
    /// What the kind is called.
    pub label: String,
    /// The input of a new node of this kind the wire would go to.
    pub input: String,
}

/// A node kind that can be put into one wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpliceSuggestion {
    /// The definition, such as `org.orange.collections/reverse`.
    pub definition: String,
    /// What the kind is called.
    pub label: String,
    /// The input of a new node of this kind the wire's output goes to.
    pub input: String,
    /// The output of the new node that goes on to the wire's input.
    pub output: String,
}

/// A node kind that can feed one input: what a wire dragged back from an input lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Giver {
    /// The definition, such as `core/text`.
    pub definition: String,
    /// What the kind is called.
    pub label: String,
    /// The output of a new node of this kind the wire would come from.
    pub output: String,
}

/// Outputs that never carry the flow on by guess: a failure or cancellation route, its
/// error value, and a loop's body, which runs per item rather than after the loop.
pub(crate) const NOT_A_CONTINUATION: [&str; 4] = ["failed", "cancelled", "error", "body"];

/// The input of a kind a wire fits, if any: `exec` for a step, else a value of type
/// `ty`, binding the kind's parameters in `fit`.
fn fitting_input(
    definition: &NodeDefinition,
    exec: bool,
    ty: Option<&str>,
    fit: &mut PortFit<'_>,
) -> Option<String> {
    let id = definition.definition.id.as_str();
    if !exec
        && ty == Some("core/bool@1")
        && matches!(
            id,
            orange_model::catalog::intrinsic_id::ALL_CONDITIONS
                | orange_model::catalog::intrinsic_id::ANY_CONDITIONS
        )
    {
        return Some("condition#0".to_owned());
    }
    match &definition.ports {
        Ports::Declared(ports) => {
            let inputs = ports
                .iter()
                .filter(|port| port.direction == PortDirection::Input && port.variadic.is_none());
            if exec {
                let outcomes = ports.iter().any(|port| {
                    port.direction == PortDirection::Output && port.kind != PortKind::Data
                });
                if implicit_port_names(id, definition.kind, outcomes).contains(&"in") {
                    return Some("in".to_owned());
                }
                return inputs
                    .filter(|port| port.kind != PortKind::Data)
                    .map(|port| port.port_id.to_string())
                    .next();
            }
            let wanted: TypeExpr = ty?.parse().ok()?;
            inputs
                .filter(|port| port.kind == PortKind::Data)
                .find(|port| {
                    port.ty
                        .as_ref()
                        .is_some_and(|declared| fit.meets(declared, &wanted))
                })
                .map(|port| port.port_id.to_string())
        }
        // A variable node has its value input only once its variable is chosen, so a
        // new one takes a step and nothing else.
        Ports::Variable(access) => (exec
            && !matches!(access, VariableAccess::Read | VariableAccess::Changed))
        .then(|| "in".to_owned()),
        // A for-each loop takes its list or map on `items` and starts from a step.
        Ports::ForEach => {
            if exec {
                Some("in".to_owned())
            } else {
                edit::collection_mode(ty?).map(|_| "items".to_owned())
            }
        }
        Ports::Event(_) => (exec && definition.kind != NodeKind::Entry).then(|| "in".to_owned()),
        // Triggers take no wire; record and enum nodes are reached through their fields.
        Ports::Manual | Ports::Timer | Ports::Nominal(_) => None,
    }
}

/// The output of a kind that goes on into an input: for a step, `then`, else a loop's
/// `completed`, else its first other step output; for a value, the first value output
/// whose type, with the bindings `fit` already holds, is `ty`.
fn continuation(
    definition: &NodeDefinition,
    exec: bool,
    ty: Option<&str>,
    fit: &mut PortFit<'_>,
) -> Option<String> {
    let id = definition.definition.id.as_str();
    let declared = match &definition.ports {
        Ports::Declared(ports) => ports.as_slice(),
        Ports::ForEach if exec => return Some("completed".to_owned()),
        Ports::Variable(access) if exec => {
            return (!matches!(access, VariableAccess::Read | VariableAccess::Changed))
                .then(|| "then".to_owned());
        }
        Ports::Event(_) if exec && definition.kind != NodeKind::Entry => {
            return implicit_port_names(id, definition.kind, false)
                .contains(&"then")
                .then(|| "then".to_owned());
        }
        // A trigger starts a step through `then`.
        Ports::Manual | Ports::Timer if exec => return Some("then".to_owned()),
        _ => return None,
    };
    let outputs = declared
        .iter()
        .filter(|port| port.direction == PortDirection::Output && port.variadic.is_none())
        .filter(|port| !NOT_A_CONTINUATION.contains(&port.port_id.as_str()));
    if exec {
        let outcomes = declared
            .iter()
            .any(|port| port.direction == PortDirection::Output && port.kind != PortKind::Data);
        let implicit = implicit_port_names(id, definition.kind, outcomes);
        for name in ["then", "completed"] {
            if implicit.contains(&name) {
                return Some(name.to_owned());
            }
        }
        return outputs
            .filter(|port| port.kind != PortKind::Data)
            .map(|port| port.port_id.to_string())
            .next();
    }
    let wanted: TypeExpr = ty?.parse().ok()?;
    outputs
        .filter(|port| port.kind == PortKind::Data)
        .find(|port| {
            port.ty
                .as_ref()
                .is_some_and(|declared| fit.clone().meets(declared, &wanted))
        })
        .map(|port| port.port_id.to_string())
}

impl Host {
    /// Actual outputs that can feed the input of a backward wire drag. Candidate
    /// nodes and their ports come from the current graph projection. The document
    /// registry checks open types and confirms that passive repair cards cannot wire.
    pub fn output_givers(
        &self,
        draft: &Draft,
        graph: &GraphView,
        to: PortEnd<'_>,
    ) -> HashMap<NodeId, Vec<String>> {
        let document = draft.document();
        if self.require_port_authority(document, to.0).is_err() {
            return HashMap::new();
        }
        let Some(input) = graph
            .nodes
            .iter()
            .find(|node| node.id == to.0)
            .and_then(|node| node.ports.iter().find(|port| port.input && port.id == to.1))
        else {
            return HashMap::new();
        };
        let registry = self.registry_for(document);
        let target_definition = document
            .node(to.0)
            .and_then(|instance| registry.definition(&instance.definition));
        let target_type = input
            .ty
            .as_deref()
            .and_then(|ty| ty.parse::<TypeExpr>().ok());
        graph
            .nodes
            .iter()
            .filter(|node| node.id != to.0)
            .filter_map(|node| {
                let instance = document.node(node.id)?;
                let authority = port_authority::port_resolution(document, instance, &registry);
                if authority.missing.is_some()
                    || (instance.signature.is_some() && authority.contract.is_none())
                {
                    return None;
                }
                let source_definition = registry.definition(&instance.definition);
                let outputs: Vec<_> = node
                    .ports
                    .iter()
                    .filter(|port| !port.input && port.exec == input.exec)
                    .filter(|port| {
                        if port.exec {
                            return true;
                        }
                        let (Some(source), Some(target)) =
                            (port.ty.as_deref(), target_type.as_ref())
                        else {
                            return false;
                        };
                        if source.starts_with("core/stream@1<") && port.connected {
                            return false;
                        }
                        if source == target.as_str() {
                            return true;
                        }
                        if !source.contains('$') && !target.as_str().contains('$') {
                            return false;
                        }
                        let Ok(source) = source.parse::<TypeExpr>() else {
                            return false;
                        };
                        if source.as_str().contains('$') && source_definition.is_none()
                            || target.as_str().contains('$') && target_definition.is_none()
                        {
                            return false;
                        }
                        source_definition.is_none_or(|definition| {
                            PortFit::new(definition, &registry).meets(&source, target)
                        }) && target_definition.is_none_or(|definition| {
                            PortFit::new(definition, &registry).meets(target, &source)
                        })
                    })
                    .map(|port| port.id.clone())
                    .collect();
                (!outputs.is_empty()).then_some((node.id, outputs))
            })
            .collect()
    }

    /// The node kinds a wire from `from` (a node of the draft's top level and one of its
    /// outputs) can go into, with the input it would take, sorted by what they are
    /// called. Empty when the output is not found.
    pub fn takers(&self, draft: &Draft, from: (NodeId, &str)) -> Vec<Suggestion> {
        let Some(port) = self.output_view(draft.document(), from) else {
            return Vec::new();
        };
        let registry = self.registry_for(draft.document());
        let mut found: Vec<Suggestion> = self
            .nodes()
            .filter(|info| info.definition.kind != NodeKind::Entry)
            .filter_map(|info| {
                let mut fit = PortFit::new(info.definition, &registry);
                let input =
                    fitting_input(info.definition, port.exec, port.ty.as_deref(), &mut fit)?;
                Some(Suggestion {
                    definition: info.definition.definition.id.as_str().to_owned(),
                    label: info.label().into_owned(),
                    input,
                })
            })
            .collect();
        found.sort_by(|a, b| a.label.cmp(&b.label).then(a.definition.cmp(&b.definition)));
        found.dedup_by(|a, b| a.definition == b.definition);
        found
    }

    /// The node kinds that can feed input `to` (a node of the draft's top level and one
    /// of its inputs), with the output the wire would come from, sorted by what they are
    /// called: what a wire dragged back from an input lists. Triggers are offered for a
    /// step input. Empty when the input is not found.
    pub fn givers(&self, draft: &Draft, to: (NodeId, &str)) -> Vec<Giver> {
        let document = draft.document();
        let Some(target) = self.input_view(document, to) else {
            return Vec::new();
        };
        let registry = self.registry_for(document);
        let mut found: Vec<Giver> = self
            .nodes()
            .filter(|info| target.exec || info.definition.kind != NodeKind::Entry)
            .filter_map(|info| {
                let mut fit = PortFit::new(info.definition, &registry);
                let output =
                    continuation(info.definition, target.exec, target.ty.as_deref(), &mut fit)?;
                Some(Giver {
                    definition: info.definition.definition.id.as_str().to_owned(),
                    label: info.label().into_owned(),
                    output,
                })
            })
            .collect();
        found.sort_by(|a, b| a.label.cmp(&b.label).then(a.definition.cmp(&b.definition)));
        found.dedup_by(|a, b| a.definition == b.definition);
        found
    }

    /// The node kinds that can be put into the wire from `from` to `to`: each takes the
    /// wire's output on one input and goes on into the wire's input from one output,
    /// with the same parameter bindings on both, sorted by what they are called. Empty
    /// when either end is not found or the two ends are not one kind of wire.
    pub fn splice_takers(
        &self,
        draft: &Draft,
        from: (NodeId, &str),
        to: (NodeId, &str),
    ) -> Vec<SpliceSuggestion> {
        let document = draft.document();
        let Some(source) = self.output_view(document, from) else {
            return Vec::new();
        };
        let Some(target) = self.input_view(document, to) else {
            return Vec::new();
        };
        if source.exec != target.exec {
            return Vec::new();
        }
        let registry = self.registry_for(document);
        let mut found: Vec<SpliceSuggestion> = self
            .nodes()
            .filter(|info| info.definition.kind != NodeKind::Entry)
            .filter_map(|info| {
                let mut fit = PortFit::new(info.definition, &registry);
                let input =
                    fitting_input(info.definition, source.exec, source.ty.as_deref(), &mut fit)?;
                let output =
                    continuation(info.definition, target.exec, target.ty.as_deref(), &mut fit)?;
                Some(SpliceSuggestion {
                    definition: info.definition.definition.id.as_str().to_owned(),
                    label: info.label().into_owned(),
                    input,
                    output,
                })
            })
            .collect();
        found.sort_by(|a, b| a.label.cmp(&b.label).then(a.definition.cmp(&b.definition)));
        found.dedup_by(|a, b| a.definition == b.definition);
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inbound_drag_matches_actual_typed_outputs() {
        let (host, _) = Host::new(HostOptions::default()).unwrap();
        let mut draft = host.draft(&new_document().unwrap()).unwrap();
        let whole = host
            .add_node(&mut draft, "core/whole-number", [0.0, 0.0])
            .unwrap();
        let number = host
            .add_node(&mut draft, "core/number", [0.0, 0.0])
            .unwrap();
        let list = host
            .add_node(&mut draft, "org.orange.collections/build-list", [0.0, 0.0])
            .unwrap();
        let generic = host
            .add_node(&mut draft, "org.orange.math/absolute", [0.0, 0.0])
            .unwrap();
        let into = host
            .add_node(&mut draft, "org.orange.math/absolute", [0.0, 0.0])
            .unwrap();
        host.choose_type(&mut draft, into, "T", Some("core/i64@1"))
            .unwrap();
        let view = host.draft_authoring_view(&draft).unwrap();
        let matches = host.output_givers(&draft, &view.graph, (into, "value"));
        assert_eq!(matches[&whole], ["result"]);
        assert_eq!(matches[&generic], ["result"]);
        for node in [number, list, into] {
            assert!(!matches.contains_key(&node));
        }
    }
}
