//! Package nodes as graph kernels (K02c): a verified archive's component becomes the
//! exact provider of that package's pure, action and async nodes, composed with the
//! host's resolvers.

use std::sync::{Arc, Mutex};

use orange_kernel::{
    ActionKernel, BindingRequest, EffectBinding, EffectResolver, EntropyBinding,
    EntropyKernelResolver, KernelBinding, KernelFault, PureKernel, PureKernelResolver,
    ResolveError,
};
use orange_model::Value;
use orange_model::catalog::ReferenceKind;
use orange_model::catalog::{ProviderKey, RegistrySnapshot};
use orange_model::document::{NodeKind, PortDirection, PortKind};
use orange_model::id::{ConnectionId, PortKey, TypeExpr};
use orange_packages::{Execution, MemorySink, VerifiedArchive};
use thiserror::Error;

use crate::{
    Grants, GuestAccess, GuestCancellation, GuestComponent, GuestEngine, GuestError, GuestPlugin,
};

/// Worlds of this major version are loadable; the host offers 1.1 (`API_VERSION`).
pub const PLUGIN_WORLD_PREFIX: &str = "orange:expansion/plugin@1.";
/// The 1.1 world of a package that also contributes trigger nodes.
pub const PLUGIN_EVENTS_WORLD_PREFIX: &str = "orange:expansion/plugin-events@1.";
/// Longest guest error code or message key retained in a fault.
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
    #[error("the component bytes do not match their verified archive entry")]
    Integrity,
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
    access: GuestAccess,
}

impl GuestPackage {
    /// Replaces the grants of instances made from now on, so a revoked grant is not
    /// linked into them. Instances already made keep theirs.
    #[must_use]
    pub fn with_grants(mut self, grants: Grants) -> Self {
        self.grants = grants;
        self
    }

    /// Sets what the package's instances reach with their `files` and `http` grants.
    #[must_use]
    pub fn with_access(mut self, access: GuestAccess) -> Self {
        self.access = access;
        self
    }

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
        if !world.as_str().starts_with(PLUGIN_WORLD_PREFIX)
            && !world.as_str().starts_with(PLUGIN_EVENTS_WORLD_PREFIX)
        {
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
        archive
            .verify_file(entry, bytes)
            .map_err(|_| GuestLoadError::Integrity)?;
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
            access: GuestAccess::default(),
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
        if request.signature().kind != NodeKind::Pure {
            return Err(ResolveError::Contract);
        }
        let (inputs, outputs, node) = package.node(request, false)?;
        Ok(KernelBinding {
            inputs,
            outputs,
            kernel: Arc::new(GuestKernel(node)),
        })
    }
}

