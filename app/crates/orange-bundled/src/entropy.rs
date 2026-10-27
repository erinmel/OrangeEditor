use crate::{BindingError, CatalogError, MathPackage};
use orange_compiler::{
    BindingRequest, EntropyBinding, EntropyKernel, EntropyKernelResolver, EntropySource,
    KernelFault, ResolveError,
};
use orange_model::catalog::{
    Determinism, NodeDefinition, Ports, ProviderKey, RegistrySnapshot, TypeArgs, Volatility,
};
use orange_model::document::{
    ConfigValue, DefinitionRef, NodeInstance, NodeKind, OutputSource, PortDescriptor,
    SignatureSnapshot,
};
use orange_model::{GraphDocument, Value};
use std::sync::Arc;

pub const RANDOM_U64_DEFINITION: &str = "org.orange.math/random-u64";

/// Full-width raw sample from the host stream; no distribution/range or OS source.
pub fn random_u64_definition() -> Result<NodeDefinition, CatalogError> {
    Ok(NodeDefinition {
        definition: DefinitionRef {
            id: RANDOM_U64_DEFINITION.parse()?,
            version: 1,
        },
        config_version: 1,
        kind: NodeKind::Volatile,
        volatility: Some(Volatility::Entropy),
        determinism: Determinism::Seeded,
        suspends: None,
        type_params: vec![],
        ports: Ports::Declared(vec![PortDescriptor::data_out(
            "value",
            "core/u64@1".parse()?,
            OutputSource::Computed,
        )]),
        references: vec![],
        specializations: None,
        config_schema: Some(serde_json::json!({"$schema":"orange.config-schema/v1",
            "type":"object","properties":{},"additionalProperties":false})),
    })
}

/// The fresh signature a declared-port definition projects without type arguments.
fn declared_signature(definition: &NodeDefinition) -> Option<SignatureSnapshot> {
    match &definition.ports {
        Ports::Declared(ports) => Some(orange_model::signature::project_with_suspension(
            definition.kind,
            definition.suspends,
            ports.clone(),
            &Default::default(),
        )),
        _ => None,
    }
}

/// Resolved native proof retaining the exact provider metadata. It is not graph freshness.
#[derive(Debug, Clone)]
pub struct ResolvedEntropyKernel {
    snapshot: Arc<RegistrySnapshot>,
    provider: ProviderKey,
}
impl ResolvedEntropyKernel {
    pub fn snapshot(&self) -> &Arc<RegistrySnapshot> {
        &self.snapshot
    }
    pub fn provider(&self) -> &ProviderKey {
        &self.provider
    }
    pub fn evaluate(&self, entropy: &mut dyn EntropySource) -> Result<u64, KernelFault> {
        entropy.next_u64()
    }
}
impl MathPackage {
    pub fn bind_entropy(
        &self,
        snapshot: Arc<RegistrySnapshot>,
        provider: &ProviderKey,
        definition: &DefinitionRef,
        config_version: u32,
        config: &ConfigValue,
    ) -> Result<ResolvedEntropyKernel, BindingError> {
        if provider != self.provider_key()
            || definition.id.as_str() != RANDOM_U64_DEFINITION
            || definition.version != 1
            || config_version != 1
            || !snapshot.provides(definition, config_version, provider)
        {
            return Err(BindingError::Unavailable);
        }
        if !config.is_empty() {
            return Err(BindingError::InvalidConfig);
        }
        let expected = random_u64_definition().map_err(|_| BindingError::Unavailable)?;
        let current = snapshot
            .definition(definition)
            .ok_or(BindingError::Unavailable)?;
        if current.kind != NodeKind::Volatile
            || current.config_version != 1
            || current.volatility != Some(Volatility::Entropy)
            || current.determinism != Determinism::Seeded
            || !current.type_params.is_empty()
            || !current.references.is_empty()
            || declared_signature(current) != declared_signature(&expected)
            || snapshot
                .provider(provider)
                .and_then(|entry| entry.specializations.get(definition))
                .is_none_or(|sets| !sets.contains(&TypeArgs::new()))
        {
            return Err(BindingError::Unavailable);
        }
        Ok(ResolvedEntropyKernel {
            snapshot,
            provider: provider.clone(),
        })
    }
}
impl EntropyKernelResolver for MathPackage {
    fn resolve_entropy(
        &self,
        document: &GraphDocument,
        node: &NodeInstance,
        signature: &SignatureSnapshot,
        snapshot: Arc<RegistrySnapshot>,
    ) -> Result<EntropyBinding, ResolveError> {
        self.resolve_entropy_binding(&BindingRequest::new(document, node, signature, snapshot)?)
    }

    fn resolve_entropy_binding(
        &self,
        request: &BindingRequest<'_>,
    ) -> Result<EntropyBinding, ResolveError> {
        let provider = request.provider().ok_or(ResolveError::Unavailable)?;
        let snapshot = request.snapshot();
        let kernel = self
            .bind_entropy(
                snapshot.clone(),
                provider,
                request.definition(),
                request.config_version(),
                request.config(),
            )
            .map_err(|error| {
                if error == BindingError::InvalidConfig {
                    ResolveError::Contract
                } else {
                    ResolveError::Unavailable
                }
            })?;
        // The bound definition has declared ports, so this is its registered contract.
        let expected = snapshot
            .definition(request.definition())
            .and_then(declared_signature)
            .ok_or(ResolveError::Contract)?;
        if request.signature() != &expected {
            return Err(ResolveError::Contract);
        }
        Ok(EntropyBinding {
            inputs: Box::new([]),
            outputs: vec!["value".parse().map_err(|_| ResolveError::Contract)?].into(),
            kernel: Arc::new(kernel),
        })
    }
}
impl EntropyKernel for ResolvedEntropyKernel {
    fn evaluate(
        &self,
        inputs: &[Value],
        outputs: &mut [Value],
        entropy: &mut dyn EntropySource,
    ) -> Result<(), KernelFault> {
        const BAD: KernelFault = KernelFault {
            code: "core/InternalFault",
            message_key: "core/kernel-input-contract",
            message: "Kernel inputs violate the resolved contract.",
        };
        if !inputs.is_empty() {
            return Err(BAD);
        }
        let [output] = outputs else {
            return Err(BAD);
        };
        let value = self.evaluate(entropy)?;
        *output = Value::U64(value);
        Ok(())
    }
}
