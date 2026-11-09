//! Package nodes as graph kernels (K02c): a verified archive's component becomes the
//! exact provider of that package's pure nodes, composed with the host's resolver.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use orange_compiler::{
    BindingRequest, EntropyBinding, EntropyKernelResolver, KernelBinding, KernelFault, PureKernel,
    PureKernelResolver, ResolveError,
};
use orange_model::Value;
use orange_model::catalog::{ProviderKey, RegistrySnapshot};
use orange_model::document::{NodeKind, PortDirection, PortKind};
use orange_model::id::{PortKey, TypeExpr};
use orange_packages::{Execution, MemorySink, VerifiedArchive};
use thiserror::Error;

use crate::{Grants, GuestComponent, GuestEngine, GuestError, GuestPlugin};

/// Worlds of this major version are loadable; the host offers 1.0 (`API_VERSION`).
pub const PLUGIN_WORLD_PREFIX: &str = "orange:expansion/plugin@1.";
/// Distinct guest error codes and keys the process keeps; later ones become
/// `core/PackageFault` (kernel faults carry static text, so interned text is never freed).
pub const MAX_INTERNED_FAULT_TEXT: usize = 4096;
/// Longest guest error code or message key that is interned.
pub const MAX_FAULT_TEXT_BYTES: usize = 128;

const PACKAGE_FAULT: &str = "core/PackageFault";
const NODE_ERROR_MESSAGE: &str = "A package node reported an error.";
const GUEST_FAULT_MESSAGE: &str = "A package node failed inside its guest.";

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GuestLoadError {
    #[error("the package has no wasm-component execution section")]
    NotAComponent,
    #[error("the package implements an unsupported world")]
    World,
    #[error("the package's component entry is missing from its files")]
    MissingEntry,
    #[error("the package declares no provider with that ID")]
    Provider,
    #[error("the package component could not be prepared")]
    Guest(GuestError),
}

/// A loaded component and the exact provider whose nodes it implements.
#[derive(Debug, Clone)]
pub struct GuestPackage {
    provider: ProviderKey,
    component: GuestComponent,
    grants: Grants,
}

impl GuestPackage {
    /// Prepares the component of a verified archive whose files `files` received.
    /// `provider_id` names one of the manifest's provider contributions; `grants` are the
    /// host capabilities the user approved for it.
    pub fn from_archive(
        engine: &GuestEngine,
        archive: &VerifiedArchive,
        files: &MemorySink,
        provider_id: &str,
        grants: Grants,
    ) -> Result<Self, GuestLoadError> {
        let manifest = archive.manifest();
        let Some(Execution::WasmComponent { entry, world }) = manifest.execution() else {
            return Err(GuestLoadError::NotAComponent);
        };
        if !world.as_str().starts_with(PLUGIN_WORLD_PREFIX) {
            return Err(GuestLoadError::World);
        }
        if !manifest
            .providers()
            .iter()
            .any(|provider| provider.id == provider_id)
        {
            return Err(GuestLoadError::Provider);
        }
        let bytes = files
            .file(entry.as_str())
            .ok_or(GuestLoadError::MissingEntry)?;
        let component = engine.compile(bytes).map_err(GuestLoadError::Guest)?;
        let key = manifest.key();
        Ok(Self {
            provider: ProviderKey {
                package: key.id.as_str().to_owned(),
                version: key.version.as_str().to_owned(),
                content_digest: archive.content_digest().to_owned(),
                id: provider_id.to_owned(),
            },
            component,
            grants,
        })
    }

    /// The exact provider identity this package's nodes bind to.
    pub fn provider(&self) -> &ProviderKey {
        &self.provider
    }
}

/// Host resolver composition: guest kernels for the exact providers of loaded packages,
/// `host` for every other binding. A binding to a loaded package's provider never falls
/// back to the host resolver.
#[derive(Debug)]
pub struct WithGuests<R> {
    host: R,
    packages: Vec<GuestPackage>,
}

impl<R> WithGuests<R> {
    pub fn new(host: R) -> Self {
        Self {
            host,
            packages: Vec::new(),
        }
    }

    pub fn add(&mut self, package: GuestPackage) {
        self.packages.push(package);
    }

    pub fn host(&self) -> &R {
        &self.host
    }

    fn package(&self, request: &BindingRequest<'_>) -> Option<&GuestPackage> {
        let provider = request.provider()?;
        self.packages
            .iter()
            .find(|package| &package.provider == provider)
    }
}

