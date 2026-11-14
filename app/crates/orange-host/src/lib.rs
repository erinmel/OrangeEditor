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

mod authoring;
mod documents;
mod effects;
mod flow;
mod flows;
mod graphs;
mod names;
mod packages;
mod persist;
mod preview;
mod services;
mod state;
mod streams;
mod variables;

use effects::Pool;
use flow::{Context, Keep, Query, Shared, Sink};
use graphs::{GraphNode, Graphs};
use orange_bundled::MathPackage;
use orange_compiler::{CompileError, CompileOptions, compile_control_with_effects};
use orange_guest::{GuestAccess, GuestEngine, GuestLoadError, GuestPackage, WithGuests};
use orange_kernel::{
    BindingRequest, EffectBinding, EffectResolver, EntropyBinding, EntropyKernelResolver,
    KernelBinding, PureKernelResolver, ResolveError,
};
use orange_model::catalog::{ProviderDescriptor, RegistryBuilder, RegistrySnapshot};
use orange_model::document::{ConnectionDecl, ConnectionScope, VarScope};
use orange_model::validation::HostOrigin;
use orange_model::validation::{LeafEffects, ValidationAnalysis};
use orange_model::{GraphDocument, analyze_with_effects, from_json, to_json};
use orange_packages::{
    ArchiveFiles, ArchiveLimits, ArchivePath, DiscardSink, Execution, HOST_API_VERSION,
    MemoryJournal, MemorySink, PackageState, PackageStore, ProviderSlot, read_archive,
};
use orange_runtime::{
    HostInput, InboxError, Run, RunDriver, RunEventSource, Workspace, WorkspaceLimits,
};
use services::Services;
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel, sync_channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub use documents::{
    Bundle, MAX_DOCUMENT_BYTES, Publish, Recovery, Requirement, autosave_document,
    discard_autosave, publish_function, recover_document, save_document,
};
pub use graphs::{GRAPH_NODE_FORMAT, GRAPH_PROVIDER};
pub use names::{Names, describe, lines, literal, plain, said};
pub use orange_guest::{Grants, GuestLimits, HttpClient, HttpRequest, HttpResponse};
pub use orange_kernel::{FaultText, KernelFault};
pub use orange_model::Value;
pub use orange_model::catalog::{NodeDefinition, Presentation, ProviderKey, TextRef};
pub use orange_model::document::DefinitionRef;
pub use orange_model::document::literal::Literal;
pub use orange_model::id::{ConnectionId, PortKey};
pub use orange_model::id::{EventDeclId, GraphId, NodeId, VariableId};
pub use orange_model::validation::Diagnostic;
pub use orange_packages::{
    About, FeedError, PackageKey, PackageManifest, Release, UpdateFeed, UpdateSource,
};
pub use orange_runtime::EffectLease;
pub use orange_runtime::{
    ControlOutcome, RefusedReason, RunError, RunEvent, RunOptions, RunState, StopMode,
};
pub use orange_sdk::{
    Call, ConnectionAdapter, EmitError, Emitter, Items, SERVICE_PROVIDER, ServiceNode,
    ServicePackage, ServiceSource, ServiceStream, Session, TARGET, TypeDecl,
};
pub use orange_sdk::{Renew, SECRET_PREFIX, SecretError, SecretStore, is_secret_name};
pub use preview::Previewed;
pub use state::StoredVariable;

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
    /// IDs of packages this host leaves out when it starts, in addition to the ones
    /// [`Host::set_enabled`] saved in [`package_dir`](Self::package_dir): the base
    /// nodes ([`BASE_PACKAGE`]), service packages and installed packages alike.
    pub disabled_packages: BTreeSet<String>,
    /// Where packages holding the `files` grant keep their data: each gets the
    /// directory `<plugin_data_dir>/<package ID>` and can name nothing outside it.
    /// `None` leaves that grant with nothing to reach.
    pub plugin_data_dir: Option<PathBuf>,
    /// The network client for packages holding the `http` grant. `None` leaves that
    /// grant with nothing to reach.
    pub plugin_http: Option<Arc<dyn HttpClient>>,
    /// Most bytes each package's data directory may hold. `None` is no limit.
    pub plugin_data_quota: Option<u64>,
    /// The hosts packages holding the `http` grant may reach; a name starting with
    /// `.` allows every host under that domain. `None` allows any host.
    pub plugin_http_hosts: Option<Vec<String>>,
    /// Where connection settings' `credential://<name>` references are read from.
    /// `None` refuses a connection that names a credential.
    pub secrets: Option<Arc<dyn SecretStore>>,
    /// Where variables declared `persistent` are stored. With a directory, a flow's
    /// persistent run variables are saved as they change and a flow opened later on
    /// the same graph starts with them; persistent shared variables likewise. `None`
    /// keeps values for the life of each flow only.
    pub state_dir: Option<PathBuf>,
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
            disabled_packages: BTreeSet::new(),
            plugin_data_dir: None,
            plugin_http: None,
            plugin_data_quota: None,
            plugin_http_hosts: None,
            secrets: None,
            state_dir: None,
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
    #[error("the variable could not be set: {0}")]
    Variable(String),
    #[error("stored variables: {0}")]
    State(String),
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
    #[error("the package could not be removed: {0}")]
    Uninstall(String),
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

