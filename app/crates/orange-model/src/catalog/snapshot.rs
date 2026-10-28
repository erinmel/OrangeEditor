//! The immutable registry snapshot and its builder.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use thiserror::Error;

use super::intrinsics::intrinsic_definitions;
use super::language::{IMPLICIT_PORTS, intrinsic_reference, is_converter_special_form};
use super::{
    ContractError, EventAccess, NodeContract, NodeDefinition, Ports, ProviderDescriptor,
    ProviderKey, ReferenceError, ReferenceField, ReferenceKind, TypeArgs, VariableAccess,
};
use crate::config_pointer::pointer_parts;
use crate::conversion::{
    ConversionClass, ConverterConfig, ConverterProvider, ConverterRegistry,
    ConverterRegistryBuilder, ExplicitConverterChoice,
};
use crate::document::DefinitionRef;
use crate::document::{
    GraphDocument, NodeInstance, NodeKind, OutputSource, PortDescriptor, PortDirection, PortKind,
    Suspends, SuspensionContractIssue, VarScope, suspension_contract_issue,
};
use crate::id::DeclId;
use crate::id::{DefinitionId, PortId};
use crate::origin::HostOrigin;
use crate::type_term::TypeTerm;
use crate::types::{
    CapabilityError, CapabilityRequest, Constraint, NumericOperation, TypeRegistry,
    TypeResolveError,
};
use crate::variable::WriteOperation;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RegistryError {
    #[error("definition `{definition}` has incompatible volatility metadata")]
    VolatilityKind { definition: DefinitionId },
    #[error("package definition `{definition}` may declare only entropy volatility")]
    VolatilityAuthority { definition: DefinitionId },
    #[error("definition `{definition}` has incompatible determinism metadata")]
    DeterminismKind { definition: DefinitionId },
    #[error("state/clock definition `{definition}` cannot declare data input `{port}`")]
    VolatileDataInput {
        definition: DefinitionId,
        port: PortId,
    },
    #[error("state/clock definition `{definition}` has an input-bearing port template")]
    VolatilePortTemplate { definition: DefinitionId },
    #[error("duplicate definition `{}@{}`", .0.id, .0.version)]
    DuplicateDefinition(DefinitionRef),
    #[error("duplicate provider `{}/{}`", .0.package, .0.id)]
    DuplicateProvider(ProviderKey),
    #[error("package definition `{0}` uses the reserved core namespace")]
    CoreNamespace(DefinitionId),
    #[error("definition `{definition}` declares port `{port}` twice")]
    DuplicatePort {
        definition: DefinitionId,
        port: PortId,
    },
    #[error("definition `{definition}` declares reserved implicit port `{port}`")]
    ReservedPort {
        definition: DefinitionId,
        port: PortId,
    },
    #[error(
        "definition `{definition}` port `{port}` must have a type exactly when it is a data or event port"
    )]
    PortType {
        definition: DefinitionId,
        port: PortId,
    },
    #[error("pure definition `{definition}` cannot declare execution port `{port}`")]
    PureExecutionPort {
        definition: DefinitionId,
        port: PortId,
    },
    #[error("volatile definition `{definition}` cannot declare execution port `{port}`")]
    VolatileExecutionPort {
        definition: DefinitionId,
        port: PortId,
    },
    #[error("definition `{definition}` event port `{port}` must be an output")]
    EventInputPort {
        definition: DefinitionId,
        port: PortId,
    },
    #[error(
        "definition `{definition}` port `{port}` may be lazy only when registered as a host intrinsic"
    )]
    LazyPackagePort {
        definition: DefinitionId,
        port: PortId,
    },
    #[error("definition `{definition}` data output port `{port}` must declare a source")]
    MissingDataOutputSource {
        definition: DefinitionId,
        port: PortId,
    },
    #[error(
        "definition `{definition}` port `{port}` has invalid variadic bounds: min {min} exceeds max {max}"
    )]
    InvalidVariadicBounds {
        definition: DefinitionId,
        port: PortId,
        min: u16,
        max: u16,
    },
    #[error("definition `{definition}` port `{port}` may have a default only as a data input")]
    DefaultPortPosition {
        definition: DefinitionId,
        port: PortId,
    },
    #[error(
        "definition `{definition}` port `{port}` default type does not match its declared type"
    )]
    DefaultType {
        definition: DefinitionId,
        port: PortId,
    },
    #[error("definition `{definition}` port `{port}` default is not a valid literal for its type")]
    DefaultLiteral {
        definition: DefinitionId,
        port: PortId,
    },
    #[error(
        "definition `{definition}` port `{port}` may declare availability only as an execution-sourced data output"
    )]
    AvailableOnPosition {
        definition: DefinitionId,
        port: PortId,
    },
    #[error("definition `{definition}` port `{port}` must name at least one available outcome")]
    AvailableOnEmpty {
        definition: DefinitionId,
        port: PortId,
    },
    #[error("definition `{definition}` port `{port}` names available outcome `{outcome}` twice")]
    AvailableOnDuplicate {
        definition: DefinitionId,
        port: PortId,
        outcome: PortId,
    },
    #[error(
        "definition `{definition}` port `{port}` names unavailable execution outcome `{outcome}`"
    )]
    AvailableOnOutcome {
        definition: DefinitionId,
        port: PortId,
        outcome: PortId,
    },
    #[error("definition `{definition}` port `{port}` may be optional only as a data input")]
    OptionalPortPosition {
        definition: DefinitionId,
        port: PortId,
    },
    #[error(
        "definition `{definition}` port `{port}` marked optional must have type core/option@1<T>"
    )]
    OptionalType {
        definition: DefinitionId,
        port: PortId,
    },
    #[error("definition `{definition}` of kind {kind:?} is missing suspension metadata")]
    SuspendsMissing {
        definition: DefinitionId,
        kind: NodeKind,
    },
    #[error(
        "definition `{definition}` of kind {kind:?} has invalid suspension metadata {suspends:?}"
    )]
    SuspendsInvalid {
        definition: DefinitionId,
        kind: NodeKind,
        suspends: Suspends,
    },
    #[error("definition `{definition}` port `{port}` uses undeclared type parameter `${param}`")]
    UndeclaredTypeParam {
        definition: DefinitionId,
        port: PortId,
        param: String,
    },
    #[error("definition `{definition}` declares invalid reference pointer `{pointer}`")]
    InvalidReferencePointer {
        definition: DefinitionId,
        pointer: String,
    },
    #[error("definition `{definition}` declares reference pointer `{pointer}` twice")]
    DuplicateReference {
        definition: DefinitionId,
        pointer: String,
    },
    #[error(
        "definition `{definition}` takes its ports from a variable but declares no variable reference"
    )]
    VariablePortsWithoutReference { definition: DefinitionId },
    #[error("variable update definition `{definition}` must have kind Action, found {kind:?}")]
    VariableUpdateKind {
        definition: DefinitionId,
        kind: NodeKind,
    },
    #[error("shared variable definition `{definition}` must have kind Async, found {kind:?}")]
    SharedVariableKind {
        definition: DefinitionId,
        kind: NodeKind,
    },
    #[error(
        "definition `{definition}` takes its ports from an event but declares no event reference"
    )]
    EventPortsWithoutReference { definition: DefinitionId },
    #[error("event definition `{definition}` must have kind {expected:?}, found {kind:?}")]
    EventKind {
        definition: DefinitionId,
        expected: NodeKind,
        kind: NodeKind,
    },
    #[error("provider `{}/{}` names unknown definition `{}@{}`", .provider.package, .provider.id, .definition.id, .definition.version)]
    UnknownDefinition {
        provider: Box<ProviderKey>,
        definition: DefinitionRef,
    },
}

