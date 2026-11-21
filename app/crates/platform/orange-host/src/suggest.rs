//! Which node kinds can take a wire from an output: what an editor lists when a wire is
//! dropped on empty canvas, so the next node is chosen from the ones that fit.
//!
//! The answer comes from each kind's declared ports, before a node exists: a step output
//! fits any kind with a way in, and a value output fits a value input of the same type or
//! of a type the kind leaves open (`$T`). It is a shortlist, not a promise: connecting
//! still runs the language's checks and refuses what does not fit with the reason.
use super::*;
use orange_model::catalog::{Ports, VariableAccess, implicit_port_names};
use orange_model::document::{NodeKind, PortDirection, PortKind};

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

/// The input of a kind a wire fits, if any: `exec` for a step, else a value of type `ty`.
fn fitting_input(definition: &NodeDefinition, exec: bool, ty: Option<&str>) -> Option<String> {
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
            let wanted = ty?;
            inputs
                .filter(|port| port.kind == PortKind::Data)
                .find(|port| {
                    port.ty.as_ref().is_some_and(|declared| {
                        let declared = declared.to_string();
                        declared == wanted || declared.contains('$')
                    })
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
                super::edit::collection_mode(ty?).map(|_| "items".to_owned())
            }
        }
        Ports::Event(_) => (exec && definition.kind != NodeKind::Entry).then(|| "in".to_owned()),
        // Triggers take no wire; record and enum nodes are reached through their fields.
        Ports::Manual | Ports::Timer | Ports::Nominal(_) => None,
    }
}

impl Host {
    /// The node kinds a wire from `from` (a node of the draft's top level and one of its
    /// outputs) can go into, with the input it would take, sorted by what they are
    /// called. Empty when the output is not found.
    pub fn takers(&self, draft: &Draft, from: (NodeId, &str)) -> Vec<Suggestion> {
        let Some(port) = self.node_view(draft.document(), from.0).and_then(|node| {
            node.ports
                .into_iter()
                .find(|port| !port.input && port.id == from.1)
        }) else {
            return Vec::new();
        };
        let mut found: Vec<Suggestion> = self
            .nodes()
            .filter(|info| info.definition.kind != NodeKind::Entry)
            .filter_map(|info| {
                let input = fitting_input(info.definition, port.exec, port.ty.as_deref())?;
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
}