/// The settings an adapter receives: every string that is exactly
/// `credential://<name>`, at any depth, replaced by that secret. The result is handed
/// to the adapter and never stored, logged or compared.
fn with_secrets(
    secrets: Option<&Arc<dyn SecretStore>>,
    settings: &serde_json::Map<String, serde_json::Value>,
) -> Result<(serde_json::Map<String, serde_json::Value>, Renew), String> {
    use serde_json::Value as Json;
    // Each setting that named a credential, as a JSON pointer, with the name.
    fn resolve(
        value: &mut Json,
        at: &mut String,
        store: Option<&Arc<dyn SecretStore>>,
        named: &mut Vec<(String, String)>,
    ) -> Result<(), String> {
        let mut inside = |part: &str, value: &mut Json, at: &mut String| {
            let length = at.len();
            at.push('/');
            at.push_str(&part.replace('~', "~0").replace('/', "~1"));
            let done = resolve(value, at, store, named);
            at.truncate(length);
            done
        };
        match value {
            Json::String(text) => {
                if let Some(name) = text.strip_prefix(SECRET_PREFIX) {
                    let store = store.ok_or_else(|| {
                        format!(
                            "it names credential `{name}` and the application has no credential store"
                        )
                    })?;
                    let secret = store
                        .get(name)
                        .map_err(|error| format!("credential `{name}`: {error}"))?;
                    named.push((at.clone(), name.to_owned()));
                    *text = secret;
                }
                Ok(())
            }
            Json::Array(items) => items
                .iter_mut()
                .enumerate()
                .try_for_each(|(index, item)| inside(&index.to_string(), item, at)),
            Json::Object(fields) => fields
                .iter_mut()
                .try_for_each(|(name, field)| inside(name, field, at)),
            _ => Ok(()),
        }
    }
    let mut resolved = Json::Object(settings.clone());
    let mut named = Vec::new();
    resolve(&mut resolved, &mut String::new(), secrets, &mut named)?;
    let Json::Object(resolved) = resolved else {
        unreachable!("resolving keeps an object an object")
    };
    let renew = match secrets {
        Some(store) if !named.is_empty() => {
            let store = Arc::clone(store);
            Renew::new(move |setting, secret| {
                if let Some((_, name)) = named.iter().find(|(at, _)| at == setting) {
                    // A store that refuses leaves the old secret: the connection keeps
                    // working and the next start asks for a new sign-in.
                    let _ = store.replace(name, secret);
                }
            })
        }
        _ => Renew::none(),
    };
    Ok((resolved, renew))
}

