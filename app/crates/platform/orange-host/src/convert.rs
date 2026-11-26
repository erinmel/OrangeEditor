//! Conversions between types (11 §6): the ones a person can add from search, and wires
//! that go through one when the two ends differ. The language never converts on its own;
//! every conversion is a visible `core/convert` node.
use crate::edit::{PortEnd, port_ref};
#[cfg(test)]
use crate::intrinsic;
use crate::{Draft, EdgeId, Host, HostError, NodeId, NodeView, Severity, TypeExpr};
use orange_model::conversion::{
    ConnectionError, ConnectionOutcome, ConnectionRequest, ConversionClass, ConverterKey,
    ConverterProvider, ConverterRegistry, ExplicitConverterChoice, NamedPolicy, SelectionRejection,
    converter_node_commands, plan_connection,
};
use orange_model::document::{Edge, PortRef};
use orange_model::fit::PortFit;
use orange_model::{DocCommand, GraphDocument};
use std::collections::BTreeMap;

/// One conversion a person can choose: a converter with every policy choice fixed, such
/// as decimal to whole number, rounding down.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conversion {
    /// The type it takes and the type it gives, such as `core/f64@1`.
    pub from: String,
    pub to: String,
    /// Its policy choices, by policy: `rounding` → `floor`.
    pub options: BTreeMap<String, String>,
    /// It may lose information (a decimal's fraction) and says what it lost.
    pub lossy: bool,
    /// It may fail at run time (text that is not a number).
    pub checked: bool,
    entry: ConverterKey,
    provider: ConverterProvider,
}

impl Conversion {
    /// The converter's ID, such as `core/f64-to-i64`.
    pub fn id(&self) -> &str {
        self.entry.id.as_str()
    }

    fn choice(&self) -> ExplicitConverterChoice {
        ExplicitConverterChoice {
            entry: self.entry.clone(),
            provider: self.provider.clone(),
            options: self.options.clone(),
        }
    }
}

/// What connecting two value ports did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Connected {
    /// The ports were joined directly.
    Wired,
    /// A conversion was put between them, the only one that loses nothing.
    Converted(NodeId),
    /// Several conversions could join them, or only ones that may lose or fail: the
    /// person chooses one, and connects again with it.
    Choose(Vec<Conversion>),
}

/// Every way to fix a converter's policies, each policy's default first: decimal to
/// whole number is offered rounding down, up, to nearest and toward zero.
fn option_sets(policies: &[NamedPolicy]) -> Vec<BTreeMap<String, String>> {
    let mut sets = vec![BTreeMap::new()];
    for policy in policies {
        let choices: Vec<&String> = policy
            .default
            .iter()
            .chain(
                policy
                    .choices
                    .iter()
                    .filter(|choice| Some(*choice) != policy.default.as_ref()),
            )
            .collect();
        sets = sets
            .into_iter()
            .flat_map(|set| {
                choices.iter().map(move |choice| {
                    let mut set = set.clone();
                    let _ = set.insert(policy.name.clone(), (*choice).clone());
                    set
                })
            })
            .collect();
    }
    sets
}

fn conversions_of(
    converters: &ConverterRegistry,
    key: &ConverterKey,
    provider: Option<&ConverterProvider>,
) -> Vec<Conversion> {
    let Some(definition) = converters.entry(key) else {
        return Vec::new();
    };
    let Some(provider) = provider
        .or_else(|| converters.new_binding_default(key))
        .or_else(|| converters.providers(key)?.next())
    else {
        return Vec::new();
    };
    option_sets(&definition.policies)
        .into_iter()
        .map(|options| Conversion {
            from: definition.from.to_string(),
            to: definition.to.to_string(),
            options,
            lossy: definition.class == ConversionClass::Lossy,
            checked: definition.class == ConversionClass::CheckedPartial,
            entry: key.clone(),
            provider: provider.clone(),
        })
        .collect()
}

impl Host {
    /// Finds a unique top-level wire to retain as the splice target.
    pub fn splice_edge(draft: &Draft, from: PortEnd<'_>, to: PortEnd<'_>) -> Option<EdgeId> {
        let (from, to) = (port_ref(from).ok()?, port_ref(to).ok()?);
        let mut edges = draft
            .document()
            .root()
            .edges
            .values()
            .filter(|edge| edge.from == from && edge.to == to);
        match (edges.next(), edges.next()) {
            (Some(edge), None) => Some(edge.id),
            _ => None,
        }
    }

