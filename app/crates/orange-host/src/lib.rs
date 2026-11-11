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

use effects::Pool;
use flow::{Context, Query, Shared, Sink};
use orange_bundled::MathPackage;
use orange_compiler::{CompileError, CompileOptions, compile_control_with_effects};
use orange_guest::{GuestEngine, GuestLoadError, GuestPackage, WithGuests};
use orange_kernel::{
    BindingRequest, EffectBinding, EffectResolver, EntropyBinding, EntropyKernelResolver,
    KernelBinding, PureKernelResolver, ResolveError,
};
use orange_model::catalog::{NodeDefinition, RegistryBuilder, RegistrySnapshot};
use orange_model::validation::{Diagnostic, HostOrigin};
use orange_model::{analyze, from_json};
use orange_packages::{
    ArchiveLimits, ArchivePath, Execution, HOST_API_VERSION, MemoryJournal, MemorySink,
    PackageStore, ProviderSlot, read_archive,
};
use orange_runtime::{HostInput, InboxError, Run, RunDriver, RunEventSource, SourceOptions};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, channel, sync_channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub use orange_guest::{Grants, GuestLimits};
pub use orange_model::Value;
pub use orange_model::id::{EventDeclId, GraphId, NodeId, VariableId};
pub use orange_packages::PackageKey;
pub use orange_runtime::{
    ControlOutcome, RefusedReason, RunError, RunEvent, RunOptions, RunState, StopMode,
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

/// Bundled native kernels for pure and entropy nodes. The host itself implements no
/// effect nodes; those come from packages.
#[derive(Debug, Clone)]
struct Native(Arc<MathPackage>);
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
    fn resolve_effect(&self, _: &BindingRequest<'_>) -> Result<EffectBinding, ResolveError> {
        Err(ResolveError::Unavailable)
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
        let start = |error: &dyn std::fmt::Display| HostError::Start(error.to_string());
        let math = MathPackage::new(built_in_digest("orange-bundled")).map_err(|e| start(&e))?;
        let origin = HostOrigin::new(env!("CARGO_PKG_VERSION"), built_in_digest("orange-host"))
            .map_err(|e| start(&e))?;
        let base = math
            .register(RegistryBuilder::core(origin))
            .map_err(|e| start(&e))?
            .build()
            .map_err(|e| start(&e))?;
        let engine = GuestEngine::new(options.guest).map_err(|e| start(&e))?;
        let pool = Pool::new(options.effect_workers).map_err(|e| start(&e))?;
        let (events, observer) = sync_channel(options.event_capacity.max(1));
        let host = Self {
            native: Native(Arc::new(math)),
            engine,
            guests: Vec::new(),
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

        let mut definitions = Vec::with_capacity(manifest.nodes().len());
        for node in manifest.nodes() {
            let name = node.to_string();
            let local = name.rsplit('/').next().unwrap_or(&name);
            let path = format!("nodes/{local}.json");
            let missing = || HostError::MissingDefinition(path.clone());
            let archive_path: ArchivePath = path.parse().map_err(|_| missing())?;
            let bytes = files.file(&path).ok_or_else(missing)?;
            verified
                .verify_file(&archive_path, bytes)
                .map_err(|error| HostError::Archive(error.to_string()))?;
            let text = std::str::from_utf8(bytes)
                .map_err(|error| HostError::Definition(error.to_string()))?;
            let definition = NodeDefinition::from_json(text)
                .map_err(|error| HostError::Definition(error.to_string()))?;
            if definition.definition.id != *node {
                return Err(HostError::Definition(format!(
                    "`{path}` does not define `{name}`"
                )));
            }
            definitions.push(definition);
        }

        // Prepare all code before anything is published, so a refused component
        // leaves the registry untouched.
        let mut guests: Vec<GuestPackage> = Vec::new();
        if matches!(manifest.execution(), Some(Execution::WasmComponent { .. })) {
            for provider in manifest.providers() {
                if guests
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
        }

        let install = |error: &dyn std::fmt::Display| HostError::Install(error.to_string());
        let _ = self
            .store
            .install(verified.into_contents(definitions))
            .map_err(|e| install(&e))?;
        for provider in manifest.providers() {
            let ProviderSlot::Node(definition) = &provider.slot else {
                continue;
            };
            let Some(guest) = guests
                .iter()
                .find(|guest| guest.provider().id == provider.id)
            else {
                continue;
            };
            if self
                .store
                .current()
                .new_binding_default(definition)
                .is_none()
            {
                let _ = self
                    .store
                    .select_default(definition.clone(), guest.provider().clone())
                    .map_err(|e| install(&e))?;
            }
        }
        self.guests.extend(guests);
        Ok(key)
    }

    /// Checks a document against the current registry without running it. An empty
    /// list means [`Host::open`] would accept it as far as validation goes.
    pub fn check(&self, json: &str) -> Result<Vec<Diagnostic>, HostError> {
        let document = from_json(json).map_err(|error| HostError::Document(error.to_string()))?;
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
        let document = from_json(json).map_err(|error| HostError::Document(error.to_string()))?;
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