/// The grants a manifest's `permissions` section asks for: a list of `"log"`,
/// `"files"` and `"http"`. No section asks for nothing.
fn asked(manifest: &PackageManifest) -> Result<Grants, HostError> {
    let refused = || {
        HostError::Install("`permissions` must be a list of `log`, `files` and `http`".to_owned())
    };
    let mut grants = Grants::default();
    let Some(section) = manifest.permissions() else {
        return Ok(grants);
    };
    for permission in section.as_array().ok_or_else(refused)? {
        match permission.as_str() {
            Some("log") => grants.log = true,
            Some("files") => grants.files = true,
            Some("http") => grants.http = true,
            _ => return Err(refused()),
        }
    }
    Ok(grants)
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
impl NodeInfo<'_> {
    /// The node's name in the default language: its definition file's label, or a name
    /// made from its ID (`core/set-variable` is "Set variable") when it has none.
    pub fn label(&self) -> Cow<'_, str> {
        match self.presentation.and_then(|shown| shown.label.as_ref()) {
            Some(label) => Cow::Borrowed(label.fallback.as_str()),
            None => {
                let id = self.definition.definition.id.as_str();
                Cow::Owned(words(id.rsplit('/').next().unwrap_or(id)))
            }
        }
    }
    /// The palette category in the default language: its definition file's category,
    /// or one made from the ID's namespace (`org.orange.math/add` is in "Math").
    pub fn category(&self) -> Cow<'_, str> {
        match self.presentation.and_then(|shown| shown.category.as_ref()) {
            Some(category) => Cow::Borrowed(category.fallback.as_str()),
            None => {
                let id = self.definition.definition.id.as_str();
                let namespace = id.split('/').next().unwrap_or(id);
                Cow::Owned(words(namespace.rsplit('.').next().unwrap_or(namespace)))
            }
        }
    }
}
/// ID under which [`Host::packages`] lists the bundled base nodes (arithmetic,
/// comparison, text, collection and JSON operations) and [`Host::set_enabled`]
/// switches them.
pub const BASE_PACKAGE: &str = "org.orange.base";

/// Where a package of a host comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PackageKind {
    /// The bundled base nodes.
    Base,
    /// A native service package: compiled into the application or loaded from a
    /// plugin library when it started.
    Native,
    /// A package installed from an archive.
    Installed,
}

/// One package of a host, as [`Host::packages`] lists it.
#[derive(Debug, Clone, Copy)]
pub struct PackageInfo<'a> {
    pub id: &'a str,
    pub version: &'a str,
    pub kind: PackageKind,
    /// Whether the package is in use now: its nodes are registered.
    pub active: bool,
    /// Whether a host started with the same options and directory uses the package.
    /// It differs from `active` after [`Host::set_enabled`] until the next start.
    pub enabled: bool,
    /// The manifest of an active installed package: its `about` text, its `update`
    /// source, its nodes and its digest. `None` for other kinds, and for an installed
    /// package that is not active, whose archive this host did not read.
    pub manifest: Option<&'a PackageManifest>,
    name: Option<&'a str>,
    description: Option<&'a str>,
    update: Option<&'a UpdateSource>,
}
impl<'a> PackageInfo<'a> {
    /// Where newer versions of the package are announced, when it says so.
    pub fn update(&self) -> Option<&'a UpdateSource> {
        self.update
            .or_else(|| self.manifest.and_then(PackageManifest::update))
    }
    /// A display name, when the package gives one.
    pub fn name(&self) -> Option<&'a str> {
        self.name
            .or_else(|| self.manifest.and_then(|manifest| manifest.about().name()))
    }
    /// One paragraph on what the package adds, when it says so.
    pub fn description(&self) -> Option<&'a str> {
        self.description.or_else(|| {
            self.manifest
                .and_then(|manifest| manifest.about().description())
        })
    }
}

/// A base or native package of this host.
#[derive(Debug)]
struct BuiltIn {
    id: String,
    version: String,
    kind: PackageKind,
    active: bool,
    name: Option<String>,
    description: Option<String>,
    update: Option<UpdateSource>,
}

/// `set-variable` as "Set variable".
fn words(slug: &str) -> String {
    let mut text = slug.replace(['-', '_'], " ");
    if let Some(first) = text.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    text
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
    /// The base nodes and each service package the application was started with.
    built_in: Vec<BuiltIn>,
    /// IDs of the packages a host started like this one leaves out.
    disabled: BTreeSet<String>,
    /// Saved installed packages left out at start because they were disabled.
    inactive: Vec<(String, String)>,
    /// The shared variables, in declaration order.
    shared_variables: Vec<VariableId>,
    space: Arc<flow::Space>,
    pool: Pool,
    sink: Sink,
    clock: Instant,
    flows: BTreeMap<FlowId, Flow>,
    next_flow: u64,
    /// Made at the first preview.
    preview: Option<preview::Engine>,
}