    /// Explicit converters that can take the source and feed the destination after
    /// the original wire stops binding the destination's generic parameters.
    pub fn splice_conversions(&self, draft: &Draft, wire: EdgeId) -> Vec<Conversion> {
        let Ok(edge) = self.data_splice_edge(draft, wire) else {
            return Vec::new();
        };
        let registry = self.registry_for(draft.document());
        let mut detached = Draft::new(draft.document().clone());
        if detached
            .commit(
                &registry,
                [
                    DocCommand::Disconnect { edge: wire },
                    DocCommand::SetLiteral {
                        port: edge.to.clone(),
                        literal: None,
                    },
                ],
            )
            .is_err()
        {
            return Vec::new();
        }
        let document = detached.document();
        let from = edge.from.port.to_string();
        let Some(source) = self.output_view(document, (edge.from.node, &from)) else {
            return Vec::new();
        };
        let Some(target) = self.wire_node_view(document, edge.to.node) else {
            return Vec::new();
        };
        let key = edge.to.port.to_string();
        let target = target
            .ports
            .iter()
            .find(|port| port.input && port.id == key);
        let (Some(source), Some(target)) = (
            source.ty.as_deref(),
            target.and_then(|port| port.ty.as_deref()),
        ) else {
            return Vec::new();
        };
        let fit = document
            .node(edge.to.node)
            .and_then(|instance| registry.definition(&instance.definition))
            .map(|definition| PortFit::new(definition, &registry));
        let parsed_target: Option<TypeExpr> = target.parse().ok();
        let converters = registry.converters();
        converters
            .entries()
            .filter(|converter| {
                converter.from.as_str() == source
                    && converter.to.as_str() != source
                    && (converter.to.as_str() == target
                        || fit
                            .clone()
                            .zip(parsed_target.as_ref())
                            .is_some_and(|(mut fit, target)| fit.meets(target, &converter.to)))
            })
            .flat_map(|converter| conversions_of(converters, &converter.key(), None))
            .collect()
    }

    /// Inserts the selected converter into precisely this wire as one edit.
    pub fn splice_conversion(
        &self,
        draft: &mut Draft,
        wire: EdgeId,
        conversion: &Conversion,
        at: [f32; 2],
    ) -> Result<NodeId, HostError> {
        let edge = self.data_splice_edge(draft, wire)?;
        let registry = self.registry_for(draft.document());
        let node = draft
            .fresh_id("conversion splice")
            .parse()
            .map_err(|_| HostError::Document("the new ID is not valid".into()))?;
        let mut commands = vec![DocCommand::Disconnect { edge: wire }];
        commands.extend(
            converter_node_commands(&registry, conversion.choice(), node)
                .map_err(|rejection| HostError::Document(rejection_text(&rejection)))?,
        );
        commands.push(crate::edit::place_command(node, Some(at)));
        let into = port_ref((node, "value"))?;
        let out = port_ref((node, "result"))?;
        for (tag, from, to) in [
            ("conversion splice input", edge.from, into),
            ("conversion splice output", out, edge.to),
        ] {
            let id = draft
                .fresh_id(tag)
                .parse()
                .map_err(|_| HostError::Document("the new ID is not valid".into()))?;
            commands.push(DocCommand::Connect {
                edge: Edge {
                    id,
                    from,
                    to,
                    extensions: Default::default(),
                },
            });
        }
        let mut staged = Draft::new(draft.document().clone());
        let _ = staged.commit(&registry, commands.iter().cloned())?;
        let before = self
            .problems(draft)?
            .into_iter()
            .map(|diagnostic| (diagnostic.code, diagnostic.subject))
            .collect::<std::collections::BTreeSet<_>>();
        let problems: Vec<_> = self
            .problems(&mut staged)?
            .into_iter()
            .filter(|diagnostic| diagnostic.severity == Severity::Error)
            .filter(|diagnostic| !before.contains(&(diagnostic.code, diagnostic.subject.clone())))
            .collect();
        if !problems.is_empty() {
            return Err(HostError::Invalid(problems));
        }
        let _ = draft.commit(&registry, commands)?;
        Ok(node)
    }

    fn data_splice_edge(&self, draft: &Draft, wire: EdgeId) -> Result<Edge, HostError> {
        let document = draft.document();
        let edge = document
            .root()
            .edges
            .get(&wire)
            .filter(|edge| unique_input(document, &edge.to))
            .ok_or_else(|| HostError::Document("the wire to insert into has changed".into()))?;
        self.require_port_authority(document, edge.from.node)?;
        self.require_port_authority(document, edge.to.node)?;
        let source = self.output_view(document, (edge.from.node, &edge.from.port.to_string()));
        if !source.is_some_and(|port| !port.exec && port.ty.is_some()) {
            return Err(HostError::Document("the wire has no data output".into()));
        }
        Ok(edge.clone())
    }

