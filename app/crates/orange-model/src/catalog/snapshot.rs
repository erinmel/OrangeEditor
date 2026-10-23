//! The immutable registry snapshot and its builder.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use thiserror::Error;

use super::intrinsics::intrinsic_definitions;
use super::{NodeDefinition, Ports, ProviderDescriptor, ProviderKey, TypeArgs};
use crate::conversion::{ConverterRegistry, ConverterRegistryBuilder};
use crate::document::{DefinitionRef, is_intrinsic_definition};
use crate::id::{DefinitionId, PortId};
use crate::type_term::TypeTerm;
use crate::types::TypeRegistry;
use crate::validation::HostOrigin;

/// Implicit exec ports of 11 §4.1; only intrinsics may declare them.
const RESERVED_PORTS: [&str; 7] = [
    "in",
    "then",
    "failed",
    "error",
    "cancelled",
    "body",
    "completed",
];

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RegistryError {
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
        let mut definitions = BTreeMap::new();
        for (definition, intrinsic) in self.definitions {
            check_definition(&definition, intrinsic)?;
            let key = (
                definition.definition.id.clone(),
                definition.definition.version,
            );
            if definitions.contains_key(&key) {
                return Err(RegistryError::DuplicateDefinition(definition.definition));
            }
            definitions.insert(key, Arc::new(definition));
        }
        let mut providers = BTreeMap::new();
        for provider in self.providers {
            let named = provider
                .implements
                .iter()
                .map(|(definition, _)| definition)
                .chain(provider.specializations.keys());
            for definition in named {
                if !definitions.contains_key(&(definition.id.clone(), definition.version)) {
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

fn check_definition(definition: &NodeDefinition, intrinsic: bool) -> Result<(), RegistryError> {
    let id = &definition.definition.id;
    if !intrinsic && (id.is_core() || is_intrinsic_definition(id.as_str())) {
        return Err(RegistryError::CoreNamespace(id.clone()));
    }
    let mut pointers = BTreeSet::new();
    for field in &definition.references {
        if !valid_pointer(&field.pointer) {
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
    let Ports::Declared(ports) = &definition.ports else {
        return Ok(());
    };
    let declared: BTreeSet<&str> = definition
        .type_params
        .iter()
        .map(|param| param.name.as_str())
        .collect();
    let mut seen = BTreeSet::new();
    for port in ports {
        let name = port.port_id.as_str();
        if !seen.insert(name) {
            return Err(RegistryError::DuplicatePort {
                definition: id.clone(),
                port: port.port_id.clone(),
            });
        }
        if !intrinsic && RESERVED_PORTS.contains(&name) {
            return Err(RegistryError::ReservedPort {
                definition: id.clone(),
                port: port.port_id.clone(),
            });
        }
        // Unparsable types are reported by inference, not here.
        if let Some(term) = port.ty.as_ref().and_then(|ty| TypeTerm::parse(ty).ok())
            && let Some(param) = undeclared_param(&term, &declared)
        {
            return Err(RegistryError::UndeclaredTypeParam {
                definition: id.clone(),
                port: port.port_id.clone(),
                param,
            });
        }
    }
    Ok(())
}

/// A non-empty RFC 6901 pointer: it starts with `/` and every `~` escapes `0` or `1`.
fn valid_pointer(pointer: &str) -> bool {
    pointer.starts_with('/')
        && pointer
            .split('~')
            .skip(1)
            .all(|rest| rest.starts_with(['0', '1']))
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
    definitions: BTreeMap<(DefinitionId, u32), Arc<NodeDefinition>>,
    providers: BTreeMap<ProviderKey, ProviderDescriptor>,
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
        self.definitions
            .get(&(definition.id.clone(), definition.version))
            .map(Arc::as_ref)
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
    use crate::catalog::{ReferenceField, ReferenceKind};
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
        let reserved = || vec![port("then", None)];
        let result = build(builder().definition(definition("acme/add", &[], reserved())));
        assert!(matches!(result, Err(RegistryError::ReservedPort { .. })));
        assert!(build(builder().intrinsic(definition("core/mine", &[], reserved()))).is_ok());
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

    #[test]
    fn the_core_snapshot_registers_the_control_intrinsics() {
        let snapshot = RegistrySnapshot::core(origin());
        for id in ["core/branch", "core/sequence", "core/wait", "core/continue"] {
            assert!(snapshot.definition(&dref(id)).is_some(), "{id}");
        }
        assert!(snapshot.definition(&dref("core/get-variable")).is_none());
    }
}