/// Registers one declared type of package `id` in a registry being built. A type the
/// registry already has with the same schema is left as it is.
pub(crate) fn register_type(
    types: &mut orange_model::types::TypeRegistryBuilder,
    id: &str,
    declaration: &TypeDecl,
) -> Result<(), String> {
    use orange_model::type_term::TypeTerm;
    use orange_model::value::literal_codec::{
        EnumSchema, FieldSchema, RecordSchema, VariantSchema,
    };
    let (TypeDecl::Record { name, .. } | TypeDecl::Enum { name, .. }) = declaration;
    let refused = |what: &str| format!("package {id}: type `{name}`: {what}");
    let term = |ty: &str| {
        ty.parse()
            .ok()
            .and_then(|expr| TypeTerm::parse(&expr).ok())
            .ok_or_else(|| refused(&format!("`{ty}` is not a type")))
    };
    let ty = format!("{id}/{name}@1")
        .parse()
        .map_err(|_| refused("the name is not valid"))?;
    let registered = match declaration {
        TypeDecl::Record { fields, .. } => {
            let fields = fields
                .iter()
                .map(|(field_id, ty, optional)| {
                    Ok(FieldSchema {
                        field_id: field_id.clone(),
                        ty: term(ty)?,
                        optional: *optional,
                    })
                })
                .collect::<Result<_, String>>()?;
            types.ensure_record(id, &ty, RecordSchema { fields })
        }
        TypeDecl::Enum { variants, .. } => {
            let variants = variants
                .iter()
                .map(|(variant_id, payload)| {
                    Ok(VariantSchema {
                        variant_id: variant_id.clone(),
                        payload: payload.as_deref().map(term).transpose()?,
                    })
                })
                .collect::<Result<_, String>>()?;
            types.ensure_enum(id, &ty, EnumSchema { variants })
        }
    };
    registered.map_err(|error| refused(&error.to_string()))
}

/// The type registry with the types the active service packages declare; `None` when
/// they declare none and the core registry serves.
fn package_types(
    packages: &[ServicePackage],
    disabled: &BTreeSet<String>,
) -> Result<Option<orange_model::types::TypeRegistry>, String> {
    let mut types = orange_model::types::TypeRegistryBuilder::core();
    let mut any = false;
    for package in packages {
        if disabled.contains(package.id()) {
            continue;
        }
        let mut declared = Vec::new();
        for declaration in package.types() {
            let (TypeDecl::Record { name, .. } | TypeDecl::Enum { name, .. }) = declaration;
            if declared.contains(&name) {
                return Err(format!(
                    "package {}: type `{name}` is declared twice",
                    package.id()
                ));
            }
            declared.push(name);
            register_type(&mut types, package.id(), declaration)?;
            any = true;
        }
    }
    if !any {
        return Ok(None);
    }
    types
        .build()
        .map(Some)
        .map_err(|error| format!("package types: {error}"))
}

/// A stable identity for code compiled into this host, in the form package digests use.
fn built_in_digest(what: &str) -> String {
    let hash = Sha256::digest(format!("{what}@{}", env!("CARGO_PKG_VERSION")).as_bytes());
    format!("sha256:{}", hex(&hash))
}