/// Collects host metadata, then freezes it into a [`RegistrySnapshot`].
#[derive(Debug)]
pub struct RegistryBuilder {
    origin: HostOrigin,
    types: Arc<TypeRegistry>,
    converters: Arc<ConverterRegistry>,
    /// Definitions with their intrinsic flag, in registration order.
    definitions: Vec<(NodeDefinition, bool)>,
    providers: Vec<ProviderDescriptor>,
}

impl RegistryBuilder {
    /// Starts with the core type registry and core conversions.
    pub fn core(origin: HostOrigin) -> Self {
        let types = Arc::new(TypeRegistry::core());
        let converters = ConverterRegistryBuilder::with_core_conversions(Arc::clone(&types))
            .expect("the built-in core conversions are valid")
            .build();
        let builder = Self {
            origin,
            types,
            converters,
            definitions: Vec::new(),
            providers: Vec::new(),
        };
        intrinsic_definitions()
            .into_iter()
            .fold(builder, Self::intrinsic)
    }

    /// Replaces the type registry; converters restart empty over it.
    pub fn types(mut self, types: Arc<TypeRegistry>) -> Self {
        self.converters = ConverterRegistryBuilder::new(Arc::clone(&types)).build();
        self.types = types;
        self
    }

    /// Replaces the converters and adopts the type registry they are bound to.
    pub fn converters(mut self, converters: Arc<ConverterRegistry>) -> Self {
        self.types = Arc::clone(converters.type_registry());
        self.converters = converters;
        self
    }

    /// Registers a package definition.
    pub fn definition(mut self, definition: NodeDefinition) -> Self {
        self.definitions.push((definition, false));
        self
    }

    /// Registers a host-owned intrinsic, which may use the `core` namespace and implicit ports.
    pub(crate) fn intrinsic(mut self, definition: NodeDefinition) -> Self {
        self.definitions.push((definition, true));
        self
    }

    pub fn provider(mut self, provider: ProviderDescriptor) -> Self {
        self.providers.push(provider);
        self
    }

    pub fn build(self) -> Result<Arc<RegistrySnapshot>, RegistryError> {
        let mut definitions: BTreeMap<DefinitionId, BTreeMap<u32, Registered>> = BTreeMap::new();
        for (definition, intrinsic) in self.definitions {
            check_definition(&definition, intrinsic)?;
            check_default_literals(&definition, &self.types)?;
            super::availability::check_availability(&definition)?;
            check_execution(&definition, intrinsic)?;
            let versions = definitions
                .entry(definition.definition.id.clone())
                .or_default();
            if versions.contains_key(&definition.definition.version) {
                return Err(RegistryError::DuplicateDefinition(definition.definition));
            }
            versions.insert(definition.definition.version, Registered::new(definition));
        }
        let mut providers = BTreeMap::new();
        for provider in self.providers {
            let named = provider
                .implements
                .iter()
                .map(|(definition, _)| definition)
                .chain(provider.specializations.keys());
            for definition in named {
                if !definitions
                    .get(&definition.id)
                    .is_some_and(|versions| versions.contains_key(&definition.version))
                {
                    return Err(RegistryError::UnknownDefinition {
                        provider: Box::new(provider.key),
                        definition: definition.clone(),
                    });
                }
            }
            if providers.contains_key(&provider.key) {
                return Err(RegistryError::DuplicateProvider(provider.key));
            }
            providers.insert(provider.key.clone(), provider);
        }
        Ok(Arc::new(RegistrySnapshot {
            generation: crate::id::process_unique(),
            origin: self.origin,
            types: self.types,
            converters: self.converters,
            definitions,
            providers,
        }))
    }
}