impl<R: PureKernelResolver> PureKernelResolver for WithGuests<R> {
    fn resolve_binding(&self, request: &BindingRequest<'_>) -> Result<KernelBinding, ResolveError> {
        let Some(package) = self.package(request) else {
            return self.host.resolve_binding(request);
        };
        let provider = request.provider().ok_or(ResolveError::Unavailable)?;
        if !request
            .snapshot()
            .provides(request.definition(), request.config_version(), provider)
        {
            return Err(ResolveError::Unavailable);
        }
        let signature = request.signature();
        if signature.kind != NodeKind::Pure || !request.config().is_empty() {
            return Err(ResolveError::Contract);
        }
        let (mut inputs, mut input_types) = (Vec::new(), Vec::new());
        let (mut outputs, mut output_types) = (Vec::new(), Vec::new());
        for port in &signature.ports {
            if port.kind != PortKind::Data {
                return Err(ResolveError::Contract);
            }
            let ty = port.ty.clone().ok_or(ResolveError::Contract)?;
            if ty.as_str().contains('$') {
                return Err(ResolveError::UnsupportedType);
            }
            let key: PortKey = port
                .port_id
                .as_str()
                .parse()
                .map_err(|_| ResolveError::Contract)?;
            if port.direction == PortDirection::Input {
                inputs.push(key);
                input_types.push(ty);
            } else {
                outputs.push(key);
                output_types.push(ty);
            }
        }
        let plugin = GuestPlugin::new(&package.component, package.grants)
            .map_err(|_| ResolveError::Unavailable)?;
        Ok(KernelBinding {
            inputs: inputs.into(),
            outputs: outputs.into(),
            kernel: Arc::new(GuestKernel {
                plugin: Mutex::new(plugin),
                snapshot: request.snapshot().clone(),
                definition: request.definition().id.as_str().to_owned(),
                inputs: input_types,
                outputs: output_types,
            }),
        })
    }
}

impl<R: EntropyKernelResolver> EntropyKernelResolver for WithGuests<R> {
    fn resolve_entropy_binding(
        &self,
        request: &BindingRequest<'_>,
    ) -> Result<EntropyBinding, ResolveError> {
        // Guest entropy sources are not part of the 1.0 world.
        if self.package(request).is_some() {
            return Err(ResolveError::Unavailable);
        }
        self.host.resolve_entropy_binding(request)
    }
}

/// One node binding's own guest instance. A trap poisons only this binding.
struct GuestKernel {
    plugin: Mutex<GuestPlugin>,
    snapshot: Arc<RegistrySnapshot>,
    definition: String,
    inputs: Vec<TypeExpr>,
    outputs: Vec<TypeExpr>,
}

impl std::fmt::Debug for GuestKernel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GuestKernel")
            .field("definition", &self.definition)
            .finish_non_exhaustive()
    }
}

fn guest_fault(error: GuestError) -> KernelFault {
    KernelFault {
        code: PACKAGE_FAULT,
        message_key: error.message_key(),
        message: GUEST_FAULT_MESSAGE,
    }
}

impl PureKernel for GuestKernel {
    fn fold(&self, inputs: &[Value], outputs: &mut [Value]) -> Result<(), KernelFault> {
        if inputs.len() != self.inputs.len() || outputs.len() != self.outputs.len() {
            return Err(guest_fault(GuestError::InputContract));
        }
        let typed: Vec<(TypeExpr, Value)> = self
            .inputs
            .iter()
            .cloned()
            .zip(inputs.iter().cloned())
            .collect();
        let mut plugin = self
            .plugin
            .lock()
            .map_err(|_| guest_fault(GuestError::Poisoned))?;
        match plugin.evaluate(
            self.snapshot.types().as_ref(),
            &self.definition,
            &typed,
            &self.outputs,
        ) {
            Ok(Ok(values)) => {
                for (slot, value) in outputs.iter_mut().zip(values) {
                    *slot = value;
                }
                Ok(())
            }
            Ok(Err(error)) => Err(match (intern(&error.code), intern(&error.message_key)) {
                (Some(code), Some(message_key)) => KernelFault {
                    code,
                    message_key,
                    message: NODE_ERROR_MESSAGE,
                },
                _ => guest_fault(GuestError::OutputContract),
            }),
            Err(error) => Err(guest_fault(error)),
        }
    }
}

static INTERNED: Mutex<BTreeSet<&'static str>> = Mutex::new(BTreeSet::new());

/// Returns process-lifetime text for a guest error identifier, bounded in count and
/// length so a guest cannot grow host memory without limit.
fn intern(text: &str) -> Option<&'static str> {
    if text.len() > MAX_FAULT_TEXT_BYTES {
        return None;
    }
    let mut interned = INTERNED.lock().ok()?;
    if let Some(existing) = interned.get(text) {
        return Some(existing);
    }
    if interned.len() >= MAX_INTERNED_FAULT_TEXT {
        return None;
    }
    let leaked: &'static str = Box::leak(text.to_owned().into_boxed_str());
    interned.insert(leaked);
    Some(leaked)
}