/// Lowercase hexadecimal.
pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
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
    /// adapters become available to documents. A package whose ID is disabled
    /// ([`HostOptions::disabled_packages`], [`Host::set_enabled`]) is listed by
    /// [`Host::packages`] and otherwise left out.
    pub fn with_services(
        options: HostOptions,
        packages: Vec<ServicePackage>,
    ) -> Result<(Self, Receiver<HostEvent>), HostError> {
        let start = |error: &dyn std::fmt::Display| HostError::Start(error.to_string());
        let math = MathPackage::new(built_in_digest("orange-bundled")).map_err(|e| start(&e))?;
        let origin = HostOrigin::new(env!("CARGO_PKG_VERSION"), built_in_digest("orange-host"))
            .map_err(|e| start(&e))?;
        let saved = match &options.package_dir {
            Some(directory) => persist::load(directory).map_err(HostError::Storage)?,
            None => Vec::new(),
        };
        let mut disabled = options.disabled_packages.clone();
        if let Some(directory) = &options.package_dir {
            disabled.extend(persist::disabled(directory).map_err(HostError::Storage)?);
        }
        let mut builder = RegistryBuilder::core(origin);
        if let Some(types) = package_types(&packages, &disabled).map_err(HostError::Start)? {
            builder = builder.types(Arc::new(types));
        }
        let base_nodes = options.base_nodes && !disabled.contains(BASE_PACKAGE);
        if base_nodes {
            builder = math.register(builder).map_err(|e| start(&e))?;
        }
        let mut services = Services::default();
        let mut built_in = vec![BuiltIn {
            id: BASE_PACKAGE.to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            kind: PackageKind::Base,
            active: base_nodes,
            name: Some("Base nodes".to_owned()),
            description: Some(
                "Arithmetic, comparison, text, collection and JSON operations.".to_owned(),
            ),
            update: None,
        }];
        built_in.extend(packages.iter().map(|package| {
            BuiltIn {
                id: package.id().to_owned(),
                version: package.version().to_owned(),
                kind: PackageKind::Native,
                active: !disabled.contains(package.id()),
                name: package.name().map(str::to_owned),
                description: package.description().map(str::to_owned),
                // A source that breaks the manifest rules is not offered.
                update: package
                    .update_source()
                    .and_then(|(feed, key)| UpdateSource::new(feed, None, key).ok()),
            }
        }));
        let mut adapters = BTreeMap::new();
        let mut presentations = BTreeMap::new();
        for package in &packages {
            if disabled.contains(package.id()) {
                continue;
            }
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
            built_in,
            disabled,
            inactive: Vec::new(),
            shared_variables: Vec::new(),
            space: Arc::default(),
            pool,
            sink: Sink {
                events,
                dropped: Arc::new(AtomicU64::new(0)),
            },
            clock: Instant::now(),
            flows: BTreeMap::new(),
            next_flow: 1,
            preview: None,
            options,
        };
        let mut host = host;
        for package in saved {
            if let Some((id, version)) = &package.key
                && host.disabled.contains(id)
            {
                host.inactive.push((id.clone(), version.clone()));
                continue;
            }
            let installed = package
                .archive
                .map_err(HostError::Storage)
                .and_then(|archive| host.install_archive(&archive, package.grants, None));
            if let Err(error) = installed {
                host.not_restored.push((package.file, error));
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
                    _ => {
                        let (settings, renew) =
                            with_secrets(self.options.secrets.as_ref(), &decl.settings)
                                .map_err(refused)?;
                        adapter.open_renewing(&settings, renew).map_err(refused)?
                    }
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
                            let (settings, renew) =
                                with_secrets(self.options.secrets.as_ref(), &shared.settings)
                                    .map_err(refused)?;
                            let session =
                                adapter.open_renewing(&settings, renew).map_err(refused)?;
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

    /// The packages a document is locked to and whether this host has each one: what an
    /// import shows before anything is opened. A document with a requirement that is
    /// not available still loads and saves; it does not open as a flow until the
    /// package is installed.
    pub fn requirements(&self, json: &str) -> Result<Vec<Requirement>, HostError> {
        let document = from_json(json).map_err(|error| HostError::Document(error.to_string()))?;
        let registry = self.store.current();
        Ok(document
            .package_lock()
            .iter()
            .map(|entry| Requirement {
                package: entry.id.clone(),
                version: entry.version.clone(),
                content_digest: entry.content_digest.clone(),
                available: registry.providers().any(|provider| {
                    provider.key.package == entry.id
                        && provider.key.version == entry.version
                        && provider.key.content_digest == entry.content_digest
                }),
            })
            .collect())
    }

    /// Builds a bundle of a document and the stored archives of the packages it is
    /// locked to, to hand to someone else. The second value lists the requirements
    /// whose archive this host does not hold (a native package, or a package installed
    /// without [`HostOptions::package_dir`]); the receiver needs those separately.
    pub fn export(&self, json: &str) -> Result<(Bundle, Vec<Requirement>), HostError> {
        let requirements = self.requirements(json)?;
        let saved = match &self.options.package_dir {
            Some(directory) => persist::load(directory).map_err(HostError::Storage)?,
            None => Vec::new(),
        };
        let (mut packages, mut missing) = (Vec::new(), Vec::new());
        for requirement in requirements {
            let archive = saved.iter().find_map(|package| {
                let (id, version) = package.key.as_ref()?;
                (*id == requirement.package && *version == requirement.version)
                    .then(|| package.archive.as_ref().ok().cloned())
                    .flatten()
            });
            match archive {
                Some(archive) => packages.push(archive),
                None => missing.push(requirement),
            }
        }
        let bundle = Bundle {
            document: json.to_owned(),
            packages,
        };
        Ok((bundle, missing))
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
