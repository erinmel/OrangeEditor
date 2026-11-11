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
mod persist;
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
use orange_model::catalog::{ProviderDescriptor, RegistryBuilder, RegistrySnapshot};
use orange_model::document::{ConnectionDecl, ConnectionScope};
use orange_model::validation::{Diagnostic, HostOrigin};
use orange_model::validation::{LeafEffects, ValidationAnalysis};
use orange_model::{GraphDocument, analyze_with_effects, from_json, to_json};
use orange_packages::{
    ArchiveFiles, ArchiveLimits, ArchivePath, DiscardSink, Execution, HOST_API_VERSION,
    MemoryJournal, MemorySink, PackageStore, ProviderSlot, read_archive,
};
use orange_runtime::{
    HostInput, InboxError, Run, RunDriver, RunEventSource, Workspace, WorkspaceLimits,
};
use services::Services;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel, sync_channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub use graphs::{GRAPH_NODE_FORMAT, GRAPH_PROVIDER};
pub use orange_guest::{Grants, GuestLimits};
pub use orange_kernel::{FaultText, KernelFault};
pub use orange_model::Value;
pub use orange_model::catalog::{NodeDefinition, Presentation, ProviderKey, TextRef};
pub use orange_model::document::DefinitionRef;
pub use orange_model::id::{ConnectionId, PortKey};
pub use orange_model::id::{EventDeclId, GraphId, NodeId, VariableId};
pub use orange_packages::PackageKey;
pub use orange_runtime::EffectLease;
pub use orange_runtime::{
    ControlOutcome, RefusedReason, RunError, RunEvent, RunOptions, RunState, StopMode,
};
pub use orange_sdk::{
    Call, ConnectionAdapter, EmitError, Emitter, SERVICE_PROVIDER, ServiceNode, ServicePackage,
    ServiceSource, Session,
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
    /// Variables whose value differs from the last one reported, with the value now
    /// (`None` while unset). Sent only while the flow is observed; the first one after
    /// observation starts holds every variable the graph uses. Several writes between
    /// two reports appear as the latest value only.
    Variables(Vec<(VariableId, Option<Value>)>),
    /// Output ports whose value differs from the last one reported, with the value now.
    /// Sent only while the flow is observed. A port appears once a node has read it;
    /// constants, faults and outputs nothing reads are not reported, and several values
    /// between two reports appear as the latest only.
    Ports(Vec<(NodeId, PortKey, Value)>),
    /// The flow's graph was replaced by an edited version. Events after this come
    /// from the new graph, starting with its state.
    Updated {
        kept_variables: usize,
        reset_variables: usize,
    },
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
    /// Where installed packages are kept between sessions. With a directory, every
    /// installation is saved there and a host started on it installs them again.
    /// `None` keeps packages for this session only.
    pub package_dir: Option<PathBuf>,
    /// Whether the bundled base nodes (arithmetic, comparison, text and collection
    /// operations) are registered. Without them a document can use only the core
    /// language and what the application and installed packages provide.
    pub base_nodes: bool,
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
            package_dir: None,
            base_nodes: true,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum HostError {
    #[error("the host could not start: {0}")]
    Start(String),
    #[error("shared variables: {0}")]
    Workspace(String),
    #[error("installed packages could not be stored: {0}")]
    Storage(String),
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
    #[error("entry node `{0}` cannot be fed: {1}")]
    Source(NodeId, String),
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

/// Validates a document with what the host knows about package nodes used inside
/// functions: none of them writes a graph variable. Package code, whether a component
/// or a native service, has no access to a run's variables, so this is a fact about the
/// execution boundary, not a claim taken from the package.
fn analyzed(document: &GraphDocument, registry: &RegistrySnapshot) -> ValidationAnalysis {
    let effects = document
        .functions()
        .values()
        .flat_map(|function| function.body.nodes.values())
        .filter(|node| !node.definition.id.is_core())
        .map(|node| (node.id, LeafEffects::Known(Vec::new())))
        .collect();
    analyze_with_effects(document, registry, effects)
}

/// Sessions by connection, with the declaration each was opened from.
type Opened = BTreeMap<ConnectionId, (ConnectionDecl, Arc<dyn Session>)>;
type Feed = (NodeId, Arc<dyn ServiceSource>, Option<Arc<dyn Session>>);

fn sessions_of(opened: &Opened) -> BTreeMap<ConnectionId, Arc<dyn Session>> {
    opened
        .iter()
        .map(|(id, (_, session))| (*id, Arc::clone(session)))
        .collect()
}

/// A saved document without its layout and edit counter: equal texts mean the two
/// documents run identically.
fn semantic(json: &str) -> Result<String, HostError> {
    let mut saved: serde_json::Value =
        serde_json::from_str(json).map_err(|error| HostError::Document(error.to_string()))?;
    if let Some(root) = saved.as_object_mut() {
        let _ = root.remove("editor");
        let _ = root.remove("revision");
    }
    Ok(saved.to_string())
}

/// A document ready to run.
struct Prepared {
    document: GraphDocument,
    registry: Arc<RegistrySnapshot>,
    plan: Arc<orange_compiler::ControlPlan>,
    opened: Opened,
    feeds: Vec<Feed>,
    manual_entries: Vec<(NodeId, bool)>,
    semantic: String,
}

/// What [`Host::update`] did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Updated {
    /// `false` when the edit changed nothing that runs; the flow was not touched.
    pub changed: bool,
    /// Run variables that kept their value.
    pub kept_variables: usize,
    /// Run variables that are new or changed type and started from their initial value.
    pub reset_variables: usize,
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

struct Flow {
    info: FlowInfo,
    inbox: orange_runtime::RunInbox,
    sources: BTreeMap<EventDeclId, RunEventSource>,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
    /// Guards of the running service sources; dropping them stops the sources.
    feeds: Vec<Box<dyn Send>>,
    /// The connections this flow holds, with the declarations they were opened from.
    opened: Opened,
    /// The saved document without layout, to tell a real edit from a moved node.
    semantic: String,
}
impl std::fmt::Debug for Flow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Flow")
            .field("info", &self.info)
            .field("feeds", &self.feeds.len())
            .finish_non_exhaustive()
    }
}
impl Flow {
    fn wake(&self) {
        if let Some(thread) = &self.thread {
            thread.thread().unpark();
        }
    }
}

