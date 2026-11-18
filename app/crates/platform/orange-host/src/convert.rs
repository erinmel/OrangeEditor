//! Conversions between types (11 §6): the ones a person can add from search, and wires
//! that go through one when the two ends differ. The language never converts on its own;
//! every conversion is a visible `core/convert` node.
use crate::edit::{PortEnd, port_ref};
use crate::{Draft, Host, HostError, NodeId};
use orange_model::conversion::{
    ConnectionError, ConnectionOutcome, ConnectionRequest, ConversionClass, ConverterKey,
    ConverterProvider, ConverterRegistry, ExplicitConverterChoice, NamedPolicy, SelectionRejection,
    converter_node_commands, plan_connection,
};
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
        let exec = self
            .node_view(draft.document(), from.0)
            .and_then(|node| {
                node.ports
                    .into_iter()
                    .find(|port| !port.input && port.id == from.1)
            })
            .is_some_and(|port| port.exec);
        if exec {
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

    /// Wiring between two number kinds offers the conversions that join them, and the
    /// chosen one goes in between with its two wires. Above 2^53 a whole number does not
    /// fit a decimal exactly, so even that conversion is chosen, never put in silently.
    #[test]
    fn a_wire_between_two_number_kinds_goes_through_a_conversion() {
        let host = host();
        let mut draft = host.draft(&crate::new_document().unwrap()).unwrap();
        let whole = host
            .add_node(&mut draft, "core/whole-number", [0.0, 0.0])
            .unwrap();
        let decimal = host
            .add_node(&mut draft, "core/number", [0.0, 0.0])
            .unwrap();
        let negate_decimal = host
            .add_node(&mut draft, "org.orange.math/float-negate", [300.0, 0.0])
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
