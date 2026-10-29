use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::id::TypeExpr;
use crate::type_term::TypeTerm;
use crate::types::{TypeOwner, TypeRegistry};

use super::{
    ConversionClass, ConverterDefinition, ConverterErrorCode, ConverterKey, ConverterProvider,
    ConverterRegistrationError, NamedPolicy, PackageProvider,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct EntryState {
    definition: ConverterDefinition,
    owner: TypeOwner,
    providers: BTreeSet<ConverterProvider>,
    new_binding_default: Option<ConverterProvider>,
}

/// Mutable construction phase for a converter metadata snapshot.
#[derive(Debug)]
pub struct ConverterRegistryBuilder {
    types: Arc<TypeRegistry>,
    entries: BTreeMap<ConverterKey, EntryState>,
}

impl ConverterRegistryBuilder {
    /// Starts an empty converter registry associated with a frozen type registry.
    pub fn new(types: Arc<TypeRegistry>) -> Self {
        Self {
            types,
            entries: BTreeMap::new(),
        }
    }

    /// Starts a registry with the foundational numeric conversions used by v1 authoring.
    pub fn with_core_conversions(
        types: Arc<TypeRegistry>,
    ) -> Result<Self, ConverterRegistrationError> {
        let mut builder = Self::new(types);
        builder.register_core_entry(core_definition(
            "core/i32-to-i64",
            "core/i32@1",
            "core/i64@1",
            ConversionClass::LosslessTotal,
            vec![],
            vec![],
        )?)?;
        builder.register_core_entry(core_definition(
            "core/i64-to-i32",
            "core/i64@1",
            "core/i32@1",
            ConversionClass::CheckedPartial,
            vec![],
            vec![ConverterErrorCode::new("core/OutOfRange")?],
        )?)?;
        builder.register_core_entry(core_definition(
            "core/f32-to-f64",
            "core/f32@1",
            "core/f64@1",
            ConversionClass::LosslessTotal,
            vec![],
            vec![],
        )?)?;
        builder.register_core_entry(core_definition(
            "core/f64-to-f32",
            "core/f64@1",
            "core/f32@1",
            ConversionClass::Lossy,
            vec![
                NamedPolicy {
                    name: "rounding".to_owned(),
                    choices: vec!["nearest-even".to_owned()],
                    default: Some("nearest-even".to_owned()),
                },
                NamedPolicy {
                    name: "overflow".to_owned(),
                    choices: vec!["reject".to_owned()],
                    default: Some("reject".to_owned()),
                },
            ],
            vec![ConverterErrorCode::new("core/OutOfRange")?],
        )?)?;
        Ok(builder)
    }

    /// Registers a semantic entry owned by a package that owns at least one endpoint.
    ///
    /// Registration authority is supplied by the host and must match the entry ID
    /// namespace. It is not itself package authentication; WU-03 owns that boundary.
    pub fn register_package_entry(
        &mut self,
        package_namespace: &str,
        definition: ConverterDefinition,
    ) -> Result<ConverterKey, ConverterRegistrationError> {
        if package_namespace == "core" || definition.id.namespace() != package_namespace {
            return Err(ConverterRegistrationError::EntryNamespaceMismatch {
                entry: definition.id,
                owner: package_namespace.to_owned(),
            });
        }
        let (from_owner, to_owner) = self.validate_definition(&definition)?;
        let owns_endpoint = matches!(&from_owner, TypeOwner::Package(namespace) if namespace == package_namespace)
            || matches!(&to_owner, TypeOwner::Package(namespace) if namespace == package_namespace);
        if !owns_endpoint {
            return Err(ConverterRegistrationError::ForeignToForeign {
                from: definition.from.clone(),
                to: definition.to,
                owner: package_namespace.to_owned(),
            });
        }
        self.insert_entry(definition, TypeOwner::Package(package_namespace.to_owned()))
    }

    /// Adds an implementation alternative for an existing semantic entry.
    ///
    /// Alternative providers implement the existing entry contract; they do not acquire
    /// authority to register new semantics for its endpoints.
    pub fn register_package_provider(
        &mut self,
        package_namespace: &str,
        key: &ConverterKey,
        provider: PackageProvider,
    ) -> Result<(), ConverterRegistrationError> {
        provider.validate()?;
        if provider.package != package_namespace {
            return Err(ConverterRegistrationError::ProviderAuthorityMismatch {
                authority: package_namespace.to_owned(),
                provider: provider.package,
            });
        }
        let entry = self
            .entries
            .get_mut(key)
            .ok_or_else(|| ConverterRegistrationError::EntryNotFound { entry: key.clone() })?;
        if provider.contract_version != key.contract_version {
            return Err(ConverterRegistrationError::ProviderContractMismatch {
                entry: key.clone(),
                found: provider.contract_version,
            });
        }
        if entry.providers.iter().any(|existing| {
            matches!(existing, ConverterProvider::Package(existing)
                if existing.package == provider.package
                    && existing.version == provider.version
                    && existing.id == provider.id
                    && existing.contract_version == provider.contract_version
                    && existing.content_digest != provider.content_digest)
        }) {
            return Err(ConverterRegistrationError::ConflictingProviderDigest {
                entry: key.clone(),
                package: provider.package,
                version: provider.version,
                id: provider.id,
                contract_version: provider.contract_version,
            });
        }
        entry.providers.insert(ConverterProvider::Package(provider));
        Ok(())
    }

    /// Sets the explicit default for new bindings of one semantic entry.
    ///
    /// Defaults are keyed by entry contract, not by the broader pair slot. Existing graph
    /// bindings are never changed by this operation.
    pub fn set_new_binding_default(
        &mut self,
        key: &ConverterKey,
        provider: ConverterProvider,
    ) -> Result<(), ConverterRegistrationError> {
        let entry = self
            .entries
            .get_mut(key)
            .ok_or_else(|| ConverterRegistrationError::EntryNotFound { entry: key.clone() })?;
        if !entry.providers.contains(&provider) {
            return Err(ConverterRegistrationError::ProviderNotRegistered { entry: key.clone() });
        }
        entry.new_binding_default = Some(provider);
        Ok(())
    }

    /// Publishes immutable entries, alternatives and explicit defaults.
    pub fn build(self) -> Arc<ConverterRegistry> {
        let mut pairs: BTreeMap<(TypeExpr, TypeExpr), Vec<ConverterKey>> = BTreeMap::new();
        for (key, state) in &self.entries {
            pairs
                .entry((state.definition.from.clone(), state.definition.to.clone()))
                .or_default()
                .push(key.clone());
        }
        Arc::new(ConverterRegistry {
            generation: crate::id::process_unique(),
            types: self.types,
            entries: self.entries,
            pairs,
        })
    }

    fn register_core_entry(
        &mut self,
        definition: ConverterDefinition,
    ) -> Result<ConverterKey, ConverterRegistrationError> {
        if definition.id.namespace() != "core" {
            return Err(ConverterRegistrationError::EntryNamespaceMismatch {
                entry: definition.id,
                owner: "core".to_owned(),
            });
        }
        let (from_owner, to_owner) = self.validate_definition(&definition)?;
        if !matches!(from_owner, TypeOwner::Core) && !matches!(to_owner, TypeOwner::Core) {
            return Err(ConverterRegistrationError::ForeignToForeign {
                from: definition.from.clone(),
                to: definition.to,
                owner: "core".to_owned(),
            });
        }
        let key = self.insert_entry(definition, TypeOwner::Core)?;
        #[expect(clippy::expect_used, reason = "`insert_entry` just inserted this key")]
        let state = self.entries.get_mut(&key).expect("entry was inserted");
        state.providers.insert(ConverterProvider::Core);
        state.new_binding_default = Some(ConverterProvider::Core);
        Ok(key)
    }

    fn insert_entry(
        &mut self,
        mut definition: ConverterDefinition,
        owner: TypeOwner,
    ) -> Result<ConverterKey, ConverterRegistrationError> {
        self.validate_definition_fields(&mut definition)?;
        let key = definition.key();
        if let Some(existing) = self.entries.get(&key) {
            if existing.definition != definition || existing.owner != owner {
                return Err(ConverterRegistrationError::ConflictingEntry { entry: key });
            }
            return Ok(key);
        }
        self.entries.insert(
            key.clone(),
            EntryState {
                definition,
                owner,
                providers: BTreeSet::new(),
                new_binding_default: None,
            },
        );
        Ok(key)
    }

    fn validate_definition(
        &self,
        definition: &ConverterDefinition,
    ) -> Result<(TypeOwner, TypeOwner), ConverterRegistrationError> {
        self.validate_definition_fields(&mut definition.clone())?;
        let from_term = TypeTerm::parse(&definition.from)?;
        let to_term = TypeTerm::parse(&definition.to)?;
        let from_owner = self.types.owner_of(&from_term)?.clone();
        let to_owner = self.types.owner_of(&to_term)?.clone();
        Ok((from_owner, to_owner))
    }

    fn validate_definition_fields(
        &self,
        definition: &mut ConverterDefinition,
    ) -> Result<(), ConverterRegistrationError> {
        if definition.contract_version == 0 {
            return Err(ConverterRegistrationError::InvalidContractVersion);
        }
        let from_term = TypeTerm::parse(&definition.from)?;
        let to_term = TypeTerm::parse(&definition.to)?;
        self.types.validate(&from_term)?;
        self.types.validate(&to_term)?;

        if definition.class == ConversionClass::LosslessTotal && !definition.errors.is_empty() {
            return Err(ConverterRegistrationError::LosslessHasErrors {
                entry: definition.key(),
            });
        }
        if definition.class == ConversionClass::CheckedPartial && definition.errors.is_empty() {
            return Err(ConverterRegistrationError::PartialNeedsErrors {
                entry: definition.key(),
            });
        }

        definition.errors.sort();
        for error in &definition.errors {
            error.validate()?;
        }
        if definition.errors.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(ConverterRegistrationError::DuplicateErrorCode {
                entry: definition.key(),
            });
        }

        definition
            .policies
            .sort_by(|left, right| left.name.cmp(&right.name));
        let mut names = BTreeSet::new();
        for policy in &mut definition.policies {
            if policy.name.trim().is_empty()
                || policy.name.trim() != policy.name
                || policy.name.chars().any(char::is_whitespace)
            {
                return Err(ConverterRegistrationError::InvalidPolicyName {
                    name: policy.name.clone(),
                });
            }
            if !names.insert(policy.name.clone()) {
                return Err(ConverterRegistrationError::DuplicatePolicy {
                    name: policy.name.clone(),
                });
            }
            policy.choices.sort();
            if policy.choices.is_empty()
                || policy
                    .choices
                    .iter()
                    .any(|choice| choice.trim().is_empty() || choice.trim() != choice)
                || policy.choices.windows(2).any(|pair| pair[0] == pair[1])
            {
                return Err(ConverterRegistrationError::InvalidPolicyChoices {
                    name: policy.name.clone(),
                });
            }
            if policy
                .default
                .as_ref()
                .is_some_and(|default| !policy.choices.contains(default))
            {
                return Err(ConverterRegistrationError::InvalidPolicyDefault {
                    name: policy.name.clone(),
                });
            }
        }
        Ok(())
    }
}