/// One node a document can use, as [`Host::nodes`] lists it.
#[derive(Debug, Clone, Copy)]
pub struct NodeInfo<'a> {
    /// Identity, kind and ports.
    pub definition: &'a NodeDefinition,
    /// Label, description and category from the node's definition file. `None` for
    /// core and bundled nodes, whose text comes from the host application's catalogue.
    pub presentation: Option<&'a Presentation>,
    /// The implementation a newly placed node binds to. `None` for core nodes, which
    /// the language implements, and for a definition nothing implements.
    pub provider: Option<&'a ProviderKey>,
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
    presentations: BTreeMap<DefinitionRef, Presentation>,
    not_restored: Vec<(String, HostError)>,
    space: Arc<flow::Space>,
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
        let mut builder = RegistryBuilder::core(origin);
        if options.base_nodes {
            builder = math.register(builder).map_err(|e| start(&e))?;
        }
        let mut services = Services::default();
        let mut adapters = BTreeMap::new();
        let mut presentations = BTreeMap::new();
        for package in &packages {
            presentations.extend(package.presentations().iter().cloned());
            let key = ProviderKey {
                package: package.id().to_owned(),
                version: package.version().to_owned(),
                content_digest: built_in_digest(package.id()),
                id: SERVICE_PROVIDER.to_owned(),
            };
            let mut provider = ProviderDescriptor {
                key: key.clone(),
                implements: Default::default(),
                specializations: Default::default(),
            };
            let definitions = package
                .nodes()
                .iter()
                .map(|(definition, _)| definition)
                .chain(package.sources().iter().map(|(definition, _)| definition));
            for definition in definitions {
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
            for (kind, adapter) in package.adapters() {
                let _ = adapters.insert(kind.clone(), Arc::clone(adapter));
            }
        }
        let base = builder.build().map_err(|e| start(&e))?;
        // A newly placed node needs a provider to bind to. Built-in nodes that have
        // one implementation and no choice yet get it, so a front end can place them.
        let mut defaults = RegistryBuilder::from_snapshot(&base);
        let mut chosen = BTreeSet::new();
        for provider in base.providers() {
            for (definition, _) in &provider.implements {
                if base.new_binding_default(definition).is_none()
                    && chosen.insert(definition.clone())
                {
                    defaults =
                        defaults.new_binding_default(definition.clone(), provider.key.clone());
                }
            }
        }
        let base = defaults.build().map_err(|e| start(&e))?;
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
            presentations,
            not_restored: Vec::new(),
            space: Arc::default(),
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
        let mut host = host;
        if let Some(directory) = host.options.package_dir.clone() {
            let saved = persist::load(&directory).map_err(HostError::Storage)?;
            for package in saved {
                let installed = package
                    .archive
                    .map_err(HostError::Storage)
                    .and_then(|archive| host.install_archive(&archive, package.grants));
                if let Err(error) = installed {
                    host.not_restored.push((package.file, error));
                }
            }
        }
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

    /// Opens the session of every connection `document` declares, keeping the session
    /// of a graph connection whose declaration is the same as in `previous`.
    fn sessions(
        &mut self,
        document: &GraphDocument,
        previous: Option<&Opened>,
    ) -> Result<Opened, HostError> {
        let mut sessions = BTreeMap::new();
        for decl in document.connections().values() {
            let refused = |reason: String| HostError::Connection(decl.name.clone(), reason);
            let adapter = self
                .adapters
                .get(&decl.adapter)
                .ok_or_else(|| refused(format!("no adapter `{}` is available", decl.adapter)))?;
            let session = match decl.scope {
                ConnectionScope::Graph => match previous.and_then(|opened| opened.get(&decl.id)) {
                    Some((before, session)) if before == decl => Arc::clone(session),
                    _ => adapter.open(&decl.settings).map_err(refused)?,
                },
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
            let _ = sessions.insert(decl.id, (decl.clone(), session));
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
    ///
    /// With [`HostOptions::package_dir`] the archive is then saved. If saving fails the
    /// package stays installed for this session and the error says it was not stored.
    pub fn install(&mut self, archive: &[u8], grants: Grants) -> Result<PackageKey, HostError> {
        let key = self.install_archive(archive, grants)?;
        if let Some(directory) = &self.options.package_dir {
            persist::save(directory, archive, grants).map_err(HostError::Storage)?;
        }
        Ok(key)
    }

    /// Saved packages that could not be installed again when this host started, by
    /// archive file name. The others are installed; these stay on disk untouched.
    pub fn not_restored(&self) -> &[(String, HostError)] {
        &self.not_restored
    }

    fn install_archive(&mut self, archive: &[u8], grants: Grants) -> Result<PackageKey, HostError> {
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
        let mut described = Vec::with_capacity(manifest.nodes().len());
        let mut definition_text = BTreeMap::new();
        for node in manifest.nodes() {
            let name = node.to_string();
            let path = format!("nodes/{}.json", local(&name));
            let text = read(&path)?;
            let (definition, presentation) = NodeDefinition::described(text)
                .map_err(|error| HostError::Definition(error.to_string()))?;
            described.push((definition.definition.clone(), presentation));
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
        let contents = verified.into_contents(definitions);

        // A graph is checked as it will be used: linked with the graphs it builds on,
        // against a registry that includes this package's own definitions. The check
        // runs on a throwaway store first, because publishing a version is permanent:
        // a package refused here can be corrected and installed under the same version.
        let mut known = self.graphs.clone();
        known.extend(graphs.iter().cloned());
        let bodies: Vec<_> = graphs.iter().map(|(_, graph)| Arc::clone(graph)).collect();
        let mut checked = bodies.is_empty();
        if !checked {
            let mut trial = PackageStore::new(
                Arc::clone(self.store.current()),
                HOST_API_VERSION,
                MemoryJournal::new(),
            );
            // A package the throwaway store cannot take (it knows no installed
            // package, so a declared dependency is missing) is checked after the real
            // installation instead.
            if trial.install(contents.clone()).is_ok() {
                self.check_graphs(key.id.as_str(), &bodies, &known, trial.current())?;
                checked = true;
            }
        }
        let _ = self.store.install(contents).map_err(|e| install(&e))?;
        if !checked
            && let Err(error) =
                self.check_graphs(key.id.as_str(), &bodies, &known, self.store.current())
        {
            let _ = self.store.rollback();
            return Err(error);
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
        self.presentations.extend(described);
        Ok(key)
    }

    fn check_graphs(
        &self,
        package: &str,
        bodies: &[Arc<GraphNode>],
        known: &Graphs,
        registry: &Arc<RegistrySnapshot>,
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
        let analysis = analyzed(&document, registry);
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
        let analysis = analyzed(&document, registry);
        if analysis.is_ready_for_lowering(&document, registry) {
            Ok(Vec::new())
        } else {
            Ok(analysis.report().diagnostics.clone())
        }
    }

    /// Validates and compiles a saved document and resolves everything it needs from
    /// the host, without starting anything. `previous` are the sessions of the flow
    /// being edited, reused where a connection's declaration is unchanged.
    fn prepare(&mut self, json: &str, previous: Option<&Opened>) -> Result<Prepared, HostError> {
        let document = self.linked(json)?;
        let registry = Arc::clone(self.store.current());
        let analysis = analyzed(&document, &registry);
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
        let opened = self.sessions(&document, previous)?;
        // Every package entry needs a source before anything starts.
        let mut feeds = Vec::new();
        let mut manual_entries = Vec::new();
        for entry in plan.manuals() {
            let node = document
                .node(entry.node())
                .ok_or_else(|| HostError::Source(entry.node(), "node not found".into()))?;
            if node.definition.id.is_core() {
                manual_entries.push((entry.node(), entry.payload_type().is_some()));
                continue;
            }
            let refused = |reason: &str| HostError::Source(node.id, reason.to_owned());
            let provider = node
                .provider
                .as_ref()
                .and_then(|binding| ProviderKey::of(&document, binding))
                .ok_or_else(|| refused("the node has no exact provider"))?;
            let (source, uses_connection) = self
                .native
                .1
                .source(&provider, &node.definition)
                .ok_or_else(|| refused("no service provides this entry"))?;
            let session = if uses_connection {
                let id: ConnectionId = node
                    .config
                    .get("connection")
                    .and_then(|value| value.as_str())
                    .and_then(|text| text.parse().ok())
                    .ok_or_else(|| refused("the node names no connection"))?;
                let (_, session) = opened.get(&id).ok_or_else(|| {
                    refused("the node names a connection this document does not declare")
                })?;
                Some(Arc::clone(session))
            } else {
                None
            };
            feeds.push((node.id, source, session));
        }
        Ok(Prepared {
            semantic: semantic(json)?,
            document,
            registry,
            plan,
            opened,
            feeds,
            manual_entries,
        })
    }

    /// Starts the source of every package entry of a flow. On a failure the sources
    /// already started are stopped again.
    fn feed(flow: &mut Flow, feeds: Vec<Feed>) -> Result<(), HostError> {
        for (node, source, session) in feeds {
            let inbox = flow.inbox.clone();
            let thread = flow
                .thread
                .as_ref()
                .map_or_else(thread::current, |t| t.thread().clone());
            let emitter = Emitter::new(move |payload| {
                let input = HostInput::Trigger {
                    node,
                    payload,
                    wall_ns: wall_ns(),
                };
                inbox.send(input).map_err(|error| match error {
                    InboxError::Full(_) => EmitError::Busy,
                    InboxError::Closed(_) => EmitError::Closed,
                })?;
                thread.unpark();
                Ok(())
            });
            match source.start(session, emitter) {
                Ok(guard) => flow.feeds.push(guard),
                Err(reason) => {
                    flow.feeds.clear();
                    return Err(HostError::Source(node, reason));
                }
            }
        }
        Ok(())
    }

    /// Validates, compiles and starts a document as a new flow on its own thread.
    pub fn open(&mut self, json: &str) -> Result<FlowId, HostError> {
        let prepared = self.prepare(json, None)?;
        let run = if flow::uses_space(&prepared.plan) {
            let snapshot = self.space.snapshot().ok_or_else(|| {
                HostError::Workspace(
                    "the document uses shared variables and none are declared".into(),
                )
            })?;
            Run::start_in_workspace(
                Arc::clone(&prepared.plan),
                &prepared.document,
                &prepared.registry,
                0,
                self.options.run.clone(),
                &snapshot,
            )?
        } else {
            Run::start(
                Arc::clone(&prepared.plan),
                &prepared.document,
                &prepared.registry,
                0,
                self.options.run.clone(),
            )?
        };

        let now = u64::try_from(self.clock.elapsed().as_nanos()).unwrap_or(u64::MAX);
        let (mut driver, inbox) = RunDriver::new(run, self.options.inbox_capacity.max(1), now);
        let sources = flow::event_sources(&mut driver, &prepared.plan);
        let info = FlowInfo {
            graph: prepared.document.graph_id(),
            manual_entries: prepared.manual_entries,
            events: sources.keys().copied().collect(),
        };

        let id = FlowId(self.next_flow);
        self.next_flow += 1;
        let shared = Arc::new(Shared {
            state: Mutex::new(driver.run().state()),
            queries: Mutex::new(Some(VecDeque::new())),
            closing: AtomicBool::new(false),
            observed: AtomicBool::new(false),
            swap: Mutex::new(None),
        });
        let context = Context {
            id,
            shared: Arc::clone(&shared),
            sink: self.sink.clone(),
            pool: self.pool.clone(),
            inbox: inbox.clone(),
            plan: prepared.plan,
            sessions: sessions_of(&prepared.opened),
            space: Arc::clone(&self.space),
            clock: self.clock,
            turn_inputs: self.options.turn_inputs.max(1),
            turn_budget: self.options.turn_budget.max(1),
        };
        let thread = thread::Builder::new()
            .name(format!("orange-flow-{id}"))
            .spawn(move || flow::drive(driver, context))
            .map_err(|error| HostError::Start(error.to_string()))?;
        let _ = self
            .space
            .flows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, thread.thread().clone());
        let mut flow = Flow {
            info,
            inbox,
            sources,
            shared,
            thread: Some(thread),
            feeds: Vec::new(),
            opened: prepared.opened,
            semantic: prepared.semantic,
        };
        if let Err(error) = Self::feed(&mut flow, prepared.feeds) {
            // Nothing has been reported as opened: stop the flow again.
            flow.shared.closing.store(true, Ordering::Relaxed);
            flow.wake();
            if let Some(thread) = flow.thread.take() {
                let _ = thread.join();
            }
            return Err(error);
        }
        let _ = self.flows.insert(id, flow);
        Ok(id)
    }

    /// Replaces a running flow's graph with an edited version of the same document.
    ///
    /// The edit is validated and compiled first; if that fails the flow is untouched and
    /// keeps running. Otherwise, between two turns of the flow, its control flow
    /// restarts on the new graph: work in progress is cancelled, On Start runs again and
    /// On Stop does not. Run variables keep their values where a variable with the same
    /// ID and type still exists; other variables, and the private state of nodes such
    /// as gates and counters, start fresh. A paused flow stays paused. Connections whose
    /// declaration is unchanged keep their session. A change to layout alone does
    /// nothing.
    pub fn update(&mut self, flow: FlowId, json: &str) -> Result<Updated, HostError> {
        let current = self.flows.get(&flow).ok_or(HostError::UnknownFlow)?;
        if semantic(json)? == current.semantic {
            return Ok(Updated::default());
        }
        let previous = current.opened.clone();
        let graph = current.info.graph;
        let prepared = self.prepare(json, Some(&previous))?;
        if prepared.document.graph_id() != graph {
            return Err(HostError::Document(
                "the edit belongs to another graph".into(),
            ));
        }
        let (reply, answer) = channel();
        let swap = flow::Swap {
            plan: prepared.plan,
            document: prepared.document,
            registry: prepared.registry,
            options: self.options.run.clone(),
            inbox_capacity: self.options.inbox_capacity.max(1),
            sessions: sessions_of(&prepared.opened),
            reply,
        };
        let current = self.flows.get_mut(&flow).ok_or(HostError::UnknownFlow)?;
        *current
            .shared
            .swap
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(swap);
        current.wake();
        let swapped = loop {
            match answer.recv_timeout(std::time::Duration::from_millis(20)) {
                Ok(outcome) => break outcome?,
                Err(RecvTimeoutError::Disconnected) => return Err(HostError::FlowEnded),
                Err(RecvTimeoutError::Timeout) => {
                    // A flow that ended without taking the edit will never answer.
                    if current.thread.as_ref().is_none_or(JoinHandle::is_finished) {
                        return Err(HostError::FlowEnded);
                    }
                }
            }
        };
        // Sources of the old graph fed its mailbox; replace them with the new graph's.
        current.feeds.clear();
        current.inbox = swapped.inbox;
        current.info.manual_entries = prepared.manual_entries;
        current.info.events = swapped.sources.keys().copied().collect();
        current.sources = swapped.sources;
        current.opened = prepared.opened;
        current.semantic = prepared.semantic;
        Self::feed(current, prepared.feeds)?;
        Ok(Updated {
            changed: true,
            kept_variables: swapped.kept,
            reset_variables: swapped.reset,
        })
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

    /// Declares the variables flows can share: the `workspace`-scope variables of the
    /// given document, with their initial values. It can be done once, before any
    /// document that uses them is opened. A document then declares the ones it uses
    /// with the same IDs and types, reads them with `core/get-variable` and writes them
    /// with `core/set-shared` or `core/update-shared`. Every flow sees a write at its
    /// next turn; writes from different flows are applied one at a time.
    pub fn share_variables(&mut self, json: &str) -> Result<(), HostError> {
        let document = from_json(json).map_err(|error| HostError::Document(error.to_string()))?;
        let mut workspace = self
            .space
            .workspace
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if workspace.is_some() {
            return Err(HostError::Workspace("they are already declared".into()));
        }
        let created = Workspace::new(
            &document,
            Arc::clone(self.store.current()),
            WorkspaceLimits::default(),
        )
        .map_err(|error| HostError::Workspace(error.to_string()))?;
        *workspace = Some(created);
        Ok(())
    }

    /// The current value of a shared variable; `None` while unset or undeclared.
    pub fn shared_variable(&self, id: VariableId) -> Option<Value> {
        self.space.snapshot()?.get(id).ok().flatten().cloned()
    }

    /// Every node a document can use, ordered by ID and version: the core language, the
    /// bundled and built-in service packages and every installed package. This is the
    /// data of a node palette; a node's ports are in its definition.
    pub fn nodes(&self) -> impl Iterator<Item = NodeInfo<'_>> {
        let registry = self.store.current();
        registry.definitions().map(move |definition| NodeInfo {
            definition,
            presentation: self.presentations.get(&definition.definition),
            provider: registry.new_binding_default(&definition.definition),
        })
    }

    /// Turns reporting of the flow's values on or off. While on, the flow sends
    /// [`FlowEvent::Variables`] and [`FlowEvent::Ports`] after each turn in which a
    /// variable or an output port differs from what was last reported, starting with
    /// every variable. It costs one comparison per variable and per port per turn, so a
    /// front end observes the flows it is showing.
    pub fn observe(&self, flow: FlowId, on: bool) -> Result<(), HostError> {
        let flow = self.flows.get(&flow).ok_or(HostError::UnknownFlow)?;
        flow.shared.observed.store(on, Ordering::Relaxed);
        flow.wake();
        Ok(())
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
        let _ = self
            .space
            .flows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&flow);
        let mut flow = self.flows.remove(&flow).ok_or(HostError::UnknownFlow)?;
        flow.feeds.clear();
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
        for flow in self.flows.values_mut() {
            flow.feeds.clear();
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