    /// Every conversion this host can add, one for each choice of its policies, in
    /// converter order.
    pub fn conversions(&self) -> Vec<Conversion> {
        let registry = self.store.current();
        let converters = registry.converters();
        converters
            .entries()
            .flat_map(|definition| conversions_of(converters, &definition.key(), None))
            .collect()
    }

    /// Adds `conversion` as a node on its own at `at`, to be wired after.
    pub fn add_conversion(
        &self,
        draft: &mut Draft,
        conversion: &Conversion,
        at: [f32; 2],
    ) -> Result<NodeId, HostError> {
        let node: NodeId = draft
            .fresh_id("conversion")
            .parse()
            .map_err(|_| HostError::Document("the new ID is not valid".into()))?;
        let registry = self.registry_for(draft.document());
        let mut commands = converter_node_commands(&registry, conversion.choice(), node)
            .map_err(|rejection| HostError::Document(rejection_text(&rejection)))?;
        commands.push(crate::edit::place_command(node, Some(at)));
        let _ = draft.commit(&registry, commands)?;
        Ok(node)
    }

    /// Returns the card for an explicit converter choice without editing the draft.
    pub fn conversion_preview(
        &self,
        draft: &Draft,
        conversion: &Conversion,
    ) -> Result<NodeView, HostError> {
        let node = draft
            .fresh_id("conversion")
            .parse()
            .map_err(|_| HostError::Document("the new ID is not valid".into()))?;
        let registry = self.registry_for(draft.document());
        let commands = converter_node_commands(&registry, conversion.choice(), node)
            .map_err(|rejection| HostError::Document(rejection_text(&rejection)))?;
        self.preview_commands(draft, node, commands)
    }

    /// Connects output `from` to input `to`, through a conversion when their types
    /// differ (11 §6 rule 3): the one that loses nothing is put in by itself; otherwise
    /// the choices come back and `through` names the one chosen. Steps connect as
    /// [`Host::connect`] does.
    pub fn connect_through(
        &self,
        draft: &mut Draft,
        from: PortEnd<'_>,
        to: PortEnd<'_>,
        through: Option<&Conversion>,
    ) -> Result<Connected, HostError> {
        self.require_port_authority(draft.document(), from.0)?;
        self.require_port_authority(draft.document(), to.0)?;
        let exec = self
            .output_view(draft.document(), from)
            .is_some_and(|port| port.exec);
        if exec {
            return self.connect(draft, from, to).map(|_| Connected::Wired);
        }
        if through.is_none() && self.for_each_wire_mode(draft, from, to).is_some() {
            return self.connect(draft, from, to).map(|_| Connected::Wired);
        }
        let registry = self.registry_for(draft.document());
        let request = ConnectionRequest {
            from: port_ref(from)?,
            to: port_ref(to)?,
            choice: through.map(Conversion::choice),
        };
        let mut made = 0u32;
        let ids: Vec<uuid::Uuid> = (0..4)
            .map(|_| {
                made += 1;
                draft
                    .fresh_id(&format!("wire-{made}"))
                    .parse()
                    .map_err(|_| HostError::Document("the new ID is not valid".into()))
            })
            .collect::<Result<_, _>>()?;
        let mut next = ids.into_iter();
        let outcome = plan_connection(draft.document(), &registry, request, || {
            // A plan makes at most a node and two wires; a fourth ID is never asked for.
            next.next().unwrap_or(uuid::Uuid::nil())
        })
        .map_err(|error| HostError::Document(connection_text(&error)))?;
        match outcome {
            ConnectionOutcome::Ready(plan) => {
                let converter = plan.converter.as_ref().map(|inserted| inserted.node);
                let _ = draft.commit(&registry, plan.commands().iter().cloned())?;
                Ok(converter.map_or(Connected::Wired, Connected::Converted))
            }
            ConnectionOutcome::ChoiceRequired { candidates, .. } => {
                let converters = registry.converters();
                Ok(Connected::Choose(
                    candidates
                        .iter()
                        .flat_map(|candidate| {
                            conversions_of(converters, &candidate.entry, Some(&candidate.provider))
                        })
                        .collect(),
                ))
            }
            ConnectionOutcome::Rejected(rejection) => {
                Err(HostError::Document(rejection_text(&rejection)))
            }
        }
    }
}

