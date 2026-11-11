//! The session an editor, a terminal or a service attaches to.
//!
//! A [`Host`] owns what no single graph does: the installed packages and their registry,
//! the bundled native kernels, a pool of effect workers, and every running graph. Each
//! opened document becomes a *flow*: its own run on its own thread, driven against real
//! time. Flows are independent, so pausing or faulting one never holds another.
//!
//! All control goes through the host by [`FlowId`] and never blocks on a run. What the
//! flows do comes back on one bounded channel of [`HostEvent`]s. This crate has no UI,
//! window or network dependency; a front end is one consumer of this API among several.
// Product code reports failures as typed errors; each remaining panic is a documented
// internal invariant (10, "Panic policy").
#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]

mod effects;
mod flow;
mod graphs;
mod services;

use effects::Pool;
use flow::{Context, Query, Shared, Sink};
use graphs::{GraphNode, Graphs};
use orange_bundled::MathPackage;
use orange_compiler::{CompileError, CompileOptions, compile_control_with_effects};
use orange_guest::{GuestEngine, GuestLoadError, GuestPackage, WithGuests};
use orange_kernel::{
    BindingRequest, EffectBinding, EffectResolver, EntropyBinding, EntropyKernelResolver,
    KernelBinding, PureKernelResolver, ResolveError,
};
use orange_model::catalog::{
    NodeDefinition, ProviderDescriptor, ProviderKey, RegistryBuilder, RegistrySnapshot,
};
use orange_model::document::ConnectionScope;
use orange_model::validation::{Diagnostic, HostOrigin};
use orange_model::{GraphDocument, analyze, from_json, to_json};
use orange_packages::{
    ArchiveFiles, ArchiveLimits, ArchivePath, DiscardSink, Execution, HOST_API_VERSION,
    MemoryJournal, MemorySink, PackageStore, ProviderSlot, read_archive,
};
use orange_runtime::{HostInput, InboxError, Run, RunDriver, RunEventSource, SourceOptions};
use services::Services;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, channel, sync_channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub use graphs::{GRAPH_NODE_FORMAT, GRAPH_PROVIDER};
pub use orange_guest::{Grants, GuestLimits};
pub use orange_kernel::{FaultText, KernelFault};
pub use orange_model::Value;
pub use orange_model::id::ConnectionId;
pub use orange_model::id::{EventDeclId, GraphId, NodeId, VariableId};
pub use orange_packages::PackageKey;
pub use orange_runtime::EffectLease;
pub use orange_runtime::{
    ControlOutcome, RefusedReason, RunError, RunEvent, RunOptions, RunState, StopMode,
};
pub use services::{
    Call, ConnectionAdapter, SERVICE_PROVIDER, ServiceNode, ServicePackage, Session,
};