fn check_execution(definition: &NodeDefinition, intrinsic: bool) -> Result<(), RegistryError> {
    use super::{Determinism, Volatility};
    let id = &definition.definition.id;
    if (definition.kind == NodeKind::Volatile) != definition.volatility.is_some() {
        return Err(RegistryError::VolatilityKind {
            definition: id.clone(),
        });
    }
    if !intrinsic
        && matches!(
            definition.volatility,
            Some(Volatility::StateRead | Volatility::Clock)
        )
    {
        return Err(RegistryError::VolatilityAuthority {
            definition: id.clone(),
        });
    }
    if (definition.kind == NodeKind::Pure && definition.determinism != Determinism::Deterministic)
        || (definition.volatility == Some(Volatility::Entropy)
            && definition.determinism != Determinism::Seeded)
    {
        return Err(RegistryError::DeterminismKind {
            definition: id.clone(),
        });
    }
    if matches!(
        definition.volatility,
        Some(Volatility::StateRead | Volatility::Clock)
    ) {
        match &definition.ports {
            Ports::Declared(ports) => {
                for port in ports {
                    if port.direction == PortDirection::Input && port.kind == PortKind::Data {
                        return Err(RegistryError::VolatileDataInput {
                            definition: id.clone(),
                            port: port.port_id.clone(),
                        });
                    }
                }
            }
            Ports::Variable(VariableAccess::Read) => {}
            _ => {
                return Err(RegistryError::VolatilePortTemplate {
                    definition: id.clone(),
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod execution_tests {
    use super::*;
    use crate::catalog::{Determinism, Volatility};
    fn reader(class: Volatility, ports: Ports) -> NodeDefinition {
        NodeDefinition {
            definition: DefinitionRef {
                id: "core/read-test".parse().unwrap(),
                version: 1,
            },
            config_version: 1,
            kind: NodeKind::Volatile,
            volatility: Some(class),
            determinism: Determinism::Deterministic,
            suspends: None,
            type_params: vec![],
            ports,
            references: vec![],
            specializations: None,
            config_schema: None,
        }
    }
    #[test]
    fn host_state_and_clock_readers_reject_input_bearing_port_contracts() {
        for class in [Volatility::StateRead, Volatility::Clock] {
            let invalid = reader(
                class,
                Ports::Declared(vec![PortDescriptor::data_in(
                    "value",
                    "core/i64@1".parse().unwrap(),
                )]),
            );
            assert!(matches!(
                check_execution(&invalid, true),
                Err(RegistryError::VolatileDataInput { .. })
            ));
            let valid = reader(
                class,
                Ports::Declared(vec![PortDescriptor::data_out(
                    "value",
                    "core/i64@1".parse().unwrap(),
                    OutputSource::State,
                )]),
            );
            assert!(check_execution(&valid, true).is_ok());
            for ports in [Ports::ForEach, Ports::Variable(VariableAccess::Write)] {
                assert!(matches!(
                    check_execution(&reader(class, ports), true),
                    Err(RegistryError::VolatilePortTemplate { .. })
                ));
            }
        }
    }
}

/// Validate defaults once their ordinary descriptor and input-metadata checks have passed.
/// Unresolved, parameterized, or malformed declared types remain deferred to inference.
fn check_default_literals(
    definition: &NodeDefinition,
    types: &TypeRegistry,
) -> Result<(), RegistryError> {
    let Ports::Declared(ports) = &definition.ports else {
        return Ok(());
    };
    for port in ports {
        let (Some(default), Some(declared_type)) = (&port.default, &port.ty) else {
            continue;
        };
        let Ok(term) = TypeTerm::parse(declared_type) else {
            continue;
        };
        if term.contains_param() || types.validate(&term).is_err() {
            continue;
        }
        if &default.ty != declared_type {
            return Err(RegistryError::DefaultType {
                definition: definition.definition.id.clone(),
                port: port.port_id.clone(),
            });
        }
        if crate::value::literal_codec::decode(default, types).is_err() {
            return Err(RegistryError::DefaultLiteral {
                definition: definition.definition.id.clone(),
                port: port.port_id.clone(),
            });
        }
    }
    Ok(())
}

/// Publication checks for one definition, in order. Each rule group stands alone so a new
/// rule lands in the group it belongs to.
fn check_definition(definition: &NodeDefinition, intrinsic: bool) -> Result<(), RegistryError> {
    let id = &definition.definition.id;
    if !intrinsic && id.is_core() {
        return Err(RegistryError::CoreNamespace(id.clone()));
    }
    check_references(definition)?;
    if let Some(issue) = suspension_contract_issue(definition.kind, definition.suspends) {
        return Err(match issue {
            SuspensionContractIssue::Missing => RegistryError::SuspendsMissing {
                definition: id.clone(),
                kind: definition.kind,
            },
            SuspensionContractIssue::Invalid(suspends) => RegistryError::SuspendsInvalid {
                definition: id.clone(),
                kind: definition.kind,
                suspends,
            },
        });
    }
    if matches!(&definition.ports, Ports::Variable(VariableAccess::Update))
        && definition.kind != NodeKind::Action
    {
        return Err(RegistryError::VariableUpdateKind {
            definition: id.clone(),
            kind: definition.kind,
        });
    }
    if matches!(
        &definition.ports,
        Ports::Variable(VariableAccess::WriteShared | VariableAccess::UpdateShared)
    ) && definition.kind != NodeKind::Async
    {
        return Err(RegistryError::SharedVariableKind {
            definition: id.clone(),
            kind: definition.kind,
        });
    }
    if let Ports::Event(access) = &definition.ports {
        let expected = match access {
            EventAccess::Handle => NodeKind::Entry,
            EventAccess::Emit => NodeKind::Action,
            EventAccess::Await => NodeKind::Async,
        };
        if definition.kind != expected {
            return Err(RegistryError::EventKind {
                definition: id.clone(),
                expected,
                kind: definition.kind,
            });
        }
    }
    let ports = match &definition.ports {
        Ports::Declared(ports) => ports,
        Ports::Variable(_) => return check_variable_ports(definition),
        Ports::Event(_) => return check_event_ports(definition),
        Ports::ForEach => return Ok(()),
    };
    let declared: BTreeSet<&str> = definition
        .type_params
        .iter()
        .map(|param| param.name.as_str())
        .collect();
    let mut seen = BTreeSet::new();
    for port in ports {
        check_port_identity(id, port, &mut seen, intrinsic)?;
        check_port_shape(id, port)?;
        check_kind_ports(definition, port)?;
        check_port_authority(id, port, intrinsic)?;
        check_port_params(id, port, &declared)?;
    }
    // Input initialization metadata checks run after the established descriptor checks,
    // preserving their error precedence (including deferred type-expression parsing).
    for port in ports {
        check_input_metadata(id, port)?;
    }
    Ok(())
}

/// Reference pointers are valid RFC 6901 pointers, each declared once.
fn check_references(definition: &NodeDefinition) -> Result<(), RegistryError> {
    let id = &definition.definition.id;
    let mut pointers = BTreeSet::new();
    for field in &definition.references {
        if pointer_parts(&field.pointer).is_none() {
            return Err(RegistryError::InvalidReferencePointer {
                definition: id.clone(),
                pointer: field.pointer.clone(),
            });
        }
        if !pointers.insert(field.pointer.as_str()) {
            return Err(RegistryError::DuplicateReference {
                definition: id.clone(),
                pointer: field.pointer.clone(),
            });
        }
    }
    Ok(())
}

/// Variable-typed ports need the variable reference that types them.
fn check_variable_ports(definition: &NodeDefinition) -> Result<(), RegistryError> {
    let has_variable = definition
        .references
        .iter()
        .any(|field| field.kind == ReferenceKind::Variable);
    if has_variable {
        Ok(())
    } else {
        Err(RegistryError::VariablePortsWithoutReference {
            definition: definition.definition.id.clone(),
        })
    }
}

/// Event-typed ports need the event reference that types them.
fn check_event_ports(definition: &NodeDefinition) -> Result<(), RegistryError> {
    if definition
        .references
        .iter()
        .any(|field| field.kind == ReferenceKind::Event)
    {
        Ok(())
    } else {
        Err(RegistryError::EventPortsWithoutReference {
            definition: definition.definition.id.clone(),
        })
    }
}

/// Port IDs are unique, and only intrinsics declare implicit port IDs (11 §4.1).
fn check_port_identity<'a>(
    id: &DefinitionId,
    port: &'a PortDescriptor,
    seen: &mut BTreeSet<&'a str>,
    intrinsic: bool,
) -> Result<(), RegistryError> {
    let name = port.port_id.as_str();
    let (definition, port) = (id.clone(), port.port_id.clone());
    if !seen.insert(name) {
        return Err(RegistryError::DuplicatePort { definition, port });
    }
    if !intrinsic && IMPLICIT_PORTS.contains(&name) {
        return Err(RegistryError::ReservedPort { definition, port });
    }
    Ok(())
}

/// A port's kind, direction, type and source agree (11 §4.1).
fn check_port_shape(id: &DefinitionId, port: &PortDescriptor) -> Result<(), RegistryError> {
    let (definition, port_id) = (id.clone(), port.port_id.clone());
    if port.ty.is_some() != matches!(port.kind, PortKind::Data | PortKind::Event) {
        return Err(RegistryError::PortType {
            definition,
            port: port_id,
        });
    }
    if port.kind == PortKind::Data
        && port.direction == PortDirection::Output
        && port.source.is_none()
    {
        return Err(RegistryError::MissingDataOutputSource {
            definition,
            port: port_id,
        });
    }
    if port.kind == PortKind::Event && port.direction != PortDirection::Output {
        return Err(RegistryError::EventInputPort {
            definition,
            port: port_id,
        });
    }
    if let Some(variadic) = port.variadic
        && let Some(max) = variadic.max
        && variadic.min > max
    {
        return Err(RegistryError::InvalidVariadicBounds {
            definition,
            port: port_id,
            min: variadic.min,
            max,
        });
    }
    Ok(())
}

/// The definition's kind allows the port (11 §3).
fn check_kind_ports(
    definition: &NodeDefinition,
    port: &PortDescriptor,
) -> Result<(), RegistryError> {
    if definition.kind == NodeKind::Pure && port.kind == PortKind::Exec {
        return Err(RegistryError::PureExecutionPort {
            definition: definition.definition.id.clone(),
            port: port.port_id.clone(),
        });
    }
    if definition.kind == NodeKind::Volatile && port.kind == PortKind::Exec {
        return Err(RegistryError::VolatileExecutionPort {
            definition: definition.definition.id.clone(),
            port: port.port_id.clone(),
        });
    }
    Ok(())
}

/// Port features reserved to host intrinsics (11 §4.1: `lazy`).
fn check_port_authority(
    id: &DefinitionId,
    port: &PortDescriptor,
    intrinsic: bool,
) -> Result<(), RegistryError> {
    if !intrinsic && port.is_lazy() {
        return Err(RegistryError::LazyPackagePort {
            definition: id.clone(),
            port: port.port_id.clone(),
        });
    }
    Ok(())
}

/// Port types mention only declared type parameters. Unparsable types are reported by
/// inference, not here.
fn check_port_params(
    id: &DefinitionId,
    port: &PortDescriptor,
    declared: &BTreeSet<&str>,
) -> Result<(), RegistryError> {
    if let Some(term) = port.ty.as_ref().and_then(|ty| TypeTerm::parse(ty).ok())
        && let Some(param) = undeclared_param(&term, declared)
    {
        return Err(RegistryError::UndeclaredTypeParam {
            definition: id.clone(),
            port: port.port_id.clone(),
            param,
        });
    }
    Ok(())
}

/// Defaults and optional absence define only how a data input is initialized (11 §4.1).
fn check_input_metadata(id: &DefinitionId, port: &PortDescriptor) -> Result<(), RegistryError> {
    let (definition, port_id) = (id.clone(), port.port_id.clone());
    let is_data_input = port.direction == PortDirection::Input && port.kind == PortKind::Data;
    if port.default.is_some() && !is_data_input {
        return Err(RegistryError::DefaultPortPosition {
            definition,
            port: port_id,
        });
    }
    if port.optional.unwrap_or(false) {
        if !is_data_input {
            return Err(RegistryError::OptionalPortPosition {
                definition,
                port: port_id,
            });
        }
        // Parse only the outer Option wrapper; name resolution (including its inner
        // argument) remains deferred to inference.
        if let Some(ty) = port.ty.as_ref()
            && let Ok(term) = TypeTerm::parse(ty)
            && !term.core("option").is_some_and(|args| args.len() == 1)
        {
            return Err(RegistryError::OptionalType {
                definition,
                port: port_id,
            });
        }
    }
    Ok(())
}

fn undeclared_param(term: &TypeTerm, declared: &BTreeSet<&str>) -> Option<String> {
    match term {
        TypeTerm::Param(name) => (!declared.contains(name.as_str())).then(|| name.clone()),
        TypeTerm::Named(named) => named
            .args
            .iter()
            .find_map(|arg| undeclared_param(arg, declared)),
    }
}

/// Immutable host metadata: types, converters, node definitions and providers.
#[derive(Debug)]
pub struct RegistrySnapshot {
    generation: u64,
    origin: HostOrigin,
    types: Arc<TypeRegistry>,
    converters: Arc<ConverterRegistry>,
    /// By id, then version, so a lookup borrows the caller's id instead of cloning it.
    definitions: BTreeMap<DefinitionId, BTreeMap<u32, Registered>>,
    providers: BTreeMap<ProviderKey, ProviderDescriptor>,
}

/// A registered definition and, for declared ports, its contract built once.
#[derive(Debug)]
struct Registered {
    definition: Arc<NodeDefinition>,
    declared: Option<NodeContract>,
}

impl Registered {
    fn new(definition: NodeDefinition) -> Self {
        let declared = match &definition.ports {
            Ports::Declared(ports) => Some(NodeContract {
                kind: definition.kind,
                suspends: definition.suspends,
                type_params: definition.type_params.clone(),
                ports: ports.clone(),
            }),
            Ports::Variable(_) | Ports::Event(_) | Ports::ForEach => None,
        };
        Self {
            definition: Arc::new(definition),
            declared,
        }
    }
}

impl RegistrySnapshot {
    /// A snapshot of the core types, conversions and intrinsic node definitions.
    pub fn core(origin: HostOrigin) -> Arc<Self> {
        RegistryBuilder::core(origin)
            .build()
            .expect("the built-in registry is valid")
    }

    /// Process-unique identity of this snapshot. Plans record it, so one made against a
    /// replaced snapshot is recognized as stale.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn origin(&self) -> &HostOrigin {
        &self.origin
    }

    pub fn types(&self) -> &Arc<TypeRegistry> {
        &self.types
    }

    pub fn converters(&self) -> &Arc<ConverterRegistry> {
        &self.converters
    }

    pub fn definition(&self, definition: &DefinitionRef) -> Option<&NodeDefinition> {
        self.registered(definition)
            .map(|registered| registered.definition.as_ref())
    }

    fn registered(&self, definition: &DefinitionRef) -> Option<&Registered> {
        self.definitions
            .get(&definition.id)?
            .get(&definition.version)
    }

    pub fn provider(&self, key: &ProviderKey) -> Option<&ProviderDescriptor> {
        self.providers.get(key)
    }

    /// Whether the exact definition version and config version has this provider.
    pub fn provides(
        &self,
        definition: &DefinitionRef,
        config_version: u32,
        provider: &ProviderKey,
    ) -> bool {
        self.provider(provider).is_some_and(|descriptor| {
            descriptor
                .implements
                .contains(&(definition.clone(), config_version))
        })
    }

    /// The single parser for identity references in node config: the registered definition's
    /// `references`, or the language table's field for an intrinsic without a definition.
    pub fn references<'a>(
        &'a self,
        node: &'a NodeInstance,
    ) -> impl Iterator<Item = (&'a ReferenceField, Result<DeclId, ReferenceError>)> + 'a {
        let fields = match self.definition(&node.definition) {
            Some(definition) => definition.references.as_slice(),
            None => intrinsic_reference(node.definition.id.as_str())
                .map_or(&[][..], std::slice::from_ref),
        };
        fields
            .iter()
            .map(|field| (field, field.parse(&node.config)))
    }

    /// The node's contract: the definition's, with variable ports computed from the document.
    /// `None` for an unknown definition. A pure function of the definition and document, so
    /// it cannot go stale.
    pub fn contract<'a>(
        &'a self,
        document: &GraphDocument,
        node: &NodeInstance,
    ) -> Result<Option<Cow<'a, NodeContract>>, ContractError> {
        if is_converter_special_form(node.definition.id.as_str()) {
            return self
                .converter_contract(document, node)
                .map(|contract| Some(Cow::Owned(contract)));
        }
        let Some(registered) = self.registered(&node.definition) else {
            return Ok(None);
        };
        if let Some(declared) = &registered.declared {
            return Ok(Some(Cow::Borrowed(declared)));
        }
        let definition = &registered.definition;
        if matches!(definition.ports, Ports::ForEach) {
            return super::intrinsics::for_each_contract(definition, node)
                .map(|contract| Some(Cow::Owned(contract)));
        }
        let ports = match &definition.ports {
            Ports::Declared(ports) => ports.clone(),
            Ports::ForEach => unreachable!("for-each returned above"),
            Ports::Variable(access) => self.variable_ports(document, node, *access)?,
            Ports::Event(access) => self.event_ports(document, node, *access)?,
        };
        Ok(Some(Cow::Owned(NodeContract {
            kind: definition.kind,
            suspends: definition.suspends,
            type_params: definition.type_params.clone(),
            ports,
        })))
    }

    pub(crate) fn converter_contract(
        &self,
        document: &GraphDocument,
        node: &NodeInstance,
    ) -> Result<NodeContract, ContractError> {
        if node.definition.version != 1 || node.config_version != 1 {
            return Err(ContractError::ConverterConfig);
        }
        let key =
            ConverterConfig::entry_of(&node.config).map_err(|_| ContractError::ConverterConfig)?;
        let definition = self
            .converters
            .entry(&key)
            .ok_or(ContractError::ConverterEntry)?;
        let options = ConverterConfig::from_config(&node.config)
            .map_err(|_| ContractError::ConverterOptions)?
            .options;
        let provider = match &node.provider {
            None => ConverterProvider::Core,
            Some(binding) => {
                let Some(exact) = ProviderKey::of(document, binding) else {
                    return Err(ContractError::ConverterPackageLockMissing);
                };
                ConverterProvider::Package(crate::conversion::PackageProvider {
                    package: exact.package,
                    version: exact.version,
                    content_digest: exact.content_digest,
                    id: exact.id,
                    contract_version: key.contract_version,
                })
            }
        };
        let resolved = self
            .converters
            .resolve_explicit_choice(
                &definition.from,
                &definition.to,
                ExplicitConverterChoice {
                    entry: key,
                    provider,
                    options: options.clone(),
                },
            )
            .map_err(|error| match error {
                crate::conversion::SelectionRejection::InvalidOptions(_) => {
                    ContractError::ConverterOptions
                }
                _ => ContractError::ConverterProvider,
            })?;
        if resolved.options != options {
            return Err(ContractError::ConverterOptions);
        }
        let mut ports = vec![
            PortDescriptor::data_in("value", definition.from.clone()),
            PortDescriptor::data_out("result", definition.to.clone(), OutputSource::Computed),
        ];
        if definition.class == ConversionClass::Lossy {
            ports.push(PortDescriptor::data_out(
                "loss",
                definition
                    .loss_output_type()
                    .expect("lossy entries declare loss output"),
                OutputSource::Computed,
            ));
        }
        Ok(NodeContract {
            kind: crate::document::NodeKind::Pure,
            suspends: None,
            type_params: Vec::new(),
            ports,
        })
    }

    fn event_ports(
        &self,
        document: &GraphDocument,
        node: &NodeInstance,
        access: EventAccess,
    ) -> Result<Vec<PortDescriptor>, ContractError> {
        let Ok(DeclId::Event(id)) = self.reference(node, ReferenceKind::Event) else {
            return Err(ContractError::EventUnknown);
        };
        let payload = document
            .events()
            .get(&id)
            .ok_or(ContractError::EventUnknown)?
            .payload
            .clone();
        Ok(match access {
            EventAccess::Handle => vec![PortDescriptor::data_out(
                "payload",
                payload,
                OutputSource::Payload,
            )],
            EventAccess::Emit => vec![PortDescriptor::data_in("payload", payload)],
            EventAccess::Await => vec![
                PortDescriptor::data_in(
                    "timeout",
                    "core/option@1<core/duration@1>"
                        .parse()
                        .expect("intrinsic type expression is valid"),
                )
                .optional(),
                PortDescriptor::exec_out("then"),
                PortDescriptor::exec_out("timed_out"),
                PortDescriptor {
                    available_on: Some(vec!["then".parse().expect("port id is valid")]),
                    ..PortDescriptor::data_out("payload", payload, OutputSource::Execution)
                },
            ],
        })
    }

    fn variable_ports(
        &self,
        document: &GraphDocument,
        node: &NodeInstance,
        access: VariableAccess,
    ) -> Result<Vec<PortDescriptor>, ContractError> {
        let Ok(DeclId::Variable(id)) = self.reference(node, ReferenceKind::Variable) else {
            return Err(ContractError::VariableUnknown);
        };
        let declaration = document
            .variables()
            .get(&id)
            .ok_or(ContractError::VariableUnknown)?;
        match (access, declaration.scope) {
            (VariableAccess::Write | VariableAccess::Update, VarScope::Workspace) => {
                return Err(ContractError::VariableScope);
            }
            (VariableAccess::WriteShared | VariableAccess::UpdateShared, scope)
                if scope != VarScope::Workspace =>
            {
                return Err(ContractError::SharedVariableScope);
            }
            _ => {}
        }
        let ty = declaration.ty.clone();
        Ok(match access {
            VariableAccess::Read => {
                vec![PortDescriptor::data_out("value", ty, OutputSource::State)]
            }
            VariableAccess::Write | VariableAccess::WriteShared => {
                vec![PortDescriptor::data_in("value", ty)]
            }
            VariableAccess::Update | VariableAccess::UpdateShared => {
                let token = node
                    .config
                    .get("operation")
                    .and_then(serde_json::Value::as_str)
                    .ok_or(ContractError::UpdateOperation)?;
                let operation =
                    WriteOperation::parse_update(token).ok_or(ContractError::UpdateOperation)?;
                let term = TypeTerm::parse(&ty).map_err(|error| match error {
                    crate::error::IdError::TypeTooDeep { .. } => ContractError::TypeComplexity,
                    _ => ContractError::TypeMismatch,
                })?;
                self.types.validate(&term).map_err(|error| match error {
                    TypeResolveError::TooDeep { .. }
                    | TypeResolveError::DerivationTooDeep { .. } => ContractError::TypeComplexity,
                    _ => ContractError::TypeMismatch,
                })?;
                let capability = match operation {
                    WriteOperation::Add => {
                        Some(CapabilityRequest::NumericOperation(NumericOperation::Add))
                    }
                    WriteOperation::Sub => Some(CapabilityRequest::NumericOperation(
                        NumericOperation::Subtract,
                    )),
                    WriteOperation::Min | WriteOperation::Max => {
                        Some(CapabilityRequest::Constraint(Constraint::Ord))
                    }
                    WriteOperation::CompareAndSet => {
                        Some(CapabilityRequest::Constraint(Constraint::Eq))
                    }
                    WriteOperation::Append => {
                        let eligible = term.core("string").is_some_and(|args| args.is_empty())
                            || term.core("bytes").is_some_and(|args| args.is_empty())
                            || term.core("list").is_some_and(|args| args.len() == 1);
                        if !eligible {
                            return Err(ContractError::UpdateConstraint);
                        }
                        // List element types need no Eq or other operation proof.
                        None
                    }
                    WriteOperation::And | WriteOperation::Or | WriteOperation::Xor => {
                        let eligible = [
                            "core/bool@1",
                            "core/i32@1",
                            "core/i64@1",
                            "core/u32@1",
                            "core/u64@1",
                        ]
                        .contains(&ty.as_str());
                        if !eligible {
                            return Err(ContractError::UpdateConstraint);
                        }
                        // Eligibility is language-owned and exact; no package proof or
                        // package implementation is consulted for these kernels.
                        None
                    }
                    WriteOperation::Set => return Err(ContractError::UpdateOperation),
                };
                if let Some(capability) = capability {
                    match self.types.supports_term(&term, capability) {
                        Ok(Some(_)) => {}
                        Ok(None) => return Err(ContractError::UpdateConstraint),
                        Err(CapabilityError::DerivationTooDeep { .. })
                        | Err(CapabilityError::Type(TypeResolveError::TooDeep { .. }))
                        | Err(CapabilityError::Type(TypeResolveError::DerivationTooDeep {
                            ..
                        })) => {
                            return Err(ContractError::TypeComplexity);
                        }
                        Err(CapabilityError::RecursiveDerivation { .. }) => {
                            return Err(ContractError::UpdateConstraint);
                        }
                        Err(CapabilityError::Type(_)) => return Err(ContractError::TypeMismatch),
                    }
                }
                let then: PortId = "then".parse().map_err(|_| ContractError::TypeMismatch)?;
                let compare_and_set = operation == WriteOperation::CompareAndSet;
                let mut ports = Vec::with_capacity(if compare_and_set { 5 } else { 3 });
                if compare_and_set {
                    ports.push(PortDescriptor::data_in("expected", ty.clone()));
                }
                ports.extend([
                    PortDescriptor::data_in("value", ty.clone()),
                    PortDescriptor {
                        available_on: Some(vec![then.clone()]),
                        ..PortDescriptor::data_out("old", ty.clone(), OutputSource::Execution)
                    },
                    PortDescriptor {
                        available_on: Some(vec![then.clone()]),
                        ..PortDescriptor::data_out("new", ty, OutputSource::Execution)
                    },
                ]);
                if compare_and_set {
                    ports.push(PortDescriptor {
                        available_on: Some(vec![then]),
                        ..PortDescriptor::data_out(
                            "exchanged",
                            "core/bool@1"
                                .parse()
                                .map_err(|_| ContractError::TypeMismatch)?,
                            OutputSource::Execution,
                        )
                    });
                }
                ports
            }
        })
    }

    /// The first reference of `kind` the node declares.
    pub fn reference(
        &self,
        node: &NodeInstance,
        kind: ReferenceKind,
    ) -> Result<DeclId, ReferenceError> {
        self.references(node)
            .find(|(field, _)| field.kind == kind)
            .map_or(Err(ReferenceError::Missing), |(_, parsed)| parsed)
    }

    /// The argument sets of a definition: the provider's own with a provider, the core
    /// implementation list without one. `None` means constraints decide.
    pub fn specializations(
        &self,
        definition: &DefinitionRef,
        provider: Option<&ProviderKey>,
    ) -> Option<&[TypeArgs]> {
        match provider {
            Some(key) => self
                .provider(key)?
                .specializations
                .get(definition)
                .map(Vec::as_slice),
            None => self.definition(definition)?.specializations.as_deref(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::NodeInstance;
    use crate::document::{NodeKind, PortDescriptor, TypeParam};

    fn origin() -> HostOrigin {
        HostOrigin::new("1.0.0", format!("sha256:{}", "0".repeat(64))).unwrap()
    }

    fn dref(id: &str) -> DefinitionRef {
        DefinitionRef {
            id: id.parse().unwrap(),
            version: 1,
        }
    }

    fn port(id: &str, ty: Option<&str>) -> PortDescriptor {
        serde_json::from_value(serde_json::json!({
            "port_id": id,
            "direction": "input",
            "kind": "data",
            "type": ty,
        }))
        .unwrap()
    }

    fn definition(id: &str, params: &[&str], ports: Vec<PortDescriptor>) -> NodeDefinition {
        NodeDefinition {
            definition: dref(id),
            config_version: 1,
            kind: NodeKind::Pure,
            volatility: None,
            determinism: super::super::Determinism::Deterministic,
            suspends: None,
            type_params: params
                .iter()
                .map(|name| {
                    serde_json::from_value::<TypeParam>(serde_json::json!({"name": name})).unwrap()
                })
                .collect(),
            ports: Ports::Declared(ports),
            references: Vec::new(),
            specializations: None,
            config_schema: None,
        }
    }

    fn key(id: &str) -> ProviderKey {
        ProviderKey {
            package: "acme".into(),
            version: "1.0.0".into(),
            content_digest: "d".into(),
            id: id.into(),
        }
    }

    fn provider(id: &str, implements: &[&str]) -> ProviderDescriptor {
        ProviderDescriptor {
            key: key(id),
            implements: implements.iter().map(|d| (dref(d), 1)).collect(),
            specializations: BTreeMap::new(),
        }
    }

    fn build(builder: RegistryBuilder) -> Result<Arc<RegistrySnapshot>, RegistryError> {
        builder.build()
    }

    fn builder() -> RegistryBuilder {
        RegistryBuilder::core(origin())
    }

    #[test]
    fn suspension_contract_is_checked_for_declared_variable_and_intrinsic_definitions() {
        for kind in [
            NodeKind::Pure,
            NodeKind::Volatile,
            NodeKind::Action,
            NodeKind::Entry,
        ] {
            let mut valid = definition("acme/sync", &[], vec![]);
            valid.kind = kind;
            if kind == NodeKind::Volatile {
                valid.volatility = Some(super::super::Volatility::Entropy);
                valid.determinism = super::super::Determinism::Seeded;
            }
            assert!(build(builder().definition(valid)).is_ok(), "{kind:?}");
        }
        for kind in [NodeKind::Async, NodeKind::Iterator] {
            for suspends in [Some(Suspends::Maybe), Some(Suspends::Always)] {
                let mut valid = definition("acme/suspending", &[], vec![]);
                valid.kind = kind;
                valid.suspends = suspends;
                assert!(build(builder().definition(valid)).is_ok(), "{kind:?}");
            }
            let mut missing = definition("acme/missing", &[], vec![]);
            missing.kind = kind;
            assert!(matches!(
                build(builder().definition(missing)),
                Err(RegistryError::SuspendsMissing { kind: actual, .. }) if actual == kind
            ));
            let mut never = definition("acme/never", &[], vec![]);
            never.kind = kind;
            never.suspends = Some(Suspends::Never);
            assert!(matches!(
                build(builder().definition(never)),
                Err(RegistryError::SuspendsInvalid { kind: actual, suspends: Suspends::Never, .. }) if actual == kind
            ));
        }

        let mut contextual = definition("acme/contextual", &[], vec![]);
        contextual.kind = NodeKind::Async;
        contextual.ports = Ports::Variable(VariableAccess::Read);
        contextual.references = vec![ReferenceField {
            pointer: "/variable".into(),
            kind: ReferenceKind::Variable,
        }];
        assert!(matches!(
            build(builder().definition(contextual)),
            Err(RegistryError::SuspendsMissing {
                kind: NodeKind::Async,
                ..
            })
        ));

        let mut intrinsic = definition("core/missing-suspension", &[], vec![]);
        intrinsic.kind = NodeKind::Async;
        assert!(matches!(
            build(builder().intrinsic(intrinsic)),
            Err(RegistryError::SuspendsMissing {
                kind: NodeKind::Async,
                ..
            })
        ));
        assert!(build(builder()).is_ok(), "built-in definitions publish");
    }

    #[test]
    fn registered_definitions_and_providers_are_found() {
        let snapshot = build(
            builder()
                .definition(definition("acme/add", &["T"], vec![port("a", Some("$T"))]))
                .provider(provider("p", &["acme/add"])),
        )
        .unwrap();
        let add = dref("acme/add");
        assert!(snapshot.definition(&add).is_some());
        assert!(snapshot.definition(&dref("acme/sub")).is_none());
        assert!(snapshot.provides(&add, 1, &key("p")));
        assert!(!snapshot.provides(&add, 2, &key("p")));
        assert_eq!(snapshot.specializations(&add, None), None);
        assert_ne!(
            snapshot.generation(),
            RegistrySnapshot::core(origin()).generation()
        );
    }

    #[test]
    fn duplicate_definition_is_rejected() {
        let result = build(
            builder()
                .definition(definition("acme/add", &[], vec![]))
                .definition(definition("acme/add", &[], vec![])),
        );
        assert_eq!(
            result.unwrap_err(),
            RegistryError::DuplicateDefinition(dref("acme/add"))
        );
    }

    #[test]
    fn duplicate_provider_is_rejected() {
        let result = build(
            builder()
                .definition(definition("acme/add", &[], vec![]))
                .provider(provider("p", &["acme/add"]))
                .provider(provider("p", &[])),
        );
        assert_eq!(
            result.unwrap_err(),
            RegistryError::DuplicateProvider(key("p"))
        );
    }

    #[test]
    fn package_definitions_cannot_use_the_core_namespace() {
        for id in ["core/mine", "core/branch"] {
            let result = build(builder().definition(definition(id, &[], vec![])));
            assert!(
                matches!(result, Err(RegistryError::CoreNamespace(_))),
                "{id}"
            );
        }
    }

    #[test]
    fn duplicate_port_ids_are_rejected() {
        let ports = vec![port("a", Some("core/i32@1")), port("a", Some("core/i32@1"))];
        let result = build(builder().definition(definition("acme/add", &[], ports)));
        assert!(matches!(result, Err(RegistryError::DuplicatePort { .. })));
    }

    #[test]
    fn reserved_ports_are_rejected_for_packages_but_allowed_for_intrinsics() {
        let reserved = || vec![PortDescriptor::exec_out("then")];
        let result = build(builder().definition(definition("acme/add", &[], reserved())));
        assert!(matches!(result, Err(RegistryError::ReservedPort { .. })));
        let mut intrinsic = definition("core/mine", &[], reserved());
        intrinsic.kind = NodeKind::Action;
        assert!(build(builder().intrinsic(intrinsic)).is_ok());
    }

    #[test]
    fn only_data_and_event_ports_have_a_type() {
        let typed_exec = PortDescriptor {
            ty: Some("core/i64@1".parse().unwrap()),
            ..PortDescriptor::exec_in("go")
        };
        for ports in [vec![port("a", None)], vec![typed_exec]] {
            let result = build(builder().definition(definition("acme/add", &[], ports)));
            assert!(matches!(result, Err(RegistryError::PortType { .. })));
        }
    }

    #[test]
    fn pure_definitions_cannot_publish_execution_ports() {
        let result = build(builder().definition(definition(
            "acme/pure-with-exec",
            &[],
            vec![PortDescriptor::exec_out("out")],
        )));
        assert_eq!(
            result.unwrap_err(),
            RegistryError::PureExecutionPort {
                definition: "acme/pure-with-exec".parse().unwrap(),
                port: "out".parse().unwrap(),
            }
        );
    }

    #[test]
    fn pure_data_action_and_host_intrinsic_definitions_still_publish() {
        let pure = definition(
            "acme/pure-data",
            &[],
            vec![port("value", Some("core/i64@1"))],
        );
        let mut action = definition("acme/action", &[], vec![PortDescriptor::exec_out("done")]);
        action.kind = NodeKind::Action;
        assert!(build(builder().definition(pure).definition(action)).is_ok());
        assert!(build(builder()).is_ok());
    }

    #[test]
    fn event_inputs_are_rejected_at_publication() {
        let event_output = PortDescriptor {
            direction: PortDirection::Output,
            kind: PortKind::Event,
            ty: Some("core/i64@1".parse().unwrap()),
            ..PortDescriptor::data_in("event", "core/i64@1".parse().unwrap())
        };
        let mut valid = definition("acme/event-output", &[], vec![event_output]);
        valid.kind = NodeKind::Action;
        assert!(build(builder().definition(valid)).is_ok());

        let event_input = PortDescriptor {
            direction: crate::document::PortDirection::Input,
            kind: PortKind::Event,
            ty: Some("core/i64@1".parse().unwrap()),
            ..PortDescriptor::data_in("event", "core/i64@1".parse().unwrap())
        };
        let result =
            build(builder().definition(definition("acme/event-input", &[], vec![event_input])));
        assert_eq!(
            result.unwrap_err(),
            RegistryError::EventInputPort {
                definition: "acme/event-input".parse().unwrap(),
                port: "event".parse().unwrap(),
            }
        );
    }

    #[test]
    fn only_host_intrinsics_may_publish_lazy_ports() {
        let lazy = || PortDescriptor::data_in("value", "core/i64@1".parse().unwrap()).lazy();
        let result = build(builder().definition(definition("acme/lazy", &[], vec![lazy()])));
        assert_eq!(
            result.unwrap_err(),
            RegistryError::LazyPackagePort {
                definition: "acme/lazy".parse().unwrap(),
                port: "value".parse().unwrap(),
            }
        );

        let mut intrinsic = definition("core/lazy", &[], vec![lazy()]);
        assert!(build(builder().intrinsic(intrinsic.clone())).is_ok());
        // An explicitly false lazy flag is semantically non-lazy and is permitted for packages.
        intrinsic.definition = dref("acme/non-lazy");
        let mut non_lazy = PortDescriptor::data_in("value", "core/i64@1".parse().unwrap());
        non_lazy.lazy = Some(false);
        intrinsic.ports = Ports::Declared(vec![non_lazy]);
        assert!(build(builder().definition(intrinsic)).is_ok());
    }

    #[test]
    fn declared_data_outputs_require_an_explicit_source() {
        let missing = PortDescriptor {
            source: None,
            ..PortDescriptor::data_out(
                "value",
                "core/i64@1".parse().unwrap(),
                OutputSource::Computed,
            )
        };
        let result =
            build(builder().definition(definition("acme/missing-source", &[], vec![missing])));
        assert_eq!(
            result.unwrap_err(),
            RegistryError::MissingDataOutputSource {
                definition: dref("acme/missing-source").id,
                port: "value".parse().unwrap(),
            }
        );
    }

    #[test]
    fn declared_data_output_source_categories_publish_and_other_positions_are_unchanged() {
        let cases = [
            (
                "acme/computed",
                NodeKind::Pure,
                OutputSource::Computed,
                false,
            ),
            (
                "acme/execution",
                NodeKind::Action,
                OutputSource::Execution,
                false,
            ),
            ("core/state", NodeKind::Volatile, OutputSource::State, true),
            (
                "acme/payload",
                NodeKind::Entry,
                OutputSource::Payload,
                false,
            ),
            (
                "acme/iteration",
                NodeKind::Iterator,
                OutputSource::Iteration,
                false,
            ),
        ];
        let mut registry_builder = builder();
        for (id, kind, source, intrinsic) in cases {
            let mut definition = definition(
                id,
                &[],
                vec![PortDescriptor::data_out(
                    "value",
                    "core/i64@1".parse().unwrap(),
                    source,
                )],
            );
            definition.kind = kind;
            if kind == NodeKind::Volatile {
                definition.volatility = Some(super::super::Volatility::StateRead);
            }
            if kind == NodeKind::Iterator {
                definition.suspends = Some(crate::document::Suspends::Maybe);
            }
            registry_builder = if intrinsic {
                registry_builder.intrinsic(definition)
            } else {
                registry_builder.definition(definition)
            };
        }
        assert!(build(registry_builder).is_ok());

        let event_without_source = PortDescriptor {
            direction: PortDirection::Output,
            kind: PortKind::Event,
            ..PortDescriptor::data_in("event", "core/i64@1".parse().unwrap())
        };
        let action = NodeDefinition {
            kind: NodeKind::Action,
            ..definition(
                "acme/other-positions",
                &[],
                vec![
                    PortDescriptor::data_in("input", "core/i64@1".parse().unwrap()),
                    event_without_source,
                    PortDescriptor::exec_out("exec_out"),
                ],
            )
        };
        assert!(build(builder().definition(action)).is_ok());
    }

    #[test]
    fn finite_variadic_bounds_publish_when_ordered_and_reject_inversion() {
        for (min, max) in [(0, Some(0)), (2, Some(2)), (1, Some(3)), (u16::MAX, None)] {
            let mut definition = definition("acme/variadic", &[], vec![]);
            definition.ports = Ports::Declared(vec![
                PortDescriptor::data_in("items", "core/i64@1".parse().unwrap()).variadic(min, max),
            ]);
            assert!(
                build(builder().definition(definition)).is_ok(),
                "{min}..{max:?}"
            );
        }

        for intrinsic in [false, true] {
            let mut definition = definition("acme/invalid-variadic", &[], vec![]);
            definition.ports = Ports::Declared(vec![
                PortDescriptor::data_in("items", "core/i64@1".parse().unwrap())
                    .variadic(3, Some(2)),
            ]);
            let builder = if intrinsic {
                builder().intrinsic(definition)
            } else {
                builder().definition(definition)
            };
            assert_eq!(
                build(builder).unwrap_err(),
                RegistryError::InvalidVariadicBounds {
                    definition: dref("acme/invalid-variadic").id,
                    port: "items".parse().unwrap(),
                    min: 3,
                    max: 2,
                }
            );
        }

        // Core sequence/fork intrinsic definitions use valid, unbounded variadic outputs.
        assert!(build(builder()).is_ok());
    }

    #[test]
    fn port_types_may_only_use_declared_parameters() {
        let ports = || vec![port("a", Some("core/list@1<$T>"))];
        let result = build(builder().definition(definition("acme/add", &[], ports())));
        assert!(matches!(
            result,
            Err(RegistryError::UndeclaredTypeParam { ref param, .. }) if param == "T"
        ));
        assert!(build(builder().definition(definition("acme/add", &["T"], ports()))).is_ok());
    }

    fn with_references(pointers: &[&str]) -> NodeDefinition {
        let mut definition = definition("acme/get", &[], vec![]);
        definition.references = pointers
            .iter()
            .map(|pointer| ReferenceField {
                pointer: (*pointer).to_owned(),
                kind: ReferenceKind::Variable,
            })
            .collect();
        definition
    }

    #[test]
    fn reference_pointers_must_be_valid_and_unique() {
        assert!(build(builder().definition(with_references(&["/variable", "/a~1b/c~0"]))).is_ok());
        for pointer in ["", "variable", "/a~", "/a~2"] {
            let result = build(builder().definition(with_references(&[pointer])));
            assert!(
                matches!(result, Err(RegistryError::InvalidReferencePointer { .. })),
                "{pointer:?}"
            );
        }
        let result = build(builder().definition(with_references(&["/variable", "/variable"])));
        assert!(matches!(
            result,
            Err(RegistryError::DuplicateReference { .. })
        ));
    }

    #[test]
    fn variable_ports_need_a_variable_reference() {
        let mut get = definition("acme/get", &[], vec![]);
        get.ports = Ports::Variable(VariableAccess::Read);
        let result = build(builder().definition(get.clone()));
        assert!(matches!(
            result,
            Err(RegistryError::VariablePortsWithoutReference { .. })
        ));
        get.references = vec![ReferenceField {
            pointer: "/variable".into(),
            kind: ReferenceKind::Variable,
        }];
        assert!(build(builder().definition(get)).is_ok());
    }

    #[test]
    fn update_definition_must_be_an_action() {
        let mut update = definition("acme/update", &[], vec![]);
        update.kind = NodeKind::Pure;
        update.ports = Ports::Variable(VariableAccess::Update);
        update.references = vec![ReferenceField {
            pointer: "/variable".into(),
            kind: ReferenceKind::Variable,
        }];
        assert!(matches!(
            build(builder().definition(update)),
            Err(RegistryError::VariableUpdateKind { definition, kind: NodeKind::Pure })
                if definition.as_str() == "acme/update"
        ));
    }

    #[test]
    fn providers_must_name_registered_definitions() {
        let result = build(builder().provider(provider("p", &["acme/add"])));
        assert!(matches!(
            result,
            Err(RegistryError::UnknownDefinition { .. })
        ));
        let mut specialized = provider("p", &[]);
        specialized
            .specializations
            .insert(dref("acme/add"), Vec::new());
        let result = build(builder().provider(specialized));
        assert!(matches!(
            result,
            Err(RegistryError::UnknownDefinition { .. })
        ));
    }

    #[test]
    fn specializations_come_from_the_definition_or_the_provider() {
        let args = |ty: &str| TypeArgs::from([("T".parse().unwrap(), ty.parse().unwrap())]);
        let mut add = definition("acme/add", &["T"], vec![]);
        add.specializations = Some(vec![args("core/i32@1")]);
        let mut p = provider("p", &["acme/add"]);
        p.specializations
            .insert(dref("acme/add"), vec![args("core/i64@1")]);
        let snapshot = build(builder().definition(add).provider(p)).unwrap();
        let add = dref("acme/add");
        assert_eq!(
            snapshot.specializations(&add, None),
            Some([args("core/i32@1")].as_slice())
        );
        assert_eq!(
            snapshot.specializations(&add, Some(&key("p"))),
            Some([args("core/i64@1")].as_slice())
        );
        assert_eq!(snapshot.specializations(&add, Some(&key("q"))), None);
    }

    const VARIABLE: &str = "11111111-1111-4111-8111-111111111111";
    const FUNCTION: &str = "22222222-2222-4222-8222-222222222222";

    fn node(definition: &str, config: serde_json::Value) -> NodeInstance {
        serde_json::from_value(serde_json::json!({
            "id": "33333333-3333-4333-8333-333333333333",
            "definition_id": definition,
            "definition_version": 1,
            "config_version": 1,
            "config": config,
        }))
        .unwrap()
    }

    #[test]
    fn core_ids_without_a_definition_use_the_language_table() {
        let snapshot = RegistrySnapshot::core(origin());
        let variable = VARIABLE.parse().unwrap();
        let get = node(
            "core/get-variable",
            serde_json::json!({"variable": VARIABLE}),
        );
        assert_eq!(
            snapshot.reference(&get, ReferenceKind::Variable),
            Ok(DeclId::Variable(variable))
        );
        assert_eq!(
            snapshot.reference(&get, ReferenceKind::Event),
            Err(ReferenceError::Missing)
        );
        let call = node("core/call", serde_json::json!({"function": FUNCTION}));
        let fields: Vec<_> = snapshot.references(&call).collect();
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].0.pointer, "/function");
        assert_eq!(fields[0].1, Ok(DeclId::Function(FUNCTION.parse().unwrap())));
        let branch = node("core/branch", serde_json::json!({}));
        assert_eq!(snapshot.references(&branch).count(), 0);
    }

    #[test]
    fn missing_and_malformed_reference_values_are_distinguished() {
        let snapshot = RegistrySnapshot::core(origin());
        for (config, expected) in [
            (serde_json::json!({}), ReferenceError::Missing),
            (
                serde_json::json!({"variable": 3}),
                ReferenceError::Malformed,
            ),
            (
                serde_json::json!({"variable": "nope"}),
                ReferenceError::Malformed,
            ),
            (
                serde_json::json!({"variable": "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA"}),
                ReferenceError::Malformed,
            ),
        ] {
            let set = node("core/set-variable", config);
            assert_eq!(
                snapshot.reference(&set, ReferenceKind::Variable),
                Err(expected)
            );
        }
    }

    #[test]
    fn registered_definitions_declare_their_own_reference_fields() {
        let mut definition = definition("acme/read", &[], vec![]);
        definition.references = vec![ReferenceField {
            pointer: "/target/id".into(),
            kind: ReferenceKind::Event,
        }];
        let snapshot = build(builder().definition(definition)).unwrap();
        let config = serde_json::json!({"target": {"id": VARIABLE}});
        let read = node("acme/read", config);
        assert_eq!(
            snapshot.reference(&read, ReferenceKind::Event),
            Ok(DeclId::Event(VARIABLE.parse().unwrap()))
        );
        // A registered definition's list replaces the core table, even for a core ID name.
        let get = node("acme/read", serde_json::json!({"variable": VARIABLE}));
        assert_eq!(
            snapshot.reference(&get, ReferenceKind::Variable),
            Err(ReferenceError::Missing)
        );
    }

    #[test]
    fn the_core_snapshot_registers_the_control_intrinsics() {
        let snapshot = RegistrySnapshot::core(origin());
        for id in ["core/branch", "core/sequence", "core/wait", "core/continue"] {
            assert!(snapshot.definition(&dref(id)).is_some(), "{id}");
        }
        assert!(snapshot.definition(&dref("core/get-variable")).is_some());
        assert!(snapshot.definition(&dref("core/call")).is_none());
    }
}