fn unique_input(document: &GraphDocument, port: &PortRef) -> bool {
    document
        .root()
        .edges
        .values()
        .filter(|edge| edge.to == *port)
        .take(2)
        .count()
        == 1
}

fn rejection_text(rejection: &SelectionRejection) -> String {
    match rejection {
        SelectionRejection::NoDirectConverter { .. } => {
            "no conversion joins these two kinds of value".into()
        }
        other => format!("the conversion cannot be used: {other}"),
    }
}

fn connection_text(error: &ConnectionError) -> String {
    match error {
        ConnectionError::TypeConflict(_) => {
            "these two kinds of value do not fit together, and no conversion joins them".into()
        }
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> Host {
        Host::new(crate::HostOptions::default()).unwrap().0
    }

    #[test]
    fn an_explicit_splice_rebinds_a_generic_target_and_undo_restores_the_exact_wire() {
        let host = host();
        let mut draft = host.draft(&crate::new_document().unwrap()).unwrap();
        let source = host
            .add_node(&mut draft, intrinsic::WHOLE_NUMBER, [0.0, 0.0])
            .unwrap();
        let target = host
            .add_node(&mut draft, "org.orange.math/absolute", [300.0, 0.0])
            .unwrap();
        let other = host
            .add_node(&mut draft, "org.orange.math/absolute", [300.0, 200.0])
            .unwrap();
        let from = (source, "result");
        let into = (target, "value");
        host.connect(&mut draft, from, into).unwrap();
        host.set_input(&mut draft, into, Some("7")).unwrap();
        host.connect(&mut draft, from, (other, "value")).unwrap();
        let old = Host::splice_edge(&draft, from, into).unwrap();
        let sibling = Host::splice_edge(&draft, from, (other, "value")).unwrap();
        assert!(
            draft
                .set_wire_waypoint(from, into, Some([126.0, 75.0]))
                .unwrap()
        );
        let original = draft.document().edge(old).unwrap().clone();
        let choice = host
            .splice_conversions(&draft, old)
            .into_iter()
            .find(|choice| choice.to == "core/f64@1")
            .expect("the generic target takes decimals");
        let node = host
            .splice_conversion(&mut draft, old, &choice, [150.0, 0.0])
            .unwrap();
        assert!(draft.document().edge(old).is_none());
        assert!(draft.document().edge(sibling).is_some());
        assert!(Host::splice_edge(&draft, from, (node, "value")).is_some());
        assert!(Host::splice_edge(&draft, (node, "result"), into).is_some());
        assert_eq!(
            host.wire_node_view(draft.document(), target)
                .unwrap()
                .ports
                .iter()
                .find(|port| port.input && port.id == "value")
                .and_then(|port| port.ty.as_deref()),
            Some("core/f64@1")
        );
        let after = crate::semantic(&draft.text().unwrap()).unwrap();
        assert!(draft.undo());
        assert_eq!(draft.document().edge(old), Some(&original));
        assert_eq!(draft.wire_waypoints().next().unwrap().1, [126.0, 75.0]);
        assert!(draft.document().edge(sibling).is_some());
        assert!(draft.document().node(node).is_none());
        assert_eq!(draft.document().node(target).unwrap().inputs.len(), 1);
        assert!(draft.redo());
        assert_eq!(crate::semantic(&draft.text().unwrap()).unwrap(), after);
    }

    #[test]
    fn a_refused_splice_keeps_the_document_and_redo_history() {
        let host = host();
        let mut draft = host.draft(&crate::new_document().unwrap()).unwrap();
        let source = host
            .add_node(&mut draft, intrinsic::WHOLE_NUMBER, [0.0, 0.0])
            .unwrap();
        let target = host
            .add_node(&mut draft, "org.orange.math/absolute", [300.0, 0.0])
            .unwrap();
        host.choose_type(&mut draft, target, "T", Some("core/i64@1"))
            .unwrap();
        let from = (source, "result");
        let into = (target, "value");
        host.connect(&mut draft, from, into).unwrap();
        let old = Host::splice_edge(&draft, from, into).unwrap();
        let _ = host
            .add_node(&mut draft, intrinsic::NUMBER, [0.0, 200.0])
            .unwrap();
        assert!(draft.undo());
        let before = draft.text().unwrap();
        let choice = host
            .conversions()
            .into_iter()
            .find(|choice| choice.from == "core/i64@1" && choice.to == "core/f64@1")
            .expect("whole-to-decimal converter");
        assert!(host.splice_conversions(&draft, old).is_empty());
        assert!(
            host.splice_conversion(&mut draft, old, &choice, [150.0, 0.0])
                .is_err()
        );
        assert_eq!(draft.text().unwrap(), before);
        assert_eq!(draft.can_undo(), (true, true));
        assert!(draft.redo());
    }

    /// Wiring between two number kinds offers the conversions that join them, and the
    /// chosen one goes in between with its two wires. Above 2^53 a whole number does not
    /// fit a decimal exactly, so even that conversion is chosen, never put in silently.
    #[test]
    fn a_wire_between_two_number_kinds_goes_through_a_conversion() {
        let host = host();
        let mut draft = host.draft(&crate::new_document().unwrap()).unwrap();
        let whole = host
            .add_node(&mut draft, intrinsic::WHOLE_NUMBER, [0.0, 0.0])
            .unwrap();
        let decimal = host
            .add_node(&mut draft, intrinsic::NUMBER, [0.0, 0.0])
            .unwrap();
        let negate_decimal = host
            .add_node(&mut draft, "org.orange.math/negate", [300.0, 0.0])
            .unwrap();
        host.choose_type(&mut draft, negate_decimal, "T", Some("core/f64@1"))
            .unwrap();
        let Connected::Choose(choices) = host
            .connect_through(
                &mut draft,
                (whole, "result"),
                (negate_decimal, "value"),
                None,
            )
            .unwrap()
        else {
            panic!("a whole number into a decimal is a choice");
        };
        assert!(
            choices
                .iter()
                .all(|choice| choice.lossy && choice.to == "core/f64@1")
        );
        let Connected::Converted(node) = host
            .connect_through(
                &mut draft,
                (whole, "result"),
                (negate_decimal, "value"),
                choices.first(),
            )
            .unwrap()
        else {
            panic!("the chosen conversion goes in");
        };
        let view = host.draft_view(&draft).unwrap();
        assert!(view.nodes.iter().any(|shown| shown.id == node));
        assert_eq!(view.wires.len(), 2);
        assert!(host.problems(&mut draft).unwrap().is_empty());

        let negate_whole = host
            .add_node(&mut draft, "org.orange.math/negate", [300.0, 200.0])
            .unwrap();
        host.choose_type(&mut draft, negate_whole, "T", Some("core/i64@1"))
            .unwrap();
        let Connected::Choose(choices) = host
            .connect_through(
                &mut draft,
                (decimal, "result"),
                (negate_whole, "value"),
                None,
            )
            .unwrap()
        else {
            panic!("a decimal into a whole number needs a choice");
        };
        assert!(choices.iter().all(|choice| choice.from == "core/f64@1"));
        let chosen = choices.first().unwrap().clone();
        assert!(matches!(
            host.connect_through(
                &mut draft,
                (decimal, "result"),
                (negate_whole, "value"),
                Some(&chosen)
            )
            .unwrap(),
            Connected::Converted(_)
        ));
    }

    /// Search offers each conversion with its policies fixed, and one added on its own
    /// is a node with a value in and a result out.
    #[test]
    fn conversions_are_listed_and_added_on_their_own() {
        let host = host();
        let conversions = host.conversions();
        assert!(
            conversions
                .iter()
                .any(|c| c.from == "core/i64@1" && c.to == "core/f64@1")
        );
        let rounding: Vec<_> = conversions
            .iter()
            .filter(|c| c.from == "core/f64@1" && c.to == "core/i64@1")
            .collect();
        assert!(rounding.len() > 1, "{rounding:?}");
        let mut draft = host.draft(&crate::new_document().unwrap()).unwrap();
        let node = host
            .add_conversion(&mut draft, rounding[0], [10.0, 20.0])
            .unwrap();
        let view = host.draft_view(&draft).unwrap();
        let shown = view.nodes.iter().find(|shown| shown.id == node).unwrap();
        let ports: Vec<_> = shown.ports.iter().map(|port| port.id.as_str()).collect();
        assert!(
            ports.contains(&"value") && ports.contains(&"result"),
            "{ports:?}"
        );
    }

    /// A package that declares its own types, as audio does, leaves every core
    /// conversion in place.
    #[test]
    fn packages_with_their_own_types_keep_the_core_conversions() {
        let bare = host().conversions();
        let typed = orange_sdk::ServicePackage::new("org.example.typed", "1.0.0")
            .enumeration("mood", &[("calm", None), ("loud", None)]);
        let (with_audio, _) =
            Host::with_services(crate::HostOptions::default(), vec![typed]).unwrap();
        assert!(!bare.is_empty());
        assert_eq!(with_audio.conversions(), bare);
    }
}