fn core_definition(
    id: &str,
    from: &str,
    to: &str,
    class: ConversionClass,
    policies: Vec<super::NamedPolicy>,
    errors: Vec<ConverterErrorCode>,
) -> Result<ConverterDefinition, ConverterRegistrationError> {
    Ok(ConverterDefinition {
        id: id.parse()?,
        contract_version: 1,
        from: from.parse()?,
        to: to.parse()?,
        class,
        policies,
        errors,
    })
}

/// Immutable converter registry snapshot bound to one frozen type registry.
#[derive(Debug)]
pub struct ConverterRegistry {
    generation: u64,
    types: Arc<TypeRegistry>,
    entries: BTreeMap<ConverterKey, EntryState>,
    pairs: BTreeMap<(TypeExpr, TypeExpr), Vec<ConverterKey>>,
}

impl ConverterRegistry {
    /// Process-unique identity of this published snapshot. Plans record it so that one made
    /// against a replaced registry (and its frozen type registry) is recognized as stale.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn type_registry(&self) -> &Arc<TypeRegistry> {
        &self.types
    }

    pub fn entry(&self, key: &ConverterKey) -> Option<&ConverterDefinition> {
        self.entries.get(key).map(|state| &state.definition)
    }

    pub fn owner(&self, key: &ConverterKey) -> Option<&TypeOwner> {
        self.entries.get(key).map(|state| &state.owner)
    }

    pub fn entries_for_pair(
        &self,
        from: &TypeExpr,
        to: &TypeExpr,
    ) -> impl Iterator<Item = (&ConverterKey, &ConverterDefinition)> {
        self.pairs
            .get(&(from.clone(), to.clone()))
            .into_iter()
            .flatten()
            .filter_map(|key| self.entries.get_key_value(key))
            .map(|(key, state)| (key, &state.definition))
    }

    pub fn providers(
        &self,
        key: &ConverterKey,
    ) -> Option<impl Iterator<Item = &ConverterProvider>> {
        self.entries.get(key).map(|state| state.providers.iter())
    }

    pub fn new_binding_default(&self, key: &ConverterKey) -> Option<&ConverterProvider> {
        self.entries
            .get(key)
            .and_then(|state| state.new_binding_default.as_ref())
    }
}