/// Identifies one flow for the life of its host. Never reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FlowId(u64);
impl std::fmt::Display for FlowId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Something a flow did, in the order its thread observed it.
#[derive(Debug, Clone)]
pub struct HostEvent {
    pub flow: FlowId,
    pub kind: FlowEvent,
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum FlowEvent {
    /// The run entered this state.
    State(RunState),
    /// A control node finished a step.
    Node(RunEvent),
    /// This many node steps happened between two observations and were not kept.
    TraceLost(u64),
    /// The run did not take an input sent to it.
    Refused(RefusedReason),
    /// The run reported an error while being driven.
    Fault(RunError),
    /// The flow's thread has ended and its run is disposed. Last event of a flow.
    Finished,
}

/// Sizes and limits of a host. Defaults suit an interactive session.
#[derive(Debug, Clone)]
pub struct HostOptions {
    /// Threads that execute work runs hand to the host.
    pub effect_workers: usize,
    /// Inputs one flow may have waiting; senders are refused beyond it.
    pub inbox_capacity: usize,
    /// Observations waiting to be read; further ones are counted as dropped.
    pub event_capacity: usize,
    /// Inputs and graph work items one flow handles before observing again.
    pub turn_inputs: usize,
    pub turn_budget: usize,
    pub run: RunOptions,
    pub guest: GuestLimits,
}
impl Default for HostOptions {
    fn default() -> Self {
        Self {
            effect_workers: 4,
            inbox_capacity: 1024,
            event_capacity: 4096,
            turn_inputs: 64,
            turn_budget: 1024,
            run: RunOptions::default(),
            guest: GuestLimits::default(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum HostError {
    #[error("the host could not start: {0}")]
    Start(String),
    #[error("the package archive was refused: {0}")]
    Archive(String),
    #[error("the package has no definition file `{0}`")]
    MissingDefinition(String),
    #[error("a package node definition was refused: {0}")]
    Definition(String),
    #[error("the package component was refused: {0}")]
    Guest(#[from] GuestLoadError),
    #[error("the package graph `{0}` was refused: {1}")]
    Graph(String, String),
    #[error("the graphs of package `{0}` have blocking diagnostics")]
    GraphInvalid(String, Vec<Diagnostic>),
    #[error("the package could not be installed: {0}")]
    Install(String),
    #[error("the document could not be read: {0}")]
    Document(String),
    #[error("the document has {} blocking diagnostics", .0.len())]
    Invalid(Vec<Diagnostic>),
    #[error("the document could not be compiled: {0}")]
    Compile(#[from] CompileError),
    #[error("the run could not start: {0}")]
    Run(#[from] RunError),
    #[error("connection `{0}` cannot be used: {1}")]
    Connection(String, String),
    #[error("no such flow")]
    UnknownFlow,
    #[error("the flow has ended")]
    FlowEnded,
    #[error("the flow's mailbox is full")]
    Busy,
    #[error("the flow declares no such host event")]
    UnknownEvent,
}

/// What a front end needs to offer controls for a flow.
#[derive(Debug, Clone)]
pub struct FlowInfo {
    pub graph: GraphId,
    /// Root manual entries the host can trigger, and whether each takes a payload.
    pub manual_entries: Vec<(NodeId, bool)>,
    /// Declared events the host can raise.
    pub events: Vec<EventDeclId>,
}

/// Code compiled into the host: bundled kernels for pure and entropy nodes, and the
/// service packages' asynchronous nodes.
#[derive(Debug, Clone)]
struct Native(Arc<MathPackage>, Services);
impl PureKernelResolver for Native {
    fn resolve_binding(&self, request: &BindingRequest<'_>) -> Result<KernelBinding, ResolveError> {
        self.0.resolve_binding(request)
    }
}
impl EntropyKernelResolver for Native {
    fn resolve_entropy_binding(
        &self,
        request: &BindingRequest<'_>,
    ) -> Result<EntropyBinding, ResolveError> {
        self.0.resolve_entropy_binding(request)
    }
}
impl EffectResolver for Native {
    fn resolve_effect(&self, request: &BindingRequest<'_>) -> Result<EffectBinding, ResolveError> {
        self.1.resolve(request)
    }
}

/// A connection the host holds for every document that declares its ID as shared.
struct SharedConnection {
    adapter: String,
    settings: serde_json::Map<String, serde_json::Value>,
    /// Opened when the first flow needs it, then the same session for all of them.
    session: Option<Arc<dyn Session>>,
}
/// Settings are left out: they are the user's and may name private hosts.
impl std::fmt::Debug for SharedConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedConnection")
            .field("adapter", &self.adapter)
            .field("open", &self.session.is_some())
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
struct Flow {
    info: FlowInfo,
    inbox: orange_runtime::RunInbox,
    sources: BTreeMap<EventDeclId, RunEventSource>,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}
impl Flow {
    fn wake(&self) {
        if let Some(thread) = &self.thread {
            thread.thread().unpark();
        }
    }
}

/// See the crate documentation.
#[derive(Debug)]
pub struct Host {
    options: HostOptions,
    native: Native,
    engine: GuestEngine,
    guests: Vec<GuestPackage>,
    graphs: Graphs,
    adapters: BTreeMap<String, Arc<dyn ConnectionAdapter>>,
    shared: BTreeMap<ConnectionId, SharedConnection>,
    store: PackageStore<MemoryJournal>,
    pool: Pool,
    sink: Sink,
    clock: Instant,
    flows: BTreeMap<FlowId, Flow>,
    next_flow: u64,
}

/// A stable identity for code compiled into this host, in the form package digests use.
fn built_in_digest(what: &str) -> String {
    let hash = Sha256::digest(format!("{what}@{}", env!("CARGO_PKG_VERSION")).as_bytes());
    let mut text = String::from("sha256:");
    for byte in hash {
        text.push_str(&format!("{byte:02x}"));
    }
    text
}

fn wall_ns() -> Option<i64> {
    let elapsed = SystemTime::now().duration_since(UNIX_EPOCH).ok()?;
    i64::try_from(elapsed.as_nanos()).ok()
}

impl Host {
    /// Starts a host with the core language and the bundled base nodes registered and no
    /// flows. The receiver carries every flow's events; dropping it only stops
    /// observation.
    pub fn new(options: HostOptions) -> Result<(Self, Receiver<HostEvent>), HostError> {
        Self::with_services(options, Vec::new())
    }

    /// Like [`Host::new`], with native service packages built into the application:
    /// their nodes are registered beside the bundled ones and their connection
    /// adapters become available to documents.
    pub fn with_services(
        options: HostOptions,
        packages: Vec<ServicePackage>,
    ) -> Result<(Self, Receiver<HostEvent>), HostError> {
        let start = |error: &dyn std::fmt::Display| HostError::Start(error.to_string());
        let math = MathPackage::new(built_in_digest("orange-bundled")).map_err(|e| start(&e))?;
        let origin = HostOrigin::new(env!("CARGO_PKG_VERSION"), built_in_digest("orange-host"))
            .map_err(|e| start(&e))?;
        let mut builder = math
            .register(RegistryBuilder::core(origin))
            .map_err(|e| start(&e))?;
        let mut services = Services::default();
        let mut adapters = BTreeMap::new();
        for package in &packages {
            let key = ProviderKey {
                package: package.id.clone(),
                version: package.version.clone(),
                content_digest: built_in_digest(&package.id),
                id: SERVICE_PROVIDER.to_owned(),
            };
            let mut provider = ProviderDescriptor {
                key: key.clone(),
                implements: Default::default(),
                specializations: Default::default(),
            };
            for (definition, _) in &package.nodes {
                let _ = provider
                    .implements
                    .insert((definition.definition.clone(), definition.config_version));
                let _ = provider
                    .specializations
                    .insert(definition.definition.clone(), vec![Default::default()]);
                builder = builder
                    .definition(definition.clone())
                    .new_binding_default(definition.definition.clone(), key.clone());
            }
            builder = builder.provider(provider);
            services.add(&key, package);
            for (kind, adapter) in &package.adapters {
                let _ = adapters.insert(kind.clone(), Arc::clone(adapter));
            }
        }
        let base = builder.build().map_err(|e| start(&e))?;
        let engine = GuestEngine::new(options.guest).map_err(|e| start(&e))?;
        let pool = Pool::new(options.effect_workers).map_err(|e| start(&e))?;
        let (events, observer) = sync_channel(options.event_capacity.max(1));
        let host = Self {
            native: Native(Arc::new(math), services),
            adapters,
            shared: BTreeMap::new(),
            engine,
            guests: Vec::new(),
            graphs: Graphs::new(),
            store: PackageStore::new(base, HOST_API_VERSION, MemoryJournal::new()),
            pool,
            sink: Sink {
                events,
                dropped: Arc::new(AtomicU64::new(0)),
            },
            clock: Instant::now(),
            flows: BTreeMap::new(),
            next_flow: 1,
            options,
        };
        Ok((host, observer))
    }

    /// Gives every document that declares connection `id` as shared the same session.
    /// The settings stay in the host and are never written into a document. Replacing a
    /// shared connection affects flows opened afterwards; running flows keep theirs.
    pub fn share_connection(
        &mut self,
        id: ConnectionId,
        adapter: impl Into<String>,
        settings: serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), HostError> {
        let adapter = adapter.into();
        if !self.adapters.contains_key(&adapter) {
            return Err(HostError::Connection(
                id.to_string(),
                format!("no adapter `{adapter}` is available"),
            ));
        }
        let _ = self.shared.insert(
            id,
            SharedConnection {
                adapter,
                settings,
                session: None,
            },
        );
        Ok(())
    }

    /// Opens the session of every connection `document` declares.
    fn sessions(
        &mut self,
        document: &GraphDocument,
    ) -> Result<BTreeMap<ConnectionId, Arc<dyn Session>>, HostError> {
        let mut sessions = BTreeMap::new();
        for decl in document.connections().values() {
            let refused = |reason: String| HostError::Connection(decl.name.clone(), reason);
            let adapter = self
                .adapters
                .get(&decl.adapter)
                .ok_or_else(|| refused(format!("no adapter `{}` is available", decl.adapter)))?;
            let session = match decl.scope {
                ConnectionScope::Graph => adapter.open(&decl.settings).map_err(refused)?,
                ConnectionScope::Shared => {
                    let shared = self.shared.get_mut(&decl.id).ok_or_else(|| {
                        refused("the host shares no connection with this ID".into())
                    })?;
                    if shared.adapter != decl.adapter {
                        return Err(refused(format!(
                            "the shared connection uses adapter `{}`",
                            shared.adapter
                        )));
                    }
                    match &shared.session {
                        Some(session) => Arc::clone(session),
                        None => {
                            let session = adapter.open(&shared.settings).map_err(refused)?;
                            shared.session = Some(Arc::clone(&session));
                            session
                        }
                    }
                }
            };
            let _ = sessions.insert(decl.id, session);
        }
        Ok(sessions)
    }

    /// The registry new documents are checked and compiled against.
    pub fn registry(&self) -> &Arc<RegistrySnapshot> {
        self.store.current()
    }

    /// Observations that found the event channel full and were discarded.
    pub fn events_dropped(&self) -> u64 {
        self.sink.dropped.load(Ordering::Relaxed)
    }

    /// Installs a `.orangeplug` archive and makes its nodes available to documents
    /// opened afterwards. Running flows keep the providers they were compiled with.
    ///
    /// Each contributed node `<package>/<name>` is described by the archive file
    /// `nodes/<name>.json`. A package provider becomes the choice for new nodes only
    /// where no provider is chosen yet; an existing choice is never replaced here.
    /// `grants` are the host capabilities the user approved for the package's code.
    pub fn install(&mut self, archive: &[u8], grants: Grants) -> Result<PackageKey, HostError> {
        let mut files = MemorySink::new();
        let verified = read_archive(archive, &ArchiveLimits::default(), &mut files)
            .map_err(|error| HostError::Archive(error.to_string()))?;
        let manifest = verified.manifest().clone();
        let key = manifest.key().clone();

        let content_digest = verified.content_digest().to_owned();
        let read = |path: &str| -> Result<&str, HostError> {
            let missing = || HostError::MissingDefinition(path.to_owned());
            let archive_path: ArchivePath = path.parse().map_err(|_| missing())?;
            let bytes = files.file(path).ok_or_else(missing)?;
            verified
                .verify_file(&archive_path, bytes)
                .map_err(|error| HostError::Archive(error.to_string()))?;
            std::str::from_utf8(bytes).map_err(|error| HostError::Definition(error.to_string()))
        };
        let local = |name: &str| name.rsplit('/').next().unwrap_or(name).to_owned();

        let mut definitions = Vec::with_capacity(manifest.nodes().len());
        let mut definition_text = BTreeMap::new();
        for node in manifest.nodes() {
            let name = node.to_string();
            let path = format!("nodes/{}.json", local(&name));
            let text = read(&path)?;
            let definition = NodeDefinition::from_json(text)
                .map_err(|error| HostError::Definition(error.to_string()))?;
            if definition.definition.id != *node {
                return Err(HostError::Definition(format!(
                    "`{path}` does not define `{name}`"
                )));
            }
            let _ = definition_text.insert((name, u64::from(definition.definition.version)), text);
            definitions.push(definition);
        }

        // Prepare every implementation before anything is published, so a refused
        // component or graph leaves the registry untouched.
        let provider_key = |id: &str| ProviderKey {
            package: key.id.as_str().to_owned(),
            version: key.version.as_str().to_owned(),
            content_digest: content_digest.clone(),
            id: id.to_owned(),
        };
        let mut guests: Vec<GuestPackage> = Vec::new();
        let mut graphs: Vec<(graphs::GraphKey, Arc<GraphNode>)> = Vec::new();
        for provider in manifest.providers() {
            if provider.id == GRAPH_PROVIDER {
                let ProviderSlot::Node(definition) = &provider.slot else {
                    continue;
                };
                let name = definition.id.to_string();
                let version = u64::from(definition.version);
                let path = format!("graphs/{}.json", local(&name));
                let text = read(&path)?;
                let contract = definition_text
                    .get(&(name.clone(), version))
                    .ok_or_else(|| HostError::MissingDefinition(name.clone()))?;
                let graph = GraphNode::decode(text, contract)
                    .map_err(|reason| HostError::Graph(name.clone(), reason))?;
                let graph_key = (
                    key.id.as_str().to_owned(),
                    key.version.as_str().to_owned(),
                    name,
                    version,
                );
                graphs.push((graph_key, Arc::new(graph)));
            } else if matches!(manifest.execution(), Some(Execution::WasmComponent { .. }))
                && guests
                    .iter()
                    .all(|guest| guest.provider().id != provider.id)
            {
                guests.push(GuestPackage::from_archive(
                    &self.engine,
                    &verified,
                    &files,
                    &provider.id,
                    grants,
                )?);
            }
        }

        let install = |error: &dyn std::fmt::Display| HostError::Install(error.to_string());
        let _ = self
            .store
            .install(verified.into_contents(definitions))
            .map_err(|e| install(&e))?;

        // A graph is checked as it will be used: linked with the graphs it builds on,
        // against the registry that now includes this package's own definitions.
        if !graphs.is_empty() {
            let mut known = self.graphs.clone();
            known.extend(graphs.iter().cloned());
            let bodies: Vec<_> = graphs.iter().map(|(_, graph)| Arc::clone(graph)).collect();
            if let Err(error) = self.check_graphs(key.id.as_str(), &bodies, &known) {
                let _ = self.store.rollback();
                return Err(error);
            }
        }

        for provider in manifest.providers() {
            let ProviderSlot::Node(definition) = &provider.slot else {
                continue;
            };
            let implemented = provider.id == GRAPH_PROVIDER
                || guests
                    .iter()
                    .any(|guest| guest.provider().id == provider.id);
            if implemented
                && self
                    .store
                    .current()
                    .new_binding_default(definition)
                    .is_none()
            {
                let _ = self
                    .store
                    .select_default(definition.clone(), provider_key(&provider.id))
                    .map_err(|e| install(&e))?;
            }
        }
        self.guests.extend(guests);
        self.graphs.extend(graphs);
        Ok(key)
    }

    fn check_graphs(
        &self,
        package: &str,
        bodies: &[Arc<GraphNode>],
        known: &Graphs,
    ) -> Result<(), HostError> {
        let refused = |reason: String| HostError::Graph(package.to_owned(), reason);
        let empty = to_json(&GraphDocument::new(
            SCRATCH_GRAPH
                .parse()
                .map_err(|_| refused("invalid validation graph id".into()))?,
        ))
        .map_err(|error| refused(error.to_string()))?;
        let mut scratch = graphs::scratch(&empty, bodies)
            .ok_or_else(|| refused("could not build a validation document".into()))?;
        graphs::link(&mut scratch, known);
        let document =
            from_json(&scratch.to_string()).map_err(|error| refused(error.to_string()))?;
        let registry = self.store.current();
        let analysis = analyze(&document, registry);
        if analysis.is_ready_for_lowering(&document, registry) {
            Ok(())
        } else {
            Err(HostError::GraphInvalid(
                package.to_owned(),
                analysis.report().diagnostics.clone(),
            ))
        }
    }

    /// Decodes a saved document and links the graph-implemented package nodes it uses.
    fn linked(&self, json: &str) -> Result<GraphDocument, HostError> {
        let document = |error: &dyn std::fmt::Display| HostError::Document(error.to_string());
        if self.graphs.is_empty() {
            return from_json(json).map_err(|e| document(&e));
        }
        let mut saved = serde_json::from_str(json).map_err(|e| document(&e))?;
        graphs::link(&mut saved, &self.graphs);
        from_json(&saved.to_string()).map_err(|e| document(&e))
    }

    /// Checks a document against the current registry without running it. An empty
    /// list means [`Host::open`] would accept it as far as validation goes.
    pub fn check(&self, json: &str) -> Result<Vec<Diagnostic>, HostError> {
        let document = self.linked(json)?;
        let registry = self.store.current();
        let analysis = analyze(&document, registry);
        if analysis.is_ready_for_lowering(&document, registry) {
            Ok(Vec::new())
        } else {
            Ok(analysis.report().diagnostics.clone())
        }
    }

    /// Validates, compiles and starts a document as a new flow on its own thread.
    pub fn open(&mut self, json: &str) -> Result<FlowId, HostError> {
        let document = self.linked(json)?;
        let registry = Arc::clone(self.store.current());
        let analysis = analyze(&document, &registry);
        if !analysis.is_ready_for_lowering(&document, &registry) {
            return Err(HostError::Invalid(analysis.report().diagnostics.clone()));
        }
        let mut resolver = WithGuests::new(self.native.clone());
        for guest in &self.guests {
            resolver.add(guest.clone());
        }
        let plan = Arc::new(compile_control_with_effects(
            &document,
            Arc::clone(&registry),
            &analysis,
            &resolver,
            &resolver,
            CompileOptions::default(),
        )?);
        let sessions = self.sessions(&document)?;
        let run = Run::start(
            Arc::clone(&plan),
            &document,
            &registry,
            0,
            self.options.run.clone(),
        )?;

        let now = u64::try_from(self.clock.elapsed().as_nanos()).unwrap_or(u64::MAX);
        let (mut driver, inbox) = RunDriver::new(run, self.options.inbox_capacity.max(1), now);
        let mut sources = BTreeMap::new();
        for event in plan.events() {
            // Events the run does not open to the host are simply not offered.
            if let Ok(source) = driver.register_source(event.id(), SourceOptions::default()) {
                let _ = sources.insert(event.id(), source);
            }
        }
        let info = FlowInfo {
            graph: document.graph_id(),
            manual_entries: plan
                .manuals()
                .iter()
                .map(|entry| (entry.node(), entry.payload_type().is_some()))
                .collect(),
            events: sources.keys().copied().collect(),
        };

        let id = FlowId(self.next_flow);
        self.next_flow += 1;
        let shared = Arc::new(Shared {
            state: Mutex::new(driver.run().state()),
            queries: Mutex::new(Some(VecDeque::new())),
            closing: AtomicBool::new(false),
        });
        let context = Context {
            id,
            shared: Arc::clone(&shared),
            sink: self.sink.clone(),
            pool: self.pool.clone(),
            inbox: inbox.clone(),
            sessions,
            clock: self.clock,
            turn_inputs: self.options.turn_inputs.max(1),
            turn_budget: self.options.turn_budget.max(1),
        };
        let thread = thread::Builder::new()
            .name(format!("orange-flow-{id}"))
            .spawn(move || flow::drive(driver, context))
            .map_err(|error| HostError::Start(error.to_string()))?;
        let _ = self.flows.insert(
            id,
            Flow {
                info,
                inbox,
                sources,
                shared,
                thread: Some(thread),
            },
        );
        Ok(id)
    }

    /// Flows that have not been closed, including ones whose run has ended.
    pub fn flows(&self) -> impl Iterator<Item = FlowId> + '_ {
        self.flows.keys().copied()
    }

    pub fn info(&self, flow: FlowId) -> Option<&FlowInfo> {
        self.flows.get(&flow).map(|flow| &flow.info)
    }

    /// The state the flow's thread last observed.
    pub fn state(&self, flow: FlowId) -> Option<RunState> {
        let flow = self.flows.get(&flow)?;
        Some(
            *flow
                .shared
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        )
    }

    fn send(&self, flow: FlowId, input: HostInput) -> Result<(), HostError> {
        let flow = self.flows.get(&flow).ok_or(HostError::UnknownFlow)?;
        flow.inbox.send(input).map_err(|error| match error {
            InboxError::Full(_) => HostError::Busy,
            InboxError::Closed(_) => HostError::FlowEnded,
        })?;
        flow.wake();
        Ok(())
    }

    /// Holds the flow: no graph work runs and its clock stops. Input is still taken
    /// and handled by each entry's `while_paused` policy.
    pub fn pause(&self, flow: FlowId) -> Result<(), HostError> {
        self.send(flow, HostInput::Pause)
    }
    pub fn resume(&self, flow: FlowId) -> Result<(), HostError> {
        self.send(flow, HostInput::Resume)
    }
    /// Asks the flow to stop. Its `Finished` event follows once cleanup is done.
    pub fn stop(&self, flow: FlowId, mode: StopMode) -> Result<(), HostError> {
        self.send(flow, HostInput::Stop(mode))
    }
    /// Fires a root manual entry.
    pub fn trigger(
        &self,
        flow: FlowId,
        node: NodeId,
        payload: Option<Value>,
    ) -> Result<(), HostError> {
        self.send(
            flow,
            HostInput::Trigger {
                node,
                payload,
                wall_ns: wall_ns(),
            },
        )
    }
    /// Raises a declared event in the flow, as an external source would.
    pub fn emit(&self, flow: FlowId, event: EventDeclId, payload: Value) -> Result<(), HostError> {
        let source = *self
            .flows
            .get(&flow)
            .ok_or(HostError::UnknownFlow)?
            .sources
            .get(&event)
            .ok_or(HostError::UnknownEvent)?;
        self.send(
            flow,
            HostInput::Event {
                source,
                payload,
                wall_ns: wall_ns(),
            },
        )
    }

    /// Reads from the flow's run on its own thread, between two turns, and returns the
    /// result. The run is never locked or paused for it. Keep `read` short: the flow
    /// does nothing else meanwhile.
    pub fn inspect<T: Send + 'static>(
        &self,
        flow: FlowId,
        read: impl FnOnce(&Run) -> T + Send + 'static,
    ) -> Result<T, HostError> {
        let flow = self.flows.get(&flow).ok_or(HostError::UnknownFlow)?;
        let (answer, result) = channel();
        let query: Query = Box::new(move |run| {
            let _ = answer.send(read(run));
        });
        flow.shared
            .queries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_mut()
            .ok_or(HostError::FlowEnded)?
            .push_back(query);
        flow.wake();
        result.recv().map_err(|_| HostError::FlowEnded)
    }

    /// Stops the flow at once if it is still running, waits for its thread and forgets
    /// it. Its remaining events stay readable.
    pub fn close(&mut self, flow: FlowId) -> Result<(), HostError> {
        let mut flow = self.flows.remove(&flow).ok_or(HostError::UnknownFlow)?;
        flow.shared.closing.store(true, Ordering::Relaxed);
        flow.wake();
        if let Some(thread) = flow.thread.take() {
            let _ = thread.join();
        }
        Ok(())
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        for flow in self.flows.values() {
            flow.shared.closing.store(true, Ordering::Relaxed);
            flow.wake();
        }
        for flow in self.flows.values_mut() {
            if let Some(thread) = flow.thread.take() {
                let _ = thread.join();
            }
        }
    }
}

/// Identity of the throwaway document a package's graphs are validated in.
const SCRATCH_GRAPH: &str = "00000000-0000-4000-8000-000000000000";

/// Builds a `.orangeplug` archive from a manifest and the package's other files, given
/// as archive paths and contents. The manifest's content digest is computed and filled
/// in, and the result is verified as an installer would before it is returned.
pub fn pack(manifest: &[u8], files: &[(String, Vec<u8>)]) -> Result<Vec<u8>, HostError> {
    let refused = |error: &dyn std::fmt::Display| HostError::Archive(error.to_string());
    let mut manifest: serde_json::Value =
        serde_json::from_slice(manifest).map_err(|e| refused(&e))?;
    let mut entries = Vec::with_capacity(files.len());
    for (path, bytes) in files {
        let path: ArchivePath = path.parse().map_err(|e| refused(&e))?;
        entries.push((path, &bytes[..]));
    }
    let entries = ArchiveFiles::new(entries).map_err(|e| refused(&e))?;
    let encode =
        |manifest: &serde_json::Value| serde_json::to_vec(manifest).map_err(|e| refused(&e));
    manifest["integrity"] =
        serde_json::json!({"algorithm": "sha256", "content_digest": "0".repeat(64)});
    let digest = entries
        .content_digest(&encode(&manifest)?)
        .map_err(|e| refused(&e))?;
    manifest["integrity"]["content_digest"] =
        serde_json::json!(digest.strip_prefix("sha256:").unwrap_or(&digest));
    let archive = entries.encode(&encode(&manifest)?);
    let _ = read_archive(&archive[..], &ArchiveLimits::default(), &mut DiscardSink)
        .map_err(|e| refused(&e))?;
    Ok(archive)
}

/// Decodes connection settings from the text of a JSON object, for callers that hold
/// them in a file. Anything but an object is refused.
pub fn settings_from_json(
    text: &[u8],
) -> Result<serde_json::Map<String, serde_json::Value>, HostError> {
    serde_json::from_slice(text)
        .map_err(|error| HostError::Connection("settings".into(), error.to_string()))
}
