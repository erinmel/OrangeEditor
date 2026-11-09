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

    /// Starts a registry with a Core conversion entry for every ordered pair of distinct
    /// numeric scalar types (11 §6): exact widening is lossless-total, integer narrowing
    /// and float-to-integer are checked-partial, and inexact integer-to-float is lossy.
    /// It also has invariant text parse/display entries for bool and those numeric
    /// types, and UTF-8 encode/decode between `string` and `bytes`.
    pub fn with_core_conversions(
        types: Arc<TypeRegistry>,
    ) -> Result<Self, ConverterRegistrationError> {
        let mut builder = Self::new(types);
        for (from, to) in NUMERIC
            .iter()
            .flat_map(|from| NUMERIC.iter().map(move |to| (*from, *to)))
            .filter(|(from, to)| from != to)
        {
            let (class, policies, errors) = numeric_contract(from, to)?;
            builder.register_core_entry(core_definition(
                &format!("core/{from}-to-{to}"),
                &format!("core/{from}@1"),
                &format!("core/{to}@1"),
                class,
                policies,
                errors,
            )?)?;
        }
        for (id, from, to, class, policies, errors) in text_contracts()? {
            builder
                .register_core_entry(core_definition(&id, &from, &to, class, policies, errors)?)?;
        }
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

const NUMERIC: [&str; 6] = ["i32", "i64", "u32", "u64", "f32", "f64"];

type NumericContract = (ConversionClass, Vec<NamedPolicy>, Vec<ConverterErrorCode>);

fn policy(name: &str, choices: &[&str], default: &str) -> NamedPolicy {
    NamedPolicy {
        name: name.to_owned(),
        choices: choices.iter().map(|&choice| choice.to_owned()).collect(),
        default: Some(default.to_owned()),
    }
}

/// Class, policies and errors of one Core numeric entry. Bit widths decide exactness:
/// every i32/u32 value is exact in f64, every 32-bit unsigned value fits i64.
fn numeric_contract(from: &str, to: &str) -> Result<NumericContract, ConverterRegistrationError> {
    let float = |ty: &str| ty.starts_with('f');
    let out_of_range = || ConverterErrorCode::new("core/OutOfRange");
    Ok(match (float(from), float(to)) {
        (false, false) => match (from, to) {
            ("i32", "i64") | ("u32", "i64") | ("u32", "u64") => {
                (ConversionClass::LosslessTotal, vec![], vec![])
            }
            _ => (
                ConversionClass::CheckedPartial,
                vec![],
                vec![out_of_range()?],
            ),
        },
        (false, true) => match (from, to) {
            ("i32" | "u32", "f64") => (ConversionClass::LosslessTotal, vec![], vec![]),
            _ => (
                ConversionClass::Lossy,
                vec![policy("rounding", &["nearest-even"], "nearest-even")],
                vec![],
            ),
        },
        (true, false) => (
            ConversionClass::CheckedPartial,
            vec![policy(
                "rounding",
                &["truncate", "floor", "ceiling", "nearest-even"],
                "truncate",
            )],
            vec![ConverterErrorCode::new("core/NonFinite")?, out_of_range()?],
        ),
        (true, true) if from == "f32" => (ConversionClass::LosslessTotal, vec![], vec![]),
        (true, true) => (
            ConversionClass::Lossy,
            vec![
                policy("rounding", &["nearest-even"], "nearest-even"),
                policy("overflow", &["reject"], "reject"),
            ],
            vec![out_of_range()?],
        ),
    })
}

type TextContract = (
    String,
    String,
    String,
    ConversionClass,
    Vec<NamedPolicy>,
    Vec<ConverterErrorCode>,
);

/// Display is lossless-total except for floats, whose NaN payloads have no text form.
/// Parsing is checked; float parsing rounds to nearest, ties to even.
fn text_contracts() -> Result<Vec<TextContract>, ConverterRegistrationError> {
    let parse = || ConverterErrorCode::new("core/Parse");
    let range = || ConverterErrorCode::new("core/OutOfRange");
    let string = "core/string@1".to_owned();
    let bytes = "core/bytes@1".to_owned();
    let mut contracts = Vec::new();
    for ty in ["bool", "i32", "i64", "u32", "u64", "f32", "f64"] {
        let float = ty.starts_with('f');
        contracts.push((
            format!("core/{ty}-to-string"),
            format!("core/{ty}@1"),
            string.clone(),
            if float {
                ConversionClass::Lossy
            } else {
                ConversionClass::LosslessTotal
            },
            vec![],
            vec![],
        ));
        contracts.push((
            format!("core/string-to-{ty}"),
            string.clone(),
            format!("core/{ty}@1"),
            ConversionClass::CheckedPartial,
            if float {
                vec![policy("rounding", &["nearest-even"], "nearest-even")]
            } else {
                vec![]
            },
            if ty == "bool" {
                vec![parse()?]
            } else {
                vec![parse()?, range()?]
            },
        ));
    }
    contracts.push((
        "core/string-to-bytes".into(),
        string.clone(),
        bytes.clone(),
        ConversionClass::LosslessTotal,
        vec![],
        vec![],
    ));
    contracts.push((
        "core/bytes-to-string".into(),
        bytes.clone(),
        string.clone(),
        ConversionClass::CheckedPartial,
        vec![],
        vec![ConverterErrorCode::new("core/InvalidUtf8")?],
    ));
    contracts.push((
        "core/bytes-to-string-lossy".into(),
        bytes,
        string,
        ConversionClass::Lossy,
        vec![policy("invalid", &["replace"], "replace")],
        vec![],
    ));
    Ok(contracts)
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