impl<R: EffectResolver> EffectResolver for WithGuests<R> {
    /// An action node runs inline through its kernel. An async node is submitted to the
    /// host, which finds its implementation with [`guest_effect`] on the request context.
    fn resolve_effect(&self, request: &BindingRequest<'_>) -> Result<EffectBinding, ResolveError> {
        let Some(package) = self.package(request) else {
            return self.host.resolve_effect(request);
        };
        let kind = request.signature().kind;
        if !matches!(kind, NodeKind::Action | NodeKind::Async) {
            return Err(ResolveError::Contract);
        }
        let (inputs, outputs, node) = package.node(request, true)?;
        let effect = Arc::new(GuestEffect(node));
        Ok(EffectBinding {
            route: 0,
            inputs,
            outputs,
            action: (kind == NodeKind::Action)
                .then(|| Arc::new(GuestAction(effect.clone())) as Arc<dyn ActionKernel>),
            context: effect,
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

/// A node binding: its input and output data ports in declared order, and its instance.
type BoundNode = (Box<[PortKey]>, Box<[PortKey]>, GuestNode);

impl GuestPackage {
    /// One node binding's own guest instance and its data ports in declared order.
    /// `control` admits the exec ports of an action or async node, which carry no value.
    fn node(&self, request: &BindingRequest<'_>, control: bool) -> Result<BoundNode, ResolveError> {
        let provider = request.provider().ok_or(ResolveError::Unavailable)?;
        if !request
            .snapshot()
            .provides(request.definition(), request.config_version(), provider)
        {
            return Err(ResolveError::Unavailable);
        }
        // Settings reach the instance as JSON (1.1); a connection they name is opened by
        // the host, which hands its session to the node's effect calls.
        let settings =
            serde_json::to_string(request.config()).map_err(|_| ResolveError::Contract)?;
        let connection = request
            .snapshot()
            .definition(request.definition())
            .filter(|definition| {
                definition
                    .references
                    .iter()
                    .any(|field| field.kind == ReferenceKind::Connection)
            })
            .and_then(|_| request.config().get("connection")?.as_str()?.parse().ok());
        let (mut inputs, mut input_types) = (Vec::new(), Vec::new());
        let (mut outputs, mut output_types) = (Vec::new(), Vec::new());
        for port in &request.signature().ports {
            match port.kind {
                PortKind::Data => {}
                PortKind::Exec if control => continue,
                _ => return Err(ResolveError::Contract),
            }
            if port.variadic.is_some() {
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
        let access = if request.signature().kind == NodeKind::Async {
            self.access.clone()
        } else {
            GuestAccess::default()
        };
        let mut plugin = GuestPlugin::with_access(&self.component, self.grants, access)
            .map_err(|_| ResolveError::Unavailable)?;
        plugin.set_settings(settings);
        let cancellation = plugin.cancellation();
        Ok((
            inputs.into(),
            outputs.into(),
            GuestNode {
                plugin: Mutex::new(plugin),
                cancellation,
                snapshot: request.snapshot().clone(),
                definition: request.definition().id.as_str().to_owned(),
                inputs: input_types,
                outputs: output_types,
                connection,
                definition_ref: request.definition().clone(),
            },
        ))
    }
}

/// One node binding's own guest instance. A trap poisons only this binding.
struct GuestNode {
    plugin: Mutex<GuestPlugin>,
    cancellation: GuestCancellation,
    snapshot: Arc<RegistrySnapshot>,
    definition: String,
    inputs: Vec<TypeExpr>,
    outputs: Vec<TypeExpr>,
    /// The connection the node's settings name, when its definition refers to one.
    connection: Option<ConnectionId>,
    definition_ref: orange_model::document::DefinitionRef,
}

impl std::fmt::Debug for GuestNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GuestNode")
            .field("definition", &self.definition)
            .finish_non_exhaustive()
    }
}

fn guest_fault(error: GuestError) -> KernelFault {
    KernelFault {
        code: PACKAGE_FAULT.into(),
        message_key: error.message_key().into(),
        message: GUEST_FAULT_MESSAGE,
    }
}

impl GuestNode {
    fn call(&self, effect: bool, inputs: &[Value]) -> Result<Vec<Value>, KernelFault> {
        if inputs.len() != self.inputs.len() {
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
        let types = self.snapshot.types();
        let result = if effect {
            plugin.execute(types.as_ref(), &self.definition, &typed, &self.outputs)
        } else {
            plugin.evaluate(types.as_ref(), &self.definition, &typed, &self.outputs)
        };
        match result {
            Ok(Ok(values)) => Ok(values),
            Ok(Err(error)) => Err(node_fault(error)),
            Err(error) => Err(guest_fault(error)),
        }
    }
    fn call_into(
        &self,
        effect: bool,
        inputs: &[Value],
        outputs: &mut [Value],
    ) -> Result<(), KernelFault> {
        if outputs.len() != self.outputs.len() {
            return Err(guest_fault(GuestError::InputContract));
        }
        for (slot, value) in outputs.iter_mut().zip(self.call(effect, inputs)?) {
            *slot = value;
        }
        Ok(())
    }
}

#[derive(Debug)]
struct GuestKernel(GuestNode);
impl PureKernel for GuestKernel {
    fn fold(&self, inputs: &[Value], outputs: &mut [Value]) -> Result<(), KernelFault> {
        self.0.call_into(false, inputs, outputs)
    }
}

/// The guest implementation of one action or async node. For an async node the host
/// receives it as the context of that node's effect requests (see [`guest_effect`])
/// and runs it off the run's thread. Every request of one node in one compiled plan
/// reaches the same instance, so the guest may keep state; calls are serialized.
#[derive(Debug)]
pub struct GuestEffect(GuestNode);
impl GuestEffect {
    /// Runs the node once on `inputs` in declared port order. The result is what the
    /// host passes to `Run::complete_effect`.
    pub fn run(&self, inputs: &[Value]) -> Result<Vec<Value>, KernelFault> {
        self.0.call(true, inputs)
    }

    /// Runs the node once like [`run`](Self::run), with the host session of the
    /// connection it names for its `websocket` import during this call only.
    pub fn run_on(
        &self,
        inputs: &[Value],
        session: Option<Arc<dyn orange_sdk::Session>>,
    ) -> Result<Vec<Value>, KernelFault> {
        self.0
            .plugin
            .lock()
            .map_err(|_| guest_fault(GuestError::Poisoned))?
            .set_session(session);
        let result = self.0.call(true, inputs);
        if let Ok(mut plugin) = self.0.plugin.lock() {
            plugin.set_session(None);
        }
        result
    }

    /// The connection the node's settings name, which the host opens for it.
    pub fn connection(&self) -> Option<ConnectionId> {
        self.0.connection
    }

    /// The node's definition, for the host's connection policy.
    pub fn definition(&self) -> &orange_model::document::DefinitionRef {
        &self.0.definition_ref
    }
    /// Interrupts this node's instance, for an effect whose lease was cancelled.
    /// Cancellation is terminal for the instance.
    pub fn cancellation(&self) -> GuestCancellation {
        self.0.cancellation.clone()
    }
}

/// The guest implementation behind an effect request's `context`, if a loaded package
/// provides that node. Other contexts belong to the host's own effect adapters.
pub fn guest_effect(context: &Arc<dyn std::any::Any + Send + Sync>) -> Option<Arc<GuestEffect>> {
    context.clone().downcast().ok()
}

#[derive(Debug)]
struct GuestAction(Arc<GuestEffect>);
impl ActionKernel for GuestAction {
    fn execute(&self, inputs: &[Value], outputs: &mut [Value]) -> Result<(), KernelFault> {
        self.0.0.call_into(true, inputs, outputs)
    }
}

fn node_fault(error: crate::GuestNodeError) -> KernelFault {
    if error.code.len() > MAX_FAULT_TEXT_BYTES || error.message_key.len() > MAX_FAULT_TEXT_BYTES {
        return guest_fault(GuestError::OutputContract);
    }
    let fault = KernelFault {
        code: error.code.into(),
        message_key: error.message_key.into(),
        message: NODE_ERROR_MESSAGE,
    };
    if fault.is_valid() {
        fault
    } else {
        guest_fault(GuestError::OutputContract)
    }
}

/// A package trigger (entry) node (1.1): started by the host for each such node of a
/// running flow, on the session of the connection the node's settings name. Every
/// message that arrives there goes to the package's `events.receive` on this node's own
/// instance, whose answer fires the node, lets the message pass or reports a fault. The
/// host keeps the connection, its credentials and reconnection; the instance has no
/// file, network or connection access while it decides.
pub struct GuestSource {
    component: GuestComponent,
    grants: Grants,
    definition: String,
    outputs: Vec<TypeExpr>,
    snapshot: Arc<RegistrySnapshot>,
}

impl std::fmt::Debug for GuestSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GuestSource")
            .field("definition", &self.definition)
            .finish_non_exhaustive()
    }
}

impl GuestPackage {
    /// The trigger source for entry node `definition` of this package, when the user
    /// granted it `websocket`: an entry has at most one data output (`payload`) of a
    /// concrete type. `None` for anything else.
    pub fn trigger(
        &self,
        definition: &orange_model::catalog::NodeDefinition,
        snapshot: Arc<RegistrySnapshot>,
    ) -> Option<GuestSource> {
        if definition.kind != NodeKind::Entry || !self.grants.websocket {
            return None;
        }
        let orange_model::catalog::Ports::Declared(ports) = &definition.ports else {
            return None;
        };
        let outputs: Vec<TypeExpr> = ports
            .iter()
            .filter(|port| port.direction == PortDirection::Output && port.kind == PortKind::Data)
            .map(|port| port.ty.clone())
            .collect::<Option<_>>()?;
        if outputs.len() > 1 || outputs.iter().any(|ty| ty.as_str().contains('$')) {
            return None;
        }
        Some(GuestSource {
            component: self.component.clone(),
            grants: self.grants,
            definition: definition.definition.id.as_str().to_owned(),
            outputs,
            snapshot,
        })
    }
}

impl orange_sdk::ServiceSource for GuestSource {
    fn start_with(
        &self,
        session: Option<Arc<dyn orange_sdk::Session>>,
        settings: &serde_json::Map<String, serde_json::Value>,
        emitter: orange_sdk::Emitter,
    ) -> Result<Box<dyn Send>, String> {
        let session = session.ok_or("the trigger names no connection")?;
        // The deciding instance gets the node's settings and nothing to reach.
        let mut plugin =
            GuestPlugin::with_access(&self.component, self.grants, GuestAccess::default())
                .map_err(|error| error.to_string())?;
        plugin.set_settings(
            serde_json::to_string(settings).map_err(|_| "the settings are not JSON".to_owned())?,
        );
        if !plugin.receives_events() {
            return Err("the package does not receive events".to_owned());
        }
        let plugin = Mutex::new(plugin);
        let (snapshot, definition, outputs) = (
            Arc::clone(&self.snapshot),
            self.definition.clone(),
            self.outputs.clone(),
        );
        let incoming = orange_sdk::Emitter::new(move |payload| {
            let message = match payload {
                Some(Value::Str(text)) => orange_sdk::Message::Text(Arc::from(text.as_str())),
                Some(Value::Bytes(data)) => orange_sdk::Message::Binary(Arc::from(data.as_slice())),
                _ => return Ok(()),
            };
            let mut plugin = plugin.lock().map_err(|_| orange_sdk::EmitError::Closed)?;
            match plugin.receive(snapshot.types(), &definition, &message, &outputs) {
                Ok(Ok(Some(mut values))) => emitter.emit(values.pop()),
                Ok(Ok(None)) => Ok(()),
                Ok(Err(error)) => emitter.fault(node_fault(error)),
                Err(error) => {
                    emitter.fault(guest_fault(error))?;
                    // A trapped instance decides nothing more: stop listening.
                    if plugin.is_poisoned() {
                        Err(orange_sdk::EmitError::Closed)
                    } else {
                        Ok(())
                    }
                }
            }
        });
        session
            .listen_messages(incoming)
            .map_err(|error| error.to_string())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use orange_kernel::FaultText;

    fn error(code: String, message_key: String) -> crate::GuestNodeError {
        crate::GuestNodeError { code, message_key }
    }

    #[test]
    fn unrelated_error_churn_cannot_change_identifiers_or_keep_text_alive() {
        let original = node_fault(error(
            "org.example/Overflow".into(),
            "org.example/overflow".into(),
        ));
        let FaultText::Shared(code) = &original.code else {
            panic!("guest text must be owned")
        };
        let weak = Arc::downgrade(code);
        let retained = original.clone();
        drop(original);
        for i in 0..5000 {
            let other = node_fault(error(
                format!("org.other/Error{i}"),
                format!("org.other/error-{i}"),
            ));
            assert_eq!(other.code, format!("org.other/Error{i}"));
        }
        let again = node_fault(error(
            "org.example/Overflow".into(),
            "org.example/overflow".into(),
        ));
        assert_eq!(retained, again);
        assert!(weak.upgrade().is_some());
        drop(retained);
        assert!(
            weak.upgrade().is_none(),
            "no global owner retains the old text"
        );
    }

    #[test]
    fn identifier_limits_remain_local_and_validated() {
        for (code, key) in [
            ("not-namespaced".into(), "org.example/error".into()),
            ("org.example/Bad/Code".into(), "org.example/error".into()),
            (
                format!("org.example/{}", "a".repeat(128)),
                "org.example/error".into(),
            ),
            (
                "org.example/Bad".into(),
                format!("org.example/{}", "a".repeat(128)),
            ),
        ] {
            let fault = node_fault(error(code, key));
            assert_eq!(fault.code, "core/PackageFault");
            assert_eq!(fault.message_key, "core/guest-output-contract");
        }
        let exact = format!(
            "org.example/{}",
            "a".repeat(MAX_FAULT_TEXT_BYTES - "org.example/".len())
        );
        assert_eq!(node_fault(error(exact.clone(), exact.clone())).code, exact);
    }
}
