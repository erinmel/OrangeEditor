//! Typed authoring inputs. The editor passes canonical values to the same codec
//! used to validate and run documents.
use super::*;
use orange_model::document::{PortDirection, PortKind};
use orange_model::type_term::TypeTerm;

/// The control a front end can offer for a value of the resolved type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiteralEditor {
    Plain,
    Option { some: bool },
    Structured,
    Unavailable,
}

/// Where an authoring input gets its value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputState {
    Wired { shadowed: bool },
    Literal,
    Default,
    ImplicitNone,
    Required,
}

/// Additional presentation and literal capabilities for one projected port.
/// Ordinary primitive rows can use the state/capability methods on PortView directly.
#[derive(Clone, Debug, PartialEq)]
pub struct PortDetails {
    pub id: String,
    pub input: bool,
    pub description: Option<String>,
    pub editor: LiteralEditor,
    pub optional: bool,
    pub stored_literal: bool,
    pub default_literal: Option<Literal>,
}

impl PortView {
    /// Input precedence is independent of wire style and the interface language.
    pub fn input_state(&self, details: Option<&PortDetails>) -> InputState {
        let literal = self.value.is_some() || details.is_some_and(|port| port.stored_literal);
        if self.connected {
            InputState::Wired { shadowed: literal }
        } else if literal {
            InputState::Literal
        } else if self.default.is_some()
            || details.is_some_and(|port| port.default_literal.is_some())
        {
            InputState::Default
        } else if details.is_some_and(|port| port.optional) {
            InputState::ImplicitNone
        } else {
            InputState::Required
        }
    }

    pub fn literal_editor(&self, details: Option<&PortDetails>) -> LiteralEditor {
        if !self.input || self.exec {
            return LiteralEditor::Unavailable;
        }
        details.map_or_else(
            || {
                if self.ty.as_deref().is_some_and(plain_literal) {
                    LiteralEditor::Plain
                } else {
                    LiteralEditor::Unavailable
                }
            },
            |port| port.editor,
        )
    }
}

impl Host {
    pub(crate) fn port_text(
        &self,
        port: &orange_model::document::PortDescriptor,
        field: &str,
        locale: &str,
    ) -> Option<String> {
        let text: TextRef = serde_json::from_value(port.extensions.get(field)?.clone()).ok()?;
        Some(self.text_in(&text, locale).into_owned())
    }
}

pub(crate) fn plain_literal(ty: &str) -> bool {
    matches!(
        ty,
        "core/string@1"
            | "core/bool@1"
            | "core/i32@1"
            | "core/u32@1"
            | "core/i64@1"
            | "core/u64@1"
            | "core/f32@1"
            | "core/f64@1"
            | "core/duration@1"
            | "core/timestamp@1"
            | RGBA_TYPE
    )
}

fn has_literal(term: &TypeTerm) -> bool {
    term.named().is_some_and(|named| {
        named.namespace != "core"
            || !matches!(
                named.name.as_str(),
                "handle" | "stream" | "result" | "error" | "secret-ref"
            )
    })
}

pub(crate) fn literal_editor(registry: &RegistrySnapshot, ty: &TypeExpr) -> LiteralEditor {
    if plain_literal(ty.as_str()) {
        return LiteralEditor::Plain;
    }
    let Some(term) = TypeTerm::parse(ty)
        .ok()
        .filter(|term| registry.types().literal_codec_identity(term).is_some())
    else {
        return LiteralEditor::Unavailable;
    };
    if let Some([inner]) = term.core("option") {
        return LiteralEditor::Option {
            some: has_literal(inner),
        };
    }
    if has_literal(&term) {
        LiteralEditor::Structured
    } else {
        LiteralEditor::Unavailable
    }
}

impl Host {
    /// Writes exact typed input values together in one undoable edit. Wired
    /// inputs retain their saved values. Each new literal must match the fresh
    /// resolved input type and pass the canonical codec before anything changes.
    pub fn set_input_literals<'a>(
        &self,
        draft: &mut Draft,
        values: impl IntoIterator<Item = ((NodeId, &'a str), Literal)>,
    ) -> Result<bool, HostError> {
        let document = draft.document();
        let registry = self.registry_for(document);
        let wired: std::collections::HashSet<_> = document
            .root()
            .edges
            .values()
            .map(|edge| &edge.to)
            .collect();
        let mut commands = Vec::new();
        for ((id, key), literal) in values {
            let port = edit::port_ref((id, key))?;
            if wired.contains(&port) {
                return Err(HostError::Document("the input is connected".into()));
            }
            let node =
                document.root().nodes.get(&id).ok_or_else(|| {
                    HostError::Document("the node for the input is absent".into())
                })?;
            let contract = port_authority::fresh_contract(document, node, &registry)
                .ok_or_else(|| HostError::Document("the input contract is unavailable".into()))?;
            let declared = contract.ports.iter().find(|entry| {
                entry.direction == PortDirection::Input
                    && entry.kind == PortKind::Data
                    && &entry.port_id == port.port.port()
                    && match port.port.instance() {
                        Some(index) => {
                            entry.variadic.is_some()
                                && node
                                    .variadic
                                    .get(&entry.port_id)
                                    .is_some_and(|state| state.order.contains(&index))
                        }
                        None => entry.variadic.is_none(),
                    }
            });
            let Some(ty) = declared.and_then(|entry| entry.ty.as_ref()) else {
                return Err(HostError::Document(
                    "the node has no such data input".into(),
                ));
            };
            let mut arguments = node.type_args.clone();
            arguments.extend(node.type_pins.clone());
            let ty = orange_model::signature::substitute_params(ty, &arguments);
            if ty != literal.ty {
                return Err(HostError::Variable(
                    "the literal must match the input type".into(),
                ));
            }
            let _ = orange_model::value::literal_codec::decode(&literal, registry.types().as_ref())
                .map_err(|error| HostError::Variable(error.to_string()))?;
            commands.push(orange_model::DocCommand::SetLiteral {
                port,
                literal: Some(literal),
            });
        }
        draft.commit(&registry, commands)
    }
}
