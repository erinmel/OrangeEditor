//! Registry snapshots assembled from definition contracts and provider facts.

use std::collections::BTreeMap;
use std::sync::Arc;

use orange_model::catalog::{
    NodeContract, NodeDefinition, ProviderDescriptor, ProviderKey, RegistryBuilder,
    RegistrySnapshot, TypeArgs,
};
use orange_model::document::DefinitionRef;
use orange_model::validation::HostOrigin;

/// Definition contracts and `(definition, config version, provider)` availability, frozen
/// into one core registry snapshot by [`Facts::snapshot`].
#[derive(Debug, Clone)]
pub struct Facts {
    origin: HostOrigin,
    definitions: Vec<(DefinitionRef, NodeContract)>,
    providers: Vec<(DefinitionRef, u32, ProviderKey)>,
    specializations: Vec<(DefinitionRef, Option<ProviderKey>, Vec<TypeArgs>)>,
}

impl Facts {
    pub fn new(origin: HostOrigin) -> Self {
        Self {
            origin,
            definitions: Vec::new(),
            providers: Vec::new(),
            specializations: Vec::new(),
        }
    }

    pub fn definition(&mut self, definition: DefinitionRef, contract: NodeContract) {
        self.definitions.push((definition, contract));
    }

    pub fn provider(&mut self, definition: DefinitionRef, config_version: u32, key: ProviderKey) {
        self.providers.push((definition, config_version, key));
    }

    /// Limits one exact provider (`None` = core) of `definition` to the listed argument sets.
    pub fn specializations(
        &mut self,
        definition: DefinitionRef,
        provider: Option<ProviderKey>,
        supported: Vec<TypeArgs>,
    ) {
        self.specializations.push((definition, provider, supported));
    }

    /// Providers sharing a key are merged; a later definition does not replace an earlier one.
    pub fn snapshot(&self) -> Arc<RegistrySnapshot> {
        let mut builder = RegistryBuilder::core(self.origin.clone());
        for (definition, contract) in self.definitions.iter().cloned() {
            let core = self
                .specializations
                .iter()
                .find(|(named, provider, _)| *named == definition && provider.is_none())
                .map(|(_, _, supported)| supported.clone());
            builder = builder.definition(NodeDefinition {
                definition,
                config_version: 1,
                contract,
                specializations: core,
                config_schema: None,
            });
        }
        let mut merged: BTreeMap<ProviderKey, ProviderDescriptor> = BTreeMap::new();
        for (definition, config_version, key) in self.providers.iter().cloned() {
            merged
                .entry(key.clone())
                .or_insert_with(|| ProviderDescriptor {
                    key,
                    implements: Default::default(),
                    specializations: Default::default(),
                })
                .implements
                .insert((definition, config_version));
        }
        for (definition, key, supported) in self.specializations.iter().cloned() {
            if let Some(key) = key {
                merged
                    .entry(key.clone())
                    .or_insert_with(|| ProviderDescriptor {
                        key,
                        implements: Default::default(),
                        specializations: Default::default(),
                    })
                    .specializations
                    .insert(definition, supported);
            }
        }
        for provider in merged.into_values() {
            builder = builder.provider(provider);
        }
        builder.build().unwrap()
    }
}
